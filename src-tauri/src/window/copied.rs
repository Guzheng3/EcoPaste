//! 复制成功小气泡窗口：复制出新内容时在屏幕底部居中弹一个极小的置顶提示窗。
//! 用独立 webview 窗实现，不依赖剪贴板主窗口是否可见（主窗口平时缩到托盘，气泡若渲染
//! 在它里面就看不见）。
//!
//! 与右键菜单窗（`context_window`）同一套参数：`focusable: false` 不抢前台焦点、
//! `always_on_top` 保证盖在任意应用上层、`transparent` 只显示圆角卡片；另加
//! `shadow: false`，避免 Windows DWM 的无边框窗口矩形描边在卡片淡出后残留。
//!
//! 生命周期由前端驱动：本端 show 后广播一次 [`COPIED_PLAY_EVENT`]，前端据此播放
//! 「出现 → 画圆 → 画勾 → 停留 → 淡出」动画，动画结束后 invoke `hide_copied_toast`
//! 让本端隐藏窗口；本端另保留一个兜底超时，防止前端异常时窗口残留。
//! 隐藏统一走 [`hide_toast`]：先停 WebView 合成再藏原生窗口，避免消失瞬间的
//! 矩形闪帧。
//!
//! 销毁策略：永不销毁（与剪贴板主窗口一致）。隐藏后 13.14s 进入休眠态，仅记录状态，
//! WebView 实例保留，下次 show 时秒级复用。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use serde_json::json;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder,
};

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

    // 与其它 WebView 窗保持同一份 `additional_browser_args`：WebView2 共享 browser
    // process，参数只在首个建窗时生效，任何建窗点带上不同参数都会导致后续 webview
    // 创建失败（0x8007139F），气泡因此无法弹出。
    let builder = crate::window::apply_webview_args(WebviewWindowBuilder::new(
        app,
        COPIED_WINDOW_LABEL,
        WebviewUrl::App("index.html/#/copied".into()),
    ))
    .inner_size(WINDOW_WIDTH, WINDOW_HEIGHT)
    .decorations(false)
    .transparent(true)
    // Windows 上 DWM 默认给无边框窗口描一圈矩形边框：卡片淡出后、窗口 hide 前，
    // 内容已透明而边框仍残留，观感上是一个矩形框最后消失。关掉 shadow 后
    // 窗口本身不再有任何原生描边，视觉只剩前端绘制的圆角卡片。
    .shadow(false)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .always_on_top(true)
    .focusable(false)
    .visible(false)
    .skip_taskbar(true);

    // `drag_and_drop` 是 Tauri 的 Windows 专属 API（源码标 `#[cfg(windows)]`），macOS 没有
    // 对应方法。按平台门控：在 Windows 上禁用文件拖放到气泡窗，同时避免 macOS 编译报错。
    #[cfg(windows)]
    let builder = builder.drag_and_drop(false);

    builder
        .build()
        .map_err(|err| AppError::Other(anyhow::anyhow!("build copied window: {err}")))?;

    #[cfg(windows)]
    disable_native_window_frame(&app.get_webview_window(COPIED_WINDOW_LABEL).unwrap());

    Ok(())
}

/// 彻底去掉 Windows DWM 对无边框窗口的默认修饰（Windows 11 22H2+ 生效）：
/// - `DWMWA_BORDER_COLOR = DWMWA_COLOR_NONE`：移除系统沿窗口矩形边缘画的 1px 描边，
///   用户看到的「透明窗口仍有一圈边框」就是它（`shadow(false)` 管不到这条线）；
/// - `DWMWA_WINDOW_CORNER_PREFERENCE = DWMWCP_DONOTROUND`：禁用系统默认圆角，
///   避免 DWM 对透明四角做圆角裁剪并沿裁剪边描线，与前端 10px 圆角卡片叠加出双圆角。
///
/// 属性与 HWND 绑定，窗口常驻复用，建窗时设置一次即可。
/// Windows 10 及更早系统不支持这两个属性，调用失败静默忽略（本就没有该描边）。
#[cfg(windows)]
fn disable_native_window_frame(window: &tauri::WebviewWindow) {
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWM_WINDOW_CORNER_PREFERENCE,
    };

    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    let hwnd = windows::Win32::Foundation::HWND(hwnd.0 as isize);

    unsafe {
        let border_none = COLORREF(DWMWA_COLOR_NONE);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &border_none as *const COLORREF as *const std::ffi::c_void,
            std::mem::size_of::<COLORREF>() as u32,
        );

        let corner = DWM_WINDOW_CORNER_PREFERENCE(DWMWCP_DONOTROUND.0);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const DWM_WINDOW_CORNER_PREFERENCE as *const std::ffi::c_void,
            std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
    }
}

/// 在剪贴板窗口所在显示器（回退主显示器）底部居中弹出复制反馈气泡，动画结束后自动隐藏。
/// - `duplicate = false`：新内容入库，绿色对勾「复制成功」；
/// - `duplicate = true`：重复复制头部条目，粉色右箭头「已复制」。
///
/// 被 [`crate::clipboard::watcher`] 调用。失败仅记日志，不阻断入库。
pub fn show(app: &AppHandle, duplicate: bool) {
    // 每 show 一次自增纪元：使在途的休眠计时失效，避免误标为休眠态。
    COPIED_EPOCH.fetch_add(1, Ordering::Relaxed);
    COPIED_DORMANT.store(false, Ordering::Relaxed);

    if let Err(err) = show_inner(app, duplicate) {
        log::warn!("show copied toast failed: {err}");
    }
}

fn show_inner(app: &AppHandle, duplicate: bool) -> Result<()> {
    ensure_window(app)?;

    let window = app
        .get_webview_window(COPIED_WINDOW_LABEL)
        .ok_or_else(|| AppError::Other(anyhow::anyhow!("copied window missing")))?;

    // 每次都按逻辑像素重设尺寸：窗口常驻不销毁，期间显示器分辨率/缩放变化时
    // Windows 会保持物理尺寸不变，逻辑宽度随之变小，内容被挤成两行。
    // 重新施加逻辑尺寸后按当前缩放换算，保证内容始终完整单行渲染。
    window
        .set_size(LogicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT))
        .map_err(|err| AppError::Other(anyhow::anyhow!("copied toast resize: {err}")))?;

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

    // 休眠期内存目标级别被压到 Low（见 [`schedule_dormant`]），显示前恢复 Normal，
    // 保证气泡动画首帧全速渲染。与下方 show 同走主线程消息队列，先入队先执行。
    if let Err(err) = super::webview_memory::set_memory_usage_target(&window, false) {
        log::warn!("restore copied toast memory target failed: {err}");
    }

    // 先恢复 WebView 合成再显示原生窗口：hide 时为避免 WebView2 在窗口消失前
    // 闪出最终帧会先停掉 webview（见 [`hide_toast`]），此处必须成对恢复，
    // 否则窗口出现时内容空白；先恢复也让窗口出现的瞬间内容已就绪。
    // `WebviewWindow` 唯一的 `AsRef` 实现即 `AsRef<Webview>`，as_ref 直接取到 webview。
    let _ = window.as_ref().show();

    window
        .show()
        .map_err(|err| AppError::Other(anyhow::anyhow!("copied toast show: {err}")))?;
    lifecycle::on_shown(app, COPIED_WINDOW_LABEL);

    // 广播一次「重播动画」，payload 携带变体：duplicate=true 走粉色「已复制」。
    // 前端在页面加载时会自动播放一遍；首次建窗可能因页面尚未
    // ready 丢失此事件，由前端 mount 自播兜底，后续复用窗口均能收到并重播。
    if let Err(err) = app.emit(COPIED_PLAY_EVENT, json!({ "duplicate": duplicate })) {
        log::warn!("emit copied play failed: {err}");
    }

    // 兜底隐藏：正常路径由前端动画结束后 invoke hide_copied_toast 完成隐藏，
    // 这里仅防御前端异常导致窗口残留。捕获当前 epoch，到点时若 epoch 已变
    // （说明期间又 show 了一次），则放弃本次隐藏，避免误杀新弹窗。
    let fallback_epoch = COPIED_EPOCH.load(Ordering::Relaxed);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(HIDE_FALLBACK_AFTER).await;
        if COPIED_EPOCH.load(Ordering::Relaxed) != fallback_epoch {
            return;
        }
        if hide_toast(&app).unwrap_or(false) {
            lifecycle::on_hidden(&app, COPIED_WINDOW_LABEL, "fallback");
            schedule_dormant(&app);
        }
    });

    Ok(())
}

/// 隐藏气泡窗：先停掉 WebView 合成，再隐藏原生窗口。
///
/// Windows 上直接 hide 窗口时，WebView2 可能在窗口消失前刷出最后一帧
/// 非透明内容（表现为矩形框闪现后才消失）。先把 webview 置为不可见，
/// 让它立即停止合成，再隐藏窗口本体，视觉上即无缝消失。
///
/// 返回隐藏前窗口是否可见（不可见说明早已隐藏，调用方可跳过状态记账）。
fn hide_toast(app: &AppHandle) -> Option<bool> {
    let window = app.get_webview_window(COPIED_WINDOW_LABEL)?;
    let was_visible = window.is_visible().unwrap_or(false);

    let _ = window.as_ref().hide();
    let _ = window.hide();

    Some(was_visible)
}

/// 前端动画（淡出）结束后调用，隐藏气泡窗。
#[tauri::command]
pub fn hide_copied_toast(app: AppHandle) {
    let hidden = hide_toast(&app).unwrap_or(false);
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
            // 长期隐藏后把 WebView2 内存目标级别压到 Low 压缩内存，下次 show 前恢复 Normal。
            if let Err(err) = super::webview_memory::set_memory_usage_target(&window, true) {
                log::warn!("set copied toast memory target low failed: {err}");
            }
            log::debug!("copied toast entered dormant state");
        }) {
            log::warn!("copied toast dormant main-thread dispatch failed: {err}");
        }
    });
}
