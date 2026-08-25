//! 复制成功小气泡窗口：复制出新内容时在屏幕底部居中弹一个极小的置顶提示窗。
//! 用独立 webview 窗实现，不依赖剪贴板主窗口是否可见（主窗口平时缩到托盘，气泡若渲染
//! 在它里面就看不见）。
//!
//! 与右键菜单窗（`context_window`）同一套参数：`focusable: false` 不抢前台焦点、
//! `always_on_top` 保证盖在任意应用上层、`transparent` 只显示圆角卡片。
//!
//! 生命周期由前端驱动：本端 show 后广播一次 [`COPIED_PLAY_EVENT`]，前端据此播放
//! 「出现 → 画圆 → 画勾 → 停留 → 淡出」动画，动画结束后 invoke `hide_copied_toast`
//! 让本端隐藏窗口；本端另保留一个兜底超时，防止前端异常时窗口残留。
//!
//! 销毁策略：永不销毁（与剪贴板主窗口一致）。隐藏后 13.14s 进入休眠态，仅记录状态，
//! WebView 实例保留，下次 show 时秒级复用。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

use crate::core::{AppError, Result};
use crate::window::lifecycle;

pub const COPIED_WINDOW_LABEL: &str = "copied";

/// 窗口尺寸紧贴内容实际大小：图标30 + gap10 + 文字~56 + padding(10+18) = 124px，
/// 留2px余量给抗锯齿。高度: padding(8+8) + 图标30 = 46px，留2px余量。
/// 透明窗口中内容边缘=窗口边缘，消除半透明伪影"边框"。
const WINDOW_WIDTH: f64 = 128.0;
const WINDOW_HEIGHT: f64 = 48.0;
/// 距屏幕底部的留白（logical px）。底部居中显示。
const SCREEN_BOTTOM_MARGIN: f64 = 56.0;
/// show 时广播给前端、通知其重播动画的事件。
const COPIED_PLAY_EVENT: &str = "copied://play";
/// 前端动画失败时的兜底隐藏时长。正常路径前端会在动画结束后主动 invoke 隐藏，
/// 此值略大于「出现(0.9s)+停留(0.52s)+淡出(0.5s)」的总和，避免打断动画。
const HIDE_FALLBACK_AFTER: Duration = Duration::from_millis(3000);
/// 隐藏后进入休眠的时长：与剪贴板主窗口同机制，隐藏 13.14s 后标记为休眠态。
/// 窗口永不销毁，WebView 实例保留供下次 show 秒级复用。
const DORMANT_AFTER: Duration = Duration::from_millis(13_140);
/// 每 show 一次自增的纪元。hide 后启动的休眠计时捕获当时的纪元，
/// 若期间又 show（纪元变化）则计时作废，避免误标休眠。
static COPIED_EPOCH: AtomicU64 = AtomicU64::new(0);
/// 是否处于休眠态（仅用于调试，不影响功能）。
static COPIED_DORMANT: AtomicBool = AtomicBool::new(false);

/// 按需建窗。窗口保持 `visible: false`，由 [`show`] 统一 show + 定位；重复调用复用已存在窗口。
fn ensure_window(app: &AppHandle) -> Result<()> {
    if app.get_webview_window(COPIED_WINDOW_LABEL).is_some() {
        return Ok(());
    }

    WebviewWindowBuilder::new(
        app,
        COPIED_WINDOW_LABEL,
        WebviewUrl::App("index.html/#/copied".into()),
    )
    .inner_size(WINDOW_WIDTH, WINDOW_HEIGHT)
    .decorations(false)
    .transparent(true)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .always_on_top(true)
    .focusable(false)
    .visible(false)
    .skip_taskbar(true)
    .drag_and_drop(false)
    .build()
    .map_err(|err| AppError::Other(anyhow::anyhow!("build copied window: {err}")))?;

    Ok(())
}

/// 在剪贴板窗口所在显示器（回退主显示器）右下角弹出复制成功提示，1.5s 后自动隐藏。
/// 被 [`crate::clipboard::watcher`] 在「非去重新入库」时调用。失败仅记日志，不阻断入库。
pub fn show(app: &AppHandle) {
    // 每 show 一次自增纪元：使在途的休眠计时失效，避免误标为休眠态。
    COPIED_EPOCH.fetch_add(1, Ordering::Relaxed);
    COPIED_DORMANT.store(false, Ordering::Relaxed);

    if let Err(err) = show_inner(app) {
        log::warn!("show copied toast failed: {err}");
    }
}

fn show_inner(app: &AppHandle) -> Result<()> {
    ensure_window(app)?;

    let window = app
        .get_webview_window(COPIED_WINDOW_LABEL)
        .ok_or_else(|| AppError::Other(anyhow::anyhow!("copied window missing")))?;

    // 底部居中定位：优先放剪贴板窗口所在显示器，其次主显示器。
    // 不再是右下角——与主流截图/通知弹窗位置一致，视觉中心化。
    let monitor = app
        .get_webview_window(crate::window::CLIPBOARD_WINDOW_LABEL)
        .and_then(|w| w.current_monitor().ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());

    if let Some(mon) = monitor {
        let scale = window.scale_factor().unwrap_or(1.0);
        let mon_pos = *mon.position();
        let mon_size = *mon.size();
        let win_size = window
            .inner_size()
            .map_err(|e| AppError::Other(anyhow::anyhow!(e)))?;
        let bottom_margin = (SCREEN_BOTTOM_MARGIN * scale) as i32;

        // 水平居中：屏幕中心 - 窗口宽度的一半。
        let x = mon_pos.x + (mon_size.width as i32 - win_size.width as i32) / 2;
        // 贴底部：屏幕底 - 窗口高度 - 边距。
        let y = mon_pos.y + mon_size.height as i32 - win_size.height as i32 - bottom_margin;
        window
            .set_position(PhysicalPosition::new(x, y))
            .map_err(|err| AppError::Other(anyhow::anyhow!("copied toast position: {err}")))?;
    }

    window
        .show()
        .map_err(|err| AppError::Other(anyhow::anyhow!("copied toast show: {err}")))?;
    lifecycle::on_shown(app, COPIED_WINDOW_LABEL);

    // 广播一次「重播动画」。前端在页面加载时会自动播放一遍；首次建窗可能因页面尚未
    // ready 丢失此事件，由前端 mount 自播兜底，后续复用窗口均能收到并重播。
    if let Err(err) = app.emit(COPIED_PLAY_EVENT, ()) {
        log::warn!("emit copied play failed: {err}");
    }

    // 兜底隐藏：正常路径由前端动画结束后 invoke hide_copied_toast 完成隐藏，
    // 这里仅防御前端异常导致窗口残留。
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(HIDE_FALLBACK_AFTER).await;
        if let Some(w) = app.get_webview_window(COPIED_WINDOW_LABEL) {
            if w.is_visible().unwrap_or(false) {
                let _ = w.hide();
                lifecycle::on_hidden(&app, COPIED_WINDOW_LABEL, "fallback");
                schedule_dormant(&app);
            }
        }
    });

    Ok(())
}

/// 前端动画（淡出）结束后调用，隐藏气泡窗。
#[tauri::command]
pub fn hide_copied_toast(app: AppHandle) {
    let hidden = app
        .get_webview_window(COPIED_WINDOW_LABEL)
        .map(|window| {
            let visible = window.is_visible().unwrap_or(false);
            let _ = window.hide();
            visible
        })
        .unwrap_or(false);
    lifecycle::on_hidden(&app, COPIED_WINDOW_LABEL, "frontend");
    if hidden {
        schedule_dormant(&app);
    }
}

/// 启动隐藏后的休眠计时：`DORMANT_AFTER` 后，若窗口仍隐藏且期间未被再次 show
/// （纪元未变），则标记为休眠态。窗口永不销毁，WebView 实例保留供下次秒级复用。
fn schedule_dormant(app: &AppHandle) {
    let epoch = COPIED_EPOCH.load(Ordering::Relaxed);
    let app = app.clone();

    thread::spawn(move || {
        thread::sleep(DORMANT_AFTER);

        // 查询窗口状态需回主线程操作窗口句柄。
        let main_app = app.clone();
        if let Err(err) = app.run_on_main_thread(move || {
            // 休眠期内又被 show（纪元变化）则放弃本次标记。
            if COPIED_EPOCH.load(Ordering::Relaxed) != epoch {
                return;
            }
            let Some(window) = main_app.get_webview_window(COPIED_WINDOW_LABEL) else {
                return;
            };
            if window.is_visible().unwrap_or(true) {
                return;
            }
            COPIED_DORMANT.store(true, Ordering::Relaxed);
            log::debug!("copied toast entered dormant state");
        }) {
            log::warn!("copied toast dormant main-thread dispatch failed: {err}");
        }
    });
}
