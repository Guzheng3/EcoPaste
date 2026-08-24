//! 重复复制「已复制」气泡窗口：当复制的内容在历史中已存在（去重命中）时，
//! 在屏幕底部居中弹一个极小的置顶提示窗，红色圆圈 + 箭头区分于绿色「复制成功」。
//! 与 [`copied`](crate::window::copied) 使用同一套透明小窗参数，但支持**多实例向上堆叠**：
//! 连续重复复制时，每次 show 分配一个环形 slot（最多 [`MAX_STACK`] 个），新的气泡叠在
//! 旧的气泡上方，旧的逐个淡出。
//!
//! 生命周期由前端驱动：show 后广播一次 `copied-dup://play`，前端播放
//! 「出现 → 画圆 → 画箭头 → 停留 → 淡出」动画，结束后 invoke `hide_copied_dup_toast`
//! 携带本窗口 label 精确隐藏对应实例；后端另保留兜底超时，防止前端异常时窗口残留。
//! 隐藏后经 keepalive 保活，超时销毁 WebView 释放内存，下次 show 时重建。

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

use crate::core::{AppError, Result};
use crate::window::lifecycle;

/// slot 数量上限：一屏最多同时叠多少个「已复制」气泡；环形复用，超出会从最旧的开始覆盖。
const MAX_STACK: usize = 5;
/// 气泡尺寸（logical px），与 `/copied-dup` 前端页 CSS 保持一致（同 copied：128×48）。
const WINDOW_WIDTH: f64 = 128.0;
const WINDOW_HEIGHT: f64 = 48.0;
/// 距屏幕底部的留白（logical px）。
const SCREEN_BOTTOM_MARGIN: f64 = 56.0;
/// 相邻堆叠气泡的垂直间距（logical px）。
const STACK_GAP: f64 = 8.0;
/// show 时广播给前端、通知其播放动画的事件名。
const COPIED_DUP_PLAY_EVENT: &str = "copied-dup://play";
/// 前端动画失败时的兜底隐藏时长。
const HIDE_FALLBACK_AFTER: Duration = Duration::from_millis(3000);
/// 隐藏后的保活（keepalive）时长：超过仍隐藏则销毁 WebView 释放内存。
const KEEPALIVE_MS: u64 = 131_400; // 131.4s

/// 下一个要使用的 slot 序号。环形分配（`% MAX_STACK`），保证新气泡总是占用一个位置。
static NEXT_SLOT: AtomicUsize = AtomicUsize::new(0);
/// 每个 slot 的保活销毁纪元：show 时递增，使其间在途的销毁计时失效，避免误毁刚复用的窗口。
static SLOT_EPOCHS: LazyLock<Vec<AtomicU64>> =
    LazyLock::new(|| (0..MAX_STACK).map(|_| AtomicU64::new(0)).collect());
/// 当前可见气泡的有序栈（从底到顶），用于动态重排：低位消失后高位自动下移。
static VISIBLE_STACK: LazyLock<Mutex<Vec<usize>>> = LazyLock::new(|| Mutex::new(Vec::new()));

fn slot_label(slot: usize) -> String {
    format!("copied-dup-{slot}")
}

/// 按需建窗（单个 slot）。窗口保持 `visible: false`，由 [`show`] 统一 show + 定位。
fn ensure_window(app: &AppHandle, slot: usize) -> Result<()> {
    let label = slot_label(slot);
    if app.get_webview_window(&label).is_some() {
        return Ok(());
    }

    WebviewWindowBuilder::new(
        app,
        label,
        WebviewUrl::App("index.html/#/copied-dup".into()),
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
    .map_err(|err| AppError::Other(anyhow::anyhow!("build copied-dup window: {err}")))?;

    Ok(())
}

/// 按当前可见栈顺序重排所有可见气泡：index 0 贴底，index 越大越靠上。
fn reposition_all(app: &AppHandle) {
    let visible = match VISIBLE_STACK.lock() {
        Ok(v) => v,
        Err(_) => return,
    };

    let monitor = app
        .get_webview_window(crate::window::CLIPBOARD_WINDOW_LABEL)
        .and_then(|w| w.current_monitor().ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());

    let Some(mon) = monitor else {
        return;
    };

    let mon_pos = *mon.position();
    let mon_size = *mon.size();

    for (i, &slot) in visible.iter().enumerate() {
        let label = slot_label(slot);
        let Some(window) = app.get_webview_window(&label) else {
            continue;
        };
        let scale = window.scale_factor().unwrap_or(1.0);
        let Ok(win_size) = window.inner_size() else {
            continue;
        };
        let bottom_margin = (SCREEN_BOTTOM_MARGIN * scale) as i32;
        let stack_offset = (i as f64 * (WINDOW_HEIGHT + STACK_GAP) * scale) as i32;

        let x = mon_pos.x + (mon_size.width as i32 - win_size.width as i32) / 2;
        let y = mon_pos.y + mon_size.height as i32
            - win_size.height as i32
            - bottom_margin
            - stack_offset;
        let _ = window.set_position(PhysicalPosition::new(x, y));
    }
}

/// 分配并显示一个新「已复制」气泡。被剪贴板监听在「去重命中」时调用；失败仅记日志。
pub fn show(app: &AppHandle) {
    let slot = NEXT_SLOT.fetch_add(1, Ordering::Relaxed) % MAX_STACK;
    SLOT_EPOCHS[slot].fetch_add(1, Ordering::Relaxed);

    if let Err(err) = show_inner(app, slot) {
        log::warn!("show copied-dup toast failed: {err}");
    }
}

fn show_inner(app: &AppHandle, slot: usize) -> Result<()> {
    ensure_window(app, slot)?;

    let label = slot_label(slot);
    let window = app
        .get_webview_window(&label)
        .ok_or_else(|| AppError::Other(anyhow::anyhow!("copied-dup window missing")))?;

    // 加入可见栈：若该 slot 已在栈中（复用旧窗口），先移除再追加到顶部。
    {
        let mut visible = VISIBLE_STACK.lock().map_err(|e| {
            AppError::Other(anyhow::anyhow!("copied-dup visible stack poisoned: {e}"))
        })?;
        visible.retain(|&s| s != slot);
        visible.push(slot);
        if visible.len() > MAX_STACK {
            // 挤出最旧的（栈底），防止溢出
            let popped = visible.remove(0);
            if let Some(w) = app.get_webview_window(&slot_label(popped)) {
                if w.is_visible().unwrap_or(false) {
                    let _ = w.hide();
                    lifecycle::on_hidden(app, &slot_label(popped), "overflow");
                    schedule_keepalive_destroy(app, &slot_label(popped));
                }
            }
        }
    }

    reposition_all(app);

    window
        .show()
        .map_err(|err| AppError::Other(anyhow::anyhow!("copied-dup toast show: {err}")))?;
    lifecycle::on_shown(app, &label);

    // 广播「播放动画」。首次建窗可能因页面未 ready 丢失此事件，由前端 mount 自播兜底。
    if let Err(err) = app.emit(COPIED_DUP_PLAY_EVENT, ()) {
        log::warn!("emit copied-dup play failed: {err}");
    }

    // 兜底隐藏：正常路径由前端动画结束后 invoke hide_copied_dup_toast 完成隐藏。
    let app = app.clone();
    let label = label.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(HIDE_FALLBACK_AFTER).await;
        if let Some(w) = app.get_webview_window(&label) {
            if w.is_visible().unwrap_or(false) {
                hide_inner(&app, &label);
            }
        }
    });

    Ok(())
}

/// 隐藏指定 slot 的气泡，从可见栈中移除并重排剩余气泡。
fn hide_inner(app: &AppHandle, label: &str) {
    // 从可见栈中移除
    if let Some(slot_str) = label.strip_prefix("copied-dup-") {
        if let Ok(slot) = slot_str.parse::<usize>() {
            if let Ok(mut visible) = VISIBLE_STACK.lock() {
                visible.retain(|&s| s != slot);
            }
        }
    }

    if let Some(window) = app.get_webview_window(label) {
        let _ = window.hide();
    }
    lifecycle::on_hidden(app, label, "frontend");
    schedule_keepalive_destroy(app, label);

    // 重排剩余可见气泡，让高位下移填空
    reposition_all(app);
}

/// 前端动画（淡出）结束后调用，按 label 隐藏对应气泡窗。
#[tauri::command]
pub fn hide_copied_dup_toast(app: AppHandle, label: String) {
    hide_inner(&app, &label);
}

/// 启动隐藏后的保活销毁：保活 `KEEPALIVE_MS` 后，若窗口仍隐藏且期间未被再次 show
/// （纪元未变），则销毁 WebView 释放内存，下次 show 时经 [`ensure_window`] 重建。
fn schedule_keepalive_destroy(app: &AppHandle, label: &str) {
    let Some(slot_str) = label.strip_prefix("copied-dup-") else {
        return;
    };
    let Ok(slot) = slot_str.parse::<usize>() else {
        return;
    };
    let epoch = SLOT_EPOCHS[slot].load(Ordering::Relaxed);
    let app = app.clone();
    let label = label.to_owned();

    thread::spawn(move || {
        thread::sleep(Duration::from_millis(KEEPALIVE_MS));

        // 销毁需回到主线程操作窗口句柄。
        let main_app = app.clone();
        let main_label = label.clone();
        if let Err(err) = app.run_on_main_thread(move || {
            // 保活期内又被 show（纪元变化）则放弃本次销毁。
            if SLOT_EPOCHS[slot].load(Ordering::Relaxed) != epoch {
                return;
            }
            let Some(window) = main_app.get_webview_window(&main_label) else {
                return;
            };
            if window.is_visible().unwrap_or(true) {
                return;
            }
            if let Err(err) = window.destroy() {
                log::warn!("keepalive destroy copied-dup window failed: {err}");
            }
        }) {
            log::warn!("keepalive destroy copied-dup window main-thread dispatch failed: {err}");
        }
    });
}
