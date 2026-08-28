//! 写回回环抑制。
//!
//! 应用自身写回剪贴板会触发 OS 监听，进而被当成一次新的
//! 复制再次入库，形成回环。写回前调用 [`WritebackGuard::suppress`] 登记将写入内容的
//! `content_hash`；监听回调读到内容后调用 [`WritebackGuard::should_skip`]，命中则跳过本次入库。
//!
//! 用 `content_hash` 比对而非简单布尔标记：避免「写回事件尚未到达就来了一次真实复制」
//! 误伤真实复制；同时带 TTL 兜底——若写回的内容与剪贴板现状完全相同（OS 可能不发变更事件），
//! 登记的指纹不会永久滞留导致后续同内容复制被吞。HTML/RTF 写回会同时写入纯文本回退，
//! 因此 guard 支持短期登记多个指纹。
//!
//! 此外维护「最近一次自身写回」时间戳：前缀式跳过比 hash 比对更早生效——hash 必须先完整
//! 读取剪贴板（图片要解码 + 编码 + 落盘）才能算出比较，而读取本身就会持有剪贴板锁、与
//! 目标应用的粘贴（Ctrl+V/⌘V 需 `OpenClipboard`）竞争，还会在图片字节往返不保真时让抑制
//! 落空。watcher 靠 [`recent_self_write`](Self::recent_self_write) 在读取前直接早退。

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 登记的写回指纹在多久内有效。写回后监听事件通常在毫秒级到达，
/// 给足冗余但不至于长到误吞后续的真实复制。
const SUPPRESS_TTL: Duration = Duration::from_secs(2);

/// 自身写回后变更事件允许到达的窗口：窗口内的变更事件被 watcher 视为自身触发器直接跳过，
/// **不读取**剪贴板。Windows 事件驱动毫秒级到达；macOS 按 [`SELF_WRITE_POLL_INTERVAL_MS`]
/// 轮询 120ms 一拍，给到 300ms 覆盖两拍冗余；过长的窗口会误吞紧随其后的真实外部复制。
/// 晚于窗口的事件仍由 hash 抑制（[`should_skip`](Self::should_skip)）兜底。
pub const SELF_WRITE_SKIP_WINDOW: Duration = Duration::from_millis(300);

/// 与 [`super::watcher::CLIPBOARD_POLL_INTERVAL`]（120ms）对应，供注释引用。
#[allow(dead_code)]
const SELF_WRITE_POLL_INTERVAL_MS: u64 = 120;

pub struct WritebackGuard {
    pending: Mutex<Vec<Pending>>,
    last_self_write: Mutex<Option<Instant>>,
}

struct Pending {
    content_hash: String,
    at: Instant,
}

impl Default for WritebackGuard {
    fn default() -> Self {
        Self {
            pending: Mutex::new(Vec::new()),
            last_self_write: Mutex::new(None),
        }
    }
}

impl WritebackGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// 写回剪贴板前登记将写入内容的 `content_hash`。
    pub fn suppress(&self, content_hash: String) {
        let mut pending = self.pending.lock().expect("writeback guard poisoned");
        pending.retain(|p| p.at.elapsed() <= SUPPRESS_TTL);
        pending.push(Pending {
            content_hash,
            at: Instant::now(),
        });
    }

    /// 监听回调判断本次变更是否为自身写回所致：命中登记指纹（且未过期）则返回 `true`
    /// 并消费掉登记；否则返回 `false`。过期的登记顺带清理。
    pub fn should_skip(&self, content_hash: &str) -> bool {
        let mut pending = self.pending.lock().expect("writeback guard poisoned");
        pending.retain(|p| p.at.elapsed() <= SUPPRESS_TTL);

        let Some(index) = pending.iter().position(|p| p.content_hash == content_hash) else {
            return false;
        };
        pending.remove(index);

        true
    }

    /// 写回剪贴板成功后调用：记录本次自身写回时间戳，供 watcher 在窗口内跳过整条读取。
    /// 放在各 `set_*` 成功之后，避免写入失败时误吞紧随其后的真实外部复制。
    pub fn mark_self_write(&self) {
        *self
            .last_self_write
            .lock()
            .expect("writeback guard poisoned") = Some(Instant::now());
    }

    /// watcher 在读取剪贴板前调用：若距最近一次自身写回不超过 [`SELF_WRITE_SKIP_WINDOW`]，
    /// 返回 `true`，调用方应直接跳过本次变更事件（不读取、不入库、不 emit）。
    pub fn recent_self_write(&self) -> bool {
        self.last_self_write
            .lock()
            .expect("writeback guard poisoned")
            .is_some_and(|at| at.elapsed() <= SELF_WRITE_SKIP_WINDOW)
    }

    /// 单测用：直接塞一条已过期登记。
    #[cfg(test)]
    fn suppress_expired_for_test(&self, content_hash: String) {
        let mut pending = self.pending.lock().expect("writeback guard poisoned");
        pending.push(Pending {
            content_hash,
            at: Instant::now() - SUPPRESS_TTL - Duration::from_millis(1),
        });
    }

    /// 单测用：把「最近一次自身写回」拨到窗口之外。
    #[cfg(test)]
    fn expire_self_write_for_test(&self) {
        *self
            .last_self_write
            .lock()
            .expect("writeback guard poisoned") =
            Some(Instant::now() - SELF_WRITE_SKIP_WINDOW - Duration::from_millis(1));
    }

    /// 单测用：返回当前登记数量。
    #[cfg(test)]
    fn pending_len_for_test(&self) -> usize {
        let pending = self.pending.lock().expect("writeback guard poisoned");

        pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_once_then_resets() {
        let guard = WritebackGuard::new();
        guard.suppress("hash-a".to_owned());

        assert!(guard.should_skip("hash-a"));
        // 登记已消费，同内容的下一次（真实复制）不再被吞。
        assert!(!guard.should_skip("hash-a"));
    }

    #[test]
    fn supports_multiple_pending_hashes() {
        let guard = WritebackGuard::new();
        guard.suppress("hash-a".to_owned());
        guard.suppress("hash-b".to_owned());

        assert!(guard.should_skip("hash-a"));
        assert!(guard.should_skip("hash-b"));
        assert_eq!(guard.pending_len_for_test(), 0);
    }

    #[test]
    fn does_not_skip_unrelated_content() {
        let guard = WritebackGuard::new();
        guard.suppress("hash-a".to_owned());

        // 写回事件未到，先来了一次别的真实复制 → 不该被吞，登记仍在。
        assert!(!guard.should_skip("hash-b"));
        assert!(guard.should_skip("hash-a"));
    }

    #[test]
    fn expired_suppression_is_ignored() {
        let guard = WritebackGuard::new();
        guard.suppress_expired_for_test("hash-a".to_owned());

        assert!(!guard.should_skip("hash-a"));
    }

    #[test]
    fn mark_self_write_triggers_recent() {
        let guard = WritebackGuard::new();
        assert!(!guard.recent_self_write());

        guard.mark_self_write();
        assert!(guard.recent_self_write());
    }

    #[test]
    fn expired_self_write_no_longer_recent() {
        let guard = WritebackGuard::new();
        guard.mark_self_write();
        assert!(guard.recent_self_write());

        guard.expire_self_write_for_test();
        assert!(!guard.recent_self_write());
    }
}
