//! 复制成功小气泡窗口：复制出新内容时在屏幕右下角弹一个极小的置顶提示窗，
//! 1.5s 后自动隐藏。用独立 webview 窗实现，不依赖剪贴板主窗口是否可见（主窗口平时缩到
//! 托盘，气泡若渲染在它里面就看不见）。
//!
//! 与右键菜单窗（`context_window`）同一套参数：`focusable: false` 不抢前台焦点、
//! `always_on_top` 保证盖在任意应用上层、`transparent` 只显示圆角卡片。

use std::time::Duration;

use tauri::{AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

use crate::core::{AppError, Result};
use crate::window::lifecycle;

pub const COPIED_WINDOW_LABEL: &str = "copied";

/// 卡片尺寸（logical px），前端 `/copied` 页 CSS 与其保持一致。
const WINDOW_WIDTH: f64 = 168.0;
const WINDOW_HEIGHT: f64 = 44.0;
/// 距屏幕底部的留白（logical px）。底部居中显示。
const SCREEN_BOTTOM_MARGIN: f64 = 56.0;
/// 展示时长后自动隐藏。
const HIDE_AFTER: Duration = Duration::from_millis(1500);

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

    // 展示到时后自动隐藏（后端定时，前端不参与生命周期）。
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(HIDE_AFTER).await;
        if let Some(w) = app.get_webview_window(COPIED_WINDOW_LABEL) {
            let _ = w.hide();
            lifecycle::on_hidden(&app, COPIED_WINDOW_LABEL, "auto");
        }
    });

    Ok(())
}
