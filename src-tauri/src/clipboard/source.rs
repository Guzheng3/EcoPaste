//! 「这次剪贴板变更来自哪个应用」的探测：在剪贴板事件回调里调用，
//! 返回稳定 id（macOS bundle id / Windows exe 绝对路径）、显示名、可选的 icon PNG 字节。
//!
//! 必须**同步**在监听回调一发生时立即抓——延后到 await 之后再问，前台应用很可能已经切走。
//! 探测失败（无前台应用 / 自身复制 / 平台 API 错误）一律返回 `None`，不阻断入库。
//!
//! 平台 API：macOS 走 `NSWorkspace.frontmostApplication`，Windows 走 `GetForegroundWindow`
//! + `QueryFullProcessImageNameW`。图标统一交给 `crate::clipboard::icon` 跨平台抽取。

use crate::db::models::Platform;

#[derive(Debug, Clone)]
pub struct FrontmostApp {
    /// 稳定主键。macOS = bundle id（如 `com.apple.Safari`），Windows = exe 绝对路径。
    pub id: String,
    /// 显示名（localizedName / FileDescription / exe stem 的优先回落）。
    pub name: String,
    pub platform: Platform,
    /// 应用图标的 PNG 字节；提取失败则 `None`。
    pub icon_png: Option<Vec<u8>>,
}

/// 启动 Windows 前台窗口追踪（非 Windows 平台上为空操作）。
/// 提高「这次复制来自哪个应用」的归属准确度：复制瞬间前台很可能已切走，
/// 用后台 hook 记录的最近活跃窗口兜底。应在 setup 阶段调用一次。
#[cfg(not(target_os = "windows"))]
pub fn init_window_tracking() {}
#[cfg(target_os = "windows")]
pub fn init_window_tracking() {
    windows::init_window_tracking();
}

/// 探测当前前台应用。失败不报错，只在 trace 级别记日志（监听回调高频，避免噪声）。
pub fn detect_frontmost() -> Option<FrontmostApp> {
    #[cfg(target_os = "macos")]
    {
        macos::detect()
    }
    #[cfg(target_os = "windows")]
    {
        windows::detect()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{FrontmostApp, Platform};
    use crate::clipboard::icon;

    use std::path::PathBuf;

    use objc2::msg_send;
    use objc2::rc::{autoreleasepool, Retained};
    use objc2_app_kit::{NSRunningApplication, NSWorkspace};
    use objc2_foundation::{NSString, NSURL};

    pub(super) fn detect() -> Option<FrontmostApp> {
        autoreleasepool(|_| {
            let workspace = NSWorkspace::sharedWorkspace();
            let app = workspace.frontmostApplication()?;

            // 没 bundle id 的进程（命令行子进程等）不入表，避免主键不稳定。
            let id = app.bundleIdentifier().map(|s| s.to_string())?;
            let name = app
                .localizedName()
                .map(|s| s.to_string())
                .unwrap_or_else(|| id.clone());

            let icon_png =
                unsafe { bundle_path(&app) }.and_then(|path| icon::icon_png(&path, None));

            Some(FrontmostApp {
                id,
                name,
                platform: Platform::Macos,
                icon_png,
            })
        })
    }

    /// 通过 NSRunningApplication.bundleURL 拿到 .app 路径。objc2-app-kit 当前 feature 没生成
    /// 该 getter，只能 msg_send!；返回 NSURL 后用 path 取 NSString → Rust String。
    unsafe fn bundle_path(app: &NSRunningApplication) -> Option<PathBuf> {
        let url: Option<Retained<NSURL>> = msg_send![app, bundleURL];
        let url = url?;
        let path: Option<Retained<NSString>> = msg_send![&*url, path];
        Some(PathBuf::from(path?.to_string()))
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use super::{FrontmostApp, Platform};
    use crate::clipboard::app_name;
    use crate::clipboard::icon;

    use std::path::Path;
    use std::sync::atomic::{AtomicIsize, Ordering};

    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::DataExchange::GetClipboardOwner;
    use windows::Win32::System::ProcessStatus::GetModuleFileNameExW;
    use windows::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    };
    use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetClassNameW, GetForegroundWindow, GetMessageW,
        GetWindowThreadProcessId, IsWindowVisible, TranslateMessage, EVENT_SYSTEM_FOREGROUND, MSG,
        WINEVENT_OUTOFCONTEXT,
    };

    /// TieZ 思路：前台窗口切换由 `SetWinEventHook` 实时记录，作为取源兜底。
    /// 剪贴板事件回调触发时，若前台早已切回我们自己、或落在系统托盘上，
    /// 就用「最近一次用户激活的窗口」来定位真正来源。存的是 HWND 的 isize 表示（0 = 未记录）。
    static LAST_ACTIVE_HWND: AtomicIsize = AtomicIsize::new(0);

    pub(super) fn detect() -> Option<FrontmostApp> {
        // 三级回退定位「这次复制来自哪个窗口」：剪贴板所有者 → 前台窗口 → 最近活跃窗口。
        // 前两者在进程内跨线程调用是安全的（只读查询）；求稳都包在 unsafe 里。
        let exe_path = unsafe { source_exe_path() }?;
        // 自身写回事件依赖 WritebackGuard 的 content_hash 判定，这里不过滤自身——
        // 与 macOS 行为一致：哪怕拿到的是 EcoPaste 自己，guard 也会在下游 short-circuit。
        // 显示名优先版本资源 FileDescription（如 chrome.exe → "Google Chrome"）。
        let name = app_name::exe_display_name(Path::new(&exe_path));
        let icon_png = icon::icon_png(Path::new(&exe_path), None);

        Some(FrontmostApp {
            id: exe_path,
            name,
            platform: Platform::Windows,
            icon_png,
        })
    }

    /// 启动后台窗口追踪线程。`SetWinEventHook` 需要在该线程内跑消息循环（GetMessageW）
    /// 才能在事件发生时收到回调；hook 句柄只要线程存活就保持有效，drop 到循环外面去。
    /// 应在 setup 里调用一次，失败只记日志，不影响启动。
    pub fn init_window_tracking() {
        std::thread::Builder::new()
            .name("window-tracker".to_owned())
            .spawn(|| unsafe {
                // 注册前台窗口变化 hook，持续记录最近一个有效的用户窗口。
                let _hook: HWINEVENTHOOK = SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    None,
                    Some(event_hook_callback),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                );

                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            })
            .expect("failed to spawn window tracker thread");
    }

    unsafe extern "system" fn event_hook_callback(
        _hook: HWINEVENTHOOK,
        event: u32,
        hwnd: HWND,
        _id_object: i32,
        _id_child: i32,
        _thread: u32,
        _time: u32,
    ) {
        if event != EVENT_SYSTEM_FOREGROUND || hwnd.0 == 0 {
            return;
        }
        // 只记录真实用户窗口：跳过不可见、自身进程、系统/托盘窗口。
        if !IsWindowVisible(hwnd).as_bool() {
            return;
        }
        if is_own_process_window(hwnd) {
            return;
        }
        if is_system_focus_window(hwnd) {
            return;
        }
        LAST_ACTIVE_HWND.store(hwnd.0, Ordering::SeqCst);
    }

    /// 三级回退定位来源 exe 绝对路径。
    unsafe fn source_exe_path() -> Option<String> {
        // 3. 兜底：后台追踪到的最近活跃窗口。
        let last_active = LAST_ACTIVE_HWND.load(Ordering::SeqCst);
        if last_active != 0 {
            if let Some(path) = exe_path_from_hwnd(HWND(last_active)) {
                return Some(path);
            }
        }
        // 2. 前台窗口（常见场景：复制时前台就是来源应用）。
        let foreground: HWND = GetForegroundWindow();
        if !is_own_process_window(foreground) && !is_system_focus_window(foreground) {
            if let Some(path) = exe_path_from_hwnd(foreground) {
                return Some(path);
            }
        }
        // 1. 剪贴板所有者：谁把内容放进剪贴板，谁就是最可靠的来源。
        let owner: HWND = GetClipboardOwner();
        if !is_own_process_window(owner) {
            if let Some(path) = exe_path_from_hwnd(owner) {
                return Some(path);
            }
        }
        None
    }

    unsafe fn exe_path_from_hwnd(hwnd: HWND) -> Option<String> {
        if hwnd.0 == 0 {
            return None;
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }

        // OpenProcess 需该进程可查询；失败一律返回 None（不阻断入库）。
        if let Ok(handle) = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid) {
            let mut buf = [0u16; 1024];
            let len = GetModuleFileNameExW(handle, None, &mut buf);
            let ok = len > 0;
            let path = ok.then(|| String::from_utf16_lossy(&buf[..len as usize]));
            let _ = windows::Win32::Foundation::CloseHandle(handle);
            return path.filter(|p| !p.is_empty());
        }
        None
    }

    fn is_own_process_window(hwnd: HWND) -> bool {
        if hwnd.0 == 0 {
            return false;
        }
        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        pid != 0 && pid == unsafe { GetCurrentProcessId() }
    }

    /// 任务栏、托盘、系统 UI 覆盖层等不应作为来源的窗口类。
    fn is_system_focus_window(hwnd: HWND) -> bool {
        if hwnd.0 == 0 {
            return true;
        }
        let mut class_name = [0u16; 256];
        let len = unsafe { GetClassNameW(hwnd, &mut class_name) };
        let class_str = if len > 0 {
            String::from_utf16_lossy(&class_name[..len as usize])
        } else {
            String::new()
        };
        matches!(
            class_str.as_str(),
            "Shell_TrayWnd"
                | "Shell_SecondaryTrayWnd"
                | "TrayNotifyWnd"
                | "NotifyIconOverflowWindow"
                | "ReBarWindow32"
                | "MSTaskSwWClass"
                | "ImmersiveLauncher"
                | "ShellExperienceHost"
                | "TaskSwitcherWnd"
                | "MultitaskingViewFrame"
        )
    }
}
