use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::core::Result;
use crate::settings::WindowPosition;

/// 剪贴板窗口的设计默认尺寸（逻辑像素）。运行时按所在屏分辨率 + DPI 自适应缩放，见 [`fit_default_size`]。
pub(super) const DEFAULT_WINDOW_WIDTH: f64 = 587.0;
pub(super) const DEFAULT_WINDOW_HEIGHT: f64 = 1055.0;

/// 自动缩放的屏幕留白（逻辑像素），避免体积紧贴工作区边缘。
const FIT_MARGIN: f64 = 12.0;

struct MonitorInfo {
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    /// 工作区大小（已扣除任务栏 / 刘海遮挡），用于自适应缩放。
    work_size: PhysicalSize<u32>,
}

fn monitor_from_cursor(
    window: &WebviewWindow,
) -> Result<Option<(MonitorInfo, PhysicalPosition<f64>)>> {
    let cursor = window.cursor_position().map_err(|e| anyhow::anyhow!(e))?;
    let scale = window.scale_factor().map_err(|e| anyhow::anyhow!(e))?;

    let logical = cursor.to_logical::<f64>(scale);

    let monitor = window
        .monitor_from_point(logical.x, logical.y)
        .map_err(|e| anyhow::anyhow!(e))?;

    let Some(monitor) = monitor else {
        return Ok(None);
    };

    Ok(Some((
        MonitorInfo {
            position: *monitor.position(),
            size: *monitor.size(),
            work_size: monitor.work_area().size,
        },
        cursor,
    )))
}

/// 剪贴板窗口尺寸自适应：设计默认 587×1055，按光标所在屏工作区 + DPI 等比缩放适配，
/// 保证完整落在可用区域内。只等比缩小、不放大超过设计尺寸（即小屏变小、大屏保持设计值）。
pub fn fit_default_size(window: &WebviewWindow) -> Result<()> {
    let Some((monitor, _)) = monitor_from_cursor(window)? else {
        return Ok(());
    };

    let scale = window.scale_factor().map_err(|e| anyhow::anyhow!(e))?;

    // 工作区换算为逻辑像素（CSS 像素），减去两侧留白。
    let avail_w = monitor.work_size.width as f64 / scale - FIT_MARGIN * 2.0;
    let avail_h = monitor.work_size.height as f64 / scale - FIT_MARGIN * 2.0;

    let ratio = (avail_w / DEFAULT_WINDOW_WIDTH)
        .min(avail_h / DEFAULT_WINDOW_HEIGHT)
        .clamp(0.01, 1.0);

    // 逻辑尺寸再乘 DPI，得到物理像素尺寸。
    let w = (DEFAULT_WINDOW_WIDTH * ratio * scale).round().max(1.0) as u32;
    let h = (DEFAULT_WINDOW_HEIGHT * ratio * scale).round().max(1.0) as u32;

    window
        .set_size(PhysicalSize::new(w, h))
        .map_err(|e| anyhow::anyhow!(e))?;

    Ok(())
}

/// `Remember` 定位时，若自适应缩放后的窗口底部/右侧超出当前屏工作区（比旧尺寸更大时可能发生），
/// 回落到该屏中心，避免窗口跑到屏幕外。
pub fn recenter_if_out_of_bounds(window: &WebviewWindow) -> Result<()> {
    let Some((monitor, _)) = monitor_from_cursor(window)? else {
        return Ok(());
    };

    let scale = window.scale_factor().map_err(|e| anyhow::anyhow!(e))?;
    let pos = window.outer_position().map_err(|e| anyhow::anyhow!(e))?;
    let size = window.inner_size().map_err(|e| anyhow::anyhow!(e))?;

    let right = pos.x as f64 + size.width as f64 / scale;
    let bottom = pos.y as f64 + size.height as f64 / scale;
    let work_w = monitor.work_size.width as f64 / scale;
    let work_h = monitor.work_size.height as f64 / scale;

    if pos.x < 0 || pos.y < 0 || right > work_w || bottom > work_h {
        center_on_cursor_monitor(window)?;
    }

    Ok(())
}

pub fn position_window(window: &WebviewWindow, position: WindowPosition) -> Result<()> {
    let Some((monitor, cursor)) = monitor_from_cursor(window)? else {
        return Ok(());
    };

    match position {
        WindowPosition::Remember => {}
        WindowPosition::FollowCursor => apply_follow(window, &monitor, &cursor)?,
        WindowPosition::Center => apply_center(window, &monitor)?,
    }

    Ok(())
}

fn apply_follow(
    window: &WebviewWindow,
    monitor: &MonitorInfo,
    cursor: &PhysicalPosition<f64>,
) -> Result<()> {
    let win_size = window.inner_size().map_err(|e| anyhow::anyhow!(e))?;
    let mon_x = monitor.position.x as f64;
    let mon_y = monitor.position.y as f64;
    let mon_w = monitor.size.width as f64;
    let mon_h = monitor.size.height as f64;

    let x = cursor.x.min(mon_x + mon_w - win_size.width as f64);
    let y = cursor.y.min(mon_y + mon_h - win_size.height as f64);

    window
        .set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}

/// 将窗口居中到当前光标所在显示器。
/// 用于存档位置已失效（显示器被拔出）时的 fallback。
pub(super) fn center_on_cursor_monitor(window: &WebviewWindow) -> Result<()> {
    let Some((monitor, _)) = monitor_from_cursor(window)? else {
        return Ok(());
    };
    apply_center(window, &monitor)
}

fn apply_center(window: &WebviewWindow, monitor: &MonitorInfo) -> Result<()> {
    let win_size = window.inner_size().map_err(|e| anyhow::anyhow!(e))?;
    let mon_x = monitor.position.x as f64;
    let mon_y = monitor.position.y as f64;
    let mon_w = monitor.size.width as f64;
    let mon_h = monitor.size.height as f64;

    let x = mon_x + (mon_w - win_size.width as f64) / 2.0;
    let y = mon_y + (mon_h - win_size.height as f64) / 2.0;

    window
        .set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))
        .map_err(|e| anyhow::anyhow!(e))?;
    Ok(())
}
