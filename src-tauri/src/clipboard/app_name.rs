//! Windows exe 显示名：优先读版本资源里的 `FileDescription`（真实软件名，
//! 如 `chrome.exe` → "Google Chrome"），其次 `ProductName`，最后回落 exe 文件名 stem。
//! `source.rs`（前台来源探测）与 `apps_registry.rs`（运行中应用扫描）共用。

#[cfg(target_os = "windows")]
mod windows {
    use std::path::Path;

    use winapi::ctypes::c_void;
    use winapi::shared::minwindef::{FALSE, UINT};
    use winapi::um::winver::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};

    /// exe 的展示名：FileDescription → ProductName → exe stem。
    pub fn exe_display_name(path: &Path) -> String {
        let info = VersionInfo::load(path);

        if let Some(info) = info {
            for field in ["FileDescription", "ProductName"] {
                if let Some(name) = info.string_field(field) {
                    return name;
                }
            }
        }

        fallback_stem(path)
    }

    fn fallback_stem(path: &Path) -> String {
        if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
            return name.to_owned();
        }

        path.to_string_lossy().into_owned()
    }

    /// 已加载的 exe 版本资源块，字段查询共用一次文件读取。
    struct VersionInfo {
        buffer: Vec<u8>,
    }

    impl VersionInfo {
        fn load(path: &Path) -> Option<Self> {
            let path_w = path_to_wide(path);
            let buffer = unsafe {
                let size = GetFileVersionInfoSizeW(path_w.as_ptr(), std::ptr::null_mut());
                if size == 0 {
                    return None;
                }

                let mut buffer = vec![0u8; size as usize];
                let ok = GetFileVersionInfoW(
                    path_w.as_ptr(),
                    0,
                    size,
                    buffer.as_mut_ptr() as *mut c_void,
                );
                if ok == FALSE {
                    return None;
                }
                buffer
            };

            Some(Self { buffer })
        }

        /// 遍历版本资源声明的语言/代码页组合，返回第一个非空的字符串字段。
        fn string_field(&self, field: &str) -> Option<String> {
            for (lang, codepage) in self.translations() {
                let sub_block = str_to_wide(&format!(
                    "\\StringFileInfo\\{lang:04x}{codepage:04x}\\{field}"
                ));
                let value = unsafe { self.query_wide_string(&sub_block) };
                if let Some(value) = value {
                    return Some(value);
                }
            }

            None
        }

        /// `\VarFileInfo\Translation`：语言 id（低 16 位）+ 代码页（高 16 位）列表。
        fn translations(&self) -> Vec<(u16, u16)> {
            let sub_block = str_to_wide("\\VarFileInfo\\Translation");

            unsafe {
                let mut ptr: *mut c_void = std::ptr::null_mut();
                let mut len: UINT = 0;
                let ok = VerQueryValueW(
                    self.buffer.as_ptr() as *const c_void,
                    sub_block.as_ptr(),
                    &mut ptr,
                    &mut len,
                );
                if ok == FALSE || ptr.is_null() || len < 4 {
                    return Vec::new();
                }

                // len 单位是字节，每对语言/代码页占 4 字节。
                let pairs = ptr as *const u32;
                (0..(len as usize) / 4)
                    .map(|i| {
                        let value = *pairs.add(i);
                        (value as u16, (value >> 16) as u16)
                    })
                    .collect()
            }
        }

        /// 按子块取字符串字段；读到 null 结尾为止（len 对字符串值的单位文档未保证
        /// 是字符数，故只作上限参考），空白值视作缺失以便尝试下一字段。
        unsafe fn query_wide_string(&self, sub_block: &[u16]) -> Option<String> {
            let mut ptr: *mut c_void = std::ptr::null_mut();
            let mut len: UINT = 0;
            let ok = VerQueryValueW(
                self.buffer.as_ptr() as *const c_void,
                sub_block.as_ptr(),
                &mut ptr,
                &mut len,
            );
            if ok == FALSE || ptr.is_null() {
                return None;
            }

            let wide = ptr as *const u16;
            // VerQueryValueW 对字符串值返回的 len 已是字符数（含或不含 null）。
            // 不加除以 2，直接以 len 为上限避免截断。
            let max_chars = len as usize + 1;
            let mut end = 0usize;
            while end < max_chars && *wide.add(end) != 0 {
                end += 1;
            }

            let value = String::from_utf16_lossy(std::slice::from_raw_parts(wide, end));
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return None;
            }

            Some(trimmed.to_owned())
        }
    }

    fn path_to_wide(path: &Path) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;

        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn str_to_wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::path::PathBuf;

        #[test]
        fn missing_file_falls_back_to_stem() {
            let path = PathBuf::from(r"C:\nonexistent\SomeApp.exe");
            assert_eq!(exe_display_name(&path), "SomeApp");
        }

        #[test]
        fn system_exe_returns_non_empty_name() {
            let path = PathBuf::from(r"C:\Windows\System32\notepad.exe");
            if !path.exists() {
                eprintln!("skip: notepad.exe not present");
                return;
            }

            let name = exe_display_name(&path);
            assert!(!name.is_empty());
            println!("notepad display name: {name}");
        }
    }
}

#[cfg(target_os = "windows")]
pub use windows::exe_display_name;
