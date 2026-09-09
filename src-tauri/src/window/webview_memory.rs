//! WebView2 内存目标级别控制（仅 Windows 实际生效）。
//!
//! 窗口隐藏（hiddenWarm / dormant）时把 WebView2 的 MemoryUsageTargetLevel 压到
//! `Low`，让浏览器进程压缩内存（官方数据可降 70-90%）；显示前恢复 `Normal`，
//! 保证前台全速渲染。相比 `TrySuspend`，该 API 异步非阻塞、不要求窗口完全不可见，
//! 专为后台常驻设计。
//!
//! 接口位于 `ICoreWebView2_19`（WebView2 Runtime 1.0.2210.55+）：本机 Runtime 过旧
//! 导致 QI 失败时只告警一次并整体降级为 no-op，不影响任何功能。

#[cfg(target_os = "windows")]
mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};

    use tauri::WebviewWindow;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
    };
    use windows_core::Interface;

    use crate::core::Result;

    /// Runtime 不支持目标接口属于环境事实，只告警一次避免每次隐藏都刷日志。
    static RUNTIME_UNSUPPORTED_WARNED: AtomicBool = AtomicBool::new(false);

    /// 设置窗口 WebView2 的内存目标级别：`low = true` 压缩内存，`false` 恢复正常。
    ///
    /// 仅 `with_webview` 派发失败时返回 Err；COM 层失败只记日志，不影响调用方流程。
    pub fn set_memory_usage_target(window: &WebviewWindow, low: bool) -> Result<()> {
        let label = window.label().to_owned();
        let target = if low {
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
        } else {
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
        };

        // 闭包被派发到创建 WebView 的主线程执行，满足 WebView2 COM 的 STA 线程约束；
        // 与 `window.show()` 同为主线程队列任务，先入队先执行，保证显示前已完成恢复。
        let closure_label = label.clone();
        window
            .with_webview(move |webview| {
                let label = closure_label;
                unsafe {
                    let Ok(core) = webview.controller().CoreWebView2() else {
                        warn_runtime_unsupported(&label);
                        return;
                    };
                    let Ok(webview19) = core.cast::<ICoreWebView2_19>() else {
                        warn_runtime_unsupported(&label);
                        return;
                    };
                    if let Err(err) = webview19.SetMemoryUsageTargetLevel(target) {
                        log::warn!("set webview2 memory usage target failed for {label}: {err}");
                    }
                }
            })
            .map_err(|err| anyhow::anyhow!("dispatch webview memory target for {label}: {err}"))?;

        Ok(())
    }

    /// Runtime 缺少目标 COM 接口（WebView2 过旧）时的一次性告警。
    fn warn_runtime_unsupported(label: &str) {
        if RUNTIME_UNSUPPORTED_WARNED.swap(true, Ordering::Relaxed) {
            return;
        }

        log::warn!(
            "webview2 runtime does not support MemoryUsageTargetLevel (ICoreWebView2_19), \
             memory compression disabled (window: {label})"
        );
    }
}

#[cfg(target_os = "windows")]
pub use imp::set_memory_usage_target;

/// 非 Windows 平台无 WebView2，保持 no-op，调用方无需平台分支。
#[cfg(not(target_os = "windows"))]
pub fn set_memory_usage_target(
    _window: &tauri::WebviewWindow,
    _low: bool,
) -> crate::core::Result<()> {
    Ok(())
}
