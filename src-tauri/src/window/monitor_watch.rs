//! 显示器配置监听：分辨率 / DPI 缩放 / 显示器增删变化后自动重启应用。
//!
//! 窗口几何是按「当时分辨率」保存的：分辨率或缩放变化后，旧几何可能超出新屏幕，
//! 表现为弹窗显示不完整。本模块轮询显示器快照，检测到变化后：
//! 1. 保留用户保存的几何存档（不清空），尺寸偏好不因分辨率变化而丢失；
//! 2. 剪贴板窗口可见时先把窗口夹回屏幕救急，等窗口隐藏后再重启；
//! 3. 窗口不可见时立即重启（`AppHandle::restart`，与托盘菜单行为一致）。
//!
//! 重启后新进程恢复存档时，`restore_window_state` 会把尺寸钳制到新屏幕内
//! （超出则压到边界、不缩回默认），从而既适应新分辨率又保住窗口大小。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Manager};

use super::position;
use super::{get_window, CLIPBOARD_WINDOW_LABEL};

/// 轮询间隔。显示器枚举开销极小，2 秒足以快速响应且功耗可忽略。
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// 启动后延迟首次检测：等待窗口 / 设置完成初始化，避免启动期枚举噪声误触发。
const STARTUP_DELAY: Duration = Duration::from_secs(5);

/// 快照变化后的确认延迟：滤掉改分辨率过程中的过渡态，回弹则忽略。
const CHANGE_CONFIRM_DELAY: Duration = Duration::from_secs(1);

/// 检测到显示器变化但窗口仍可见时的挂起重启标记，窗口隐藏后消费。
static PENDING_RESTART: AtomicBool = AtomicBool::new(false);

/// 取走挂起的重启标记；仅在窗口隐藏路径调用，取到 `true` 后调用方应立即重启。
pub fn take_pending_restart() -> bool {
    PENDING_RESTART.swap(false, Ordering::SeqCst)
}

/// 启动显示器快照轮询任务。在整个应用生命周期持续运行。
pub fn spawn(app_handle: &AppHandle) {
    let handle = app_handle.clone();

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(STARTUP_DELAY).await;

        let Some(initial) = snapshot(&handle) else {
            log::warn!("monitor watch: initial snapshot unavailable, watcher disabled");
            return;
        };
        log::info!("monitor watch started: {initial}");

        let mut last = initial;

        loop {
            tokio::time::sleep(POLL_INTERVAL).await;

            let Some(current) = snapshot(&handle) else {
                continue;
            };
            if current == last {
                continue;
            }

            // 过渡态确认：分辨率切换瞬间枚举可能返回中间值，确认后仍不同才动作。
            tokio::time::sleep(CHANGE_CONFIRM_DELAY).await;
            let Some(confirmed) = snapshot(&handle) else {
                continue;
            };
            if confirmed == last {
                continue;
            }

            log::warn!("monitor configuration changed: {last} -> {confirmed}");
            last = confirmed;
            on_monitor_changed(&handle);
        }
    });
}

/// 把所有显示器的物理尺寸、位置与缩放系数序列化为稳定排序的快照字符串。
/// 任一值变化（改分辨率 / 改缩放 / 插拔显示器）都会改变快照。
fn snapshot(app_handle: &AppHandle) -> Option<String> {
    let window = app_handle.webview_windows().into_values().next()?;

    let monitors = window.available_monitors().ok()?;
    let mut parts: Vec<String> = monitors
        .iter()
        .map(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            format!(
                "{}x{}@{},{}x{}",
                size.width,
                size.height,
                monitor.scale_factor(),
                position.x,
                position.y
            )
        })
        .collect();
    // 排序稳定化：枚举顺序不影响快照，避免顺序抖动误报。
    parts.sort();

    Some(parts.join("|"))
}

/// 显示器配置变化后的处理：保留用户保存的窗口几何，按窗口可见性决定立即重启还是延后。
fn on_monitor_changed(app_handle: &AppHandle) {
    // 不清空几何存档：存档尺寸是用户偏好，恢复时由 `restore_window_state` 钳制到
    // 新屏幕内（超出则压到边界、不缩回默认），既适应新分辨率又保住窗口大小。

    // 窗口可见：先夹回屏幕边界保证当前会话可操作，等窗口隐藏后再重启。
    let clipboard_visible = app_handle
        .get_webview_window(CLIPBOARD_WINDOW_LABEL)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false);

    if clipboard_visible {
        PENDING_RESTART.store(true, Ordering::SeqCst);

        match get_window(app_handle, CLIPBOARD_WINDOW_LABEL) {
            Ok(window) => {
                if let Err(err) = position::clamp_within_screen(&window) {
                    log::warn!("clamp clipboard window on monitor change failed: {err}");
                }
            }
            Err(err) => log::warn!("get clipboard window on monitor change failed: {err}"),
        }

        return;
    }

    // 3. 不可见：立即重启。
    log::info!("monitor configuration changed, restarting app");
    app_handle.restart();
}
