mod app_name;
mod app_store;
mod apps_registry;
mod cleanup;
mod detect;
mod entities;
mod file_icon_store;
mod guard;
mod icon;
mod ingest;
mod payload;
mod read;
mod secrets;
mod segment;
mod source;
mod storage;
mod watcher;
mod write;

pub use app_store::AppIconStore;
pub use apps_registry::{
    add_app_from_path, delete_unreferenced_apps, refresh_running_apps, AppsRegistry,
};
pub use detect::sanitize_css_color;
pub use entities::{extract_entities, ExtractedEntity};
pub use file_icon_store::FileIconStore;
pub use guard::WritebackGuard;
pub use icon::{get_icon_cache_key, icon_png, DIR_CACHE_KEY};
#[cfg(test)]
pub use ingest::build_item;
pub use ingest::{build_item_with_settings, build_item_with_source};
pub use payload::{ClipboardPayload, ImagePayload, TextPayload};
pub use read::ClipboardReader;
pub use segment::segment_text;
pub use source::{detect_frontmost, init_window_tracking};
pub use storage::ImageStore;
pub use watcher::{init, materialize_source, persist_and_notify, WatcherPause};
pub use write::write_to_clipboard;

#[cfg(test)]
pub(crate) mod test_lock {
    use std::sync::{Mutex, MutexGuard};

    /// 系统剪贴板是单一全局资源，触碰它的测试不能并行（否则相互覆盖内容而 flaky）。
    /// 这些测试统一持有此锁串行执行，即使用默认多线程 runner 也稳定。
    static LOCK: Mutex<()> = Mutex::new(());

    /// 获取串行锁。即便上一个持锁测试 panic 导致锁中毒，也恢复使用——
    /// 锁仅用于串行化，内部 `()` 无状态可破坏。
    pub fn serial() -> MutexGuard<'static, ()> {
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
