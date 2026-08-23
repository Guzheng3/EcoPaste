//! 剪贴板图片（无原始文件路径，如从网页 / App 复制）的临时落盘。
//!
//! 这类图片只有像素字节、无法追溯到磁盘原文件，因此不做长期存档，而是存到应用数据下的
//! `temp` 目录、按「来源 + 时间」命名，由调度器 24 小时后连同数据库记录一起清除。
//! 目录布局（`content` 字段存文件名，读取路径由文件名现算，不入库）：
//! ```text
//! <app_local_data>/resources/clipboard-images/
//!   temp/<来源>_<时间>.png              原图（PNG）
//!   temp/thumbnails/<同上>.png          缩略图（PNG，最长边 <= THUMBNAIL_MAX），首次预览时按需生成
//! ```
//!
//! 早期版本曾按「PNG 字节哈希分片」永久存档（`origin/<ab>/<hash>.png`）。为保证已存记录仍能
//! 预览，`origin_path` / `thumbnail_path` 在 temp 下找不到时回退到旧布局路径；`remove` 则两个
//! 布局都清理。新写入一律只进 temp。
//!
//! 缩略图解码/缩放/编码不在复制热路径上——`store` 只写原图，缩略图由
//! [`ImageStore::ensure_thumbnail`] 在前端首次取图时懒生成并缓存。

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use super::payload::ImagePayload;
use crate::core::{AppError, Result};
use anyhow::Context;
use blake3::Hasher;
use chrono::Utc;
use clipboard_rs::common::{RustImage, RustImageData};
use tauri::AppHandle;

/// 缩略图最长边像素。仅用于列表预览，够清晰即可。
const THUMBNAIL_MAX: u32 = 300;

/// 剪贴板图片目录名，挂在 `core::paths::resources_dir` 下（与 `app-icons` 并列）。
const IMAGES_DIR: &str = "clipboard-images";
/// 临时（无原始路径）图片存放目录名。
const TEMP_DIR: &str = "temp";
/// 缩略图子目录名（挂在 temp 之下）。
const THUMBNAILS_DIR: &str = "thumbnails";
/// 来源名参与文件名的最大长度（截断，避免超长文件名 / 深目录问题）。
const MAX_SOURCE_CHARS: usize = 40;

/// 一次图片落盘的结果，交给 ingest 写入 `ClipboardItem`。
pub struct StoredImage {
    /// 入库 `content`：图片文件名 `<来源>_<时间>.png`（不含目录）。
    pub file_name: String,
    /// 去重指纹来源：PNG 字节的 blake3（十六进制）。与文件名无关，保证同期去重稳定。
    #[allow(dead_code)]
    pub content_digest: String,
    pub width: i64,
    pub height: i64,
    /// 原图字节数。
    pub size: i64,
}

/// 图片存储器：持有 app data 下的 `resources/clipboard-images` 根目录。
/// 放入 Tauri `State`，监听线程与命令共用。
#[derive(Clone)]
pub struct ImageStore {
    images_root: Arc<RwLock<PathBuf>>,
}

impl ImageStore {
    /// 从 `AppHandle` 解析 `<app_local_data>/resources/clipboard-images` 作为根。
    pub fn new(app: &AppHandle) -> Result<Self> {
        let images_root = crate::core::paths::resources_dir(app)?.join(IMAGES_DIR);
        Ok(Self {
            images_root: Arc::new(RwLock::new(images_root)),
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(images_root: PathBuf) -> Self {
        Self {
            images_root: Arc::new(RwLock::new(images_root)),
        }
    }

    /// 重新绑定到当前真实数据根；数据目录热迁移后由存储命令调用。
    pub fn rebase(&self, app: &AppHandle) -> Result<()> {
        let next = crate::core::paths::resources_dir(app)?.join(IMAGES_DIR);
        *self
            .images_root
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = next;
        Ok(())
    }

    /// 落盘原图到临时目录，返回 [`StoredImage`]。
    ///
    /// `source` 为来源应用名，用于文件名前缀；`None` 时退化为 `unknown`。文件名含毫秒级时间，
    /// 不做幂等去写（同来源同毫秒不同图撞名概率极低）。缩略图不在此生成，
    /// 由 [`Self::ensure_thumbnail`] 在前端首次预览时懒生成。
    pub fn store(&self, image: &ImagePayload, source: Option<&str>) -> Result<StoredImage> {
        let content_digest = blake3_hex(&image.bytes);
        let file_name = temp_file_name(source);

        let origin_path = self.temp_origin_path(&file_name);
        write_file(&origin_path, &image.bytes)?;

        Ok(StoredImage {
            file_name,
            content_digest,
            width: i64::from(image.width),
            height: i64::from(image.height),
            size: image.bytes.len() as i64,
        })
    }

    /// 确保缩略图存在并返回其绝对路径：已存在直接返回；否则读原图 → 解码 → 缩放 → 编码 PNG → 落盘。
    ///
    /// 供 `get_clipboard_image_path(thumbnail=true)` 调用。把生成放在「读」而非「写」侧，
    /// 既将解码/编码移出复制热路径，又因「返回前文件已确保存在」天然避免前端加载到半成品文件。
    pub fn ensure_thumbnail(&self, file_name: &str) -> Result<PathBuf> {
        let thumb_path = self.temp_thumbnail_path(file_name);
        if thumb_path.exists() {
            return Ok(thumb_path);
        }

        let origin_bytes = std::fs::read(self.origin_file(file_name))
            .with_context(|| format!("failed to read origin image {file_name}"))?;
        let thumb_bytes = encode_thumbnail(&origin_bytes)?;
        write_file(&thumb_path, &thumb_bytes)?;
        Ok(thumb_path)
    }

    /// 删除一张图片的临时原图 / 缩略图，并顺带清理旧布局下的同名文件（幂等，缺失视作成功）。
    /// 尽力清理变空目录。调用前提：库里该图文件名唯一可判定为孤儿（temp 语义），
    /// 其余 IO 错误上抛由调用方记日志。
    pub fn remove(&self, file_name: &str) -> Result<()> {
        for path in [
            self.temp_origin_path(file_name),
            self.temp_thumbnail_path(file_name),
            self.legacy_origin_path(file_name),
            self.legacy_thumbnail_path(file_name),
        ] {
            remove_if_present(&path)?;
            remove_dir_if_empty(path.parent());
        }
        Ok(())
    }

    /// 由文件名解析原图绝对路径。优先 temp；temp 不存在时回退旧 hash 分片布局（兼容存量）。
    pub fn origin_path(&self, file_name: &str) -> PathBuf {
        let temp = self.temp_origin_path(file_name);
        if temp.exists() {
            temp
        } else {
            self.legacy_origin_path(file_name)
        }
    }

    /// 由文件名解析缩略图绝对路径。优先 temp；否则回退旧 hash 分片布局（兼容存量）。
    pub fn thumbnail_path(&self, file_name: &str) -> PathBuf {
        let temp = self.temp_thumbnail_path(file_name);
        if temp.exists() {
            temp
        } else {
            self.legacy_thumbnail_path(file_name)
        }
    }

    /// 读取用：定位「原图文件」真实路径（temp 或旧布局中存在者）。供 `ensure_thumbnail` 读源。
    fn origin_file(&self, file_name: &str) -> PathBuf {
        let temp = self.temp_origin_path(file_name);
        if temp.exists() {
            temp
        } else {
            self.legacy_origin_path(file_name)
        }
    }

    fn temp_origin_path(&self, file_name: &str) -> PathBuf {
        self.temp_dir().join(file_name)
    }

    fn temp_thumbnail_path(&self, file_name: &str) -> PathBuf {
        self.temp_dir().join(THUMBNAILS_DIR).join(file_name)
    }

    fn temp_dir(&self) -> PathBuf {
        self.images_root().join(TEMP_DIR)
    }

    /// 旧布局（hash 分片永久存档）的原图路径——仅用于读取存量与清理，不再写入。
    fn legacy_origin_path(&self, file_name: &str) -> PathBuf {
        self.legacy_shard_path("origin", file_name)
    }

    /// 旧布局（hash 分片永久存档）的缩略图路径——仅用于读取存量与清理，不再写入。
    fn legacy_thumbnail_path(&self, file_name: &str) -> PathBuf {
        self.legacy_shard_path(THUMBNAILS_DIR, file_name)
    }

    fn legacy_shard_path(&self, kind_dir: &str, file_name: &str) -> PathBuf {
        self.images_root()
            .join(kind_dir)
            .join(shard_dir(shard_key(file_name)))
            .join(file_name)
    }

    fn images_root(&self) -> PathBuf {
        self.images_root
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// 由「来源应用名 + 当前时间」生成 temp 图片文件名，形如 `Chrome_20260823_214500.123.png`。
/// 毫秒保证同一秒内同来源的不同图不撞名；来源名清洗非法字符后截断。
fn temp_file_name(source: Option<&str>) -> String {
    let src = sanitize_source(source);
    let ts = Utc::now().format("%Y%m%d_%H%M%S%.3f");
    format!("{src}_{ts}.png")
}

/// 清洗来源名用于文件名：替换路径非法字符与空白为 `_`，去首尾空白/点，空串退化为 `unknown`，截断。
/// 空白先映射为 `_` 再整体剥离首尾 `_` / `.`，保证 `" x "` 收敛为 `x`、`"..x.."` 收敛为 `x`。
fn sanitize_source(source: Option<&str>) -> String {
    let raw = source.unwrap_or("unknown");
    let mut cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_whitespace()
                || matches!(
                    c,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '\u{0000}'..='\u{001f}'
                )
            {
                '_'
            } else {
                c
            }
        })
        .collect();
    while cleaned.starts_with('_') || cleaned.starts_with('.') {
        cleaned.remove(0);
    }
    while cleaned.ends_with('_') || cleaned.ends_with('.') {
        cleaned.pop();
    }
    if cleaned.is_empty() {
        "unknown".to_owned()
    } else {
        cleaned.chars().take(MAX_SOURCE_CHARS).collect()
    }
}

/// 旧分片布局的分片子目录名：取文件名去扩展名前 2 字符（旧 hash 恒为 hex）。
fn shard_dir(src: &str) -> &str {
    if src.len() >= 2 {
        &src[..2]
    } else {
        "00"
    }
}

/// 从文件名 `<prefix>.png` 取分片来源（旧 hash 布局即文件名主体）。
fn shard_key(file_name: &str) -> &str {
    file_name.split('.').next().unwrap_or(file_name)
}

fn blake3_hex(bytes: &[u8]) -> String {
    let mut hasher = Hasher::new();
    hasher.update(bytes);
    hasher.finalize().to_hex().to_string()
}

/// 写文件（自动建父目录）。需幂等写入时由调用方判断；这里 stdin 覆盖写入。
fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create image dir {parent:?}"))?;
    }
    std::fs::write(path, bytes)
        .with_context(|| format!("failed to write image {path:?}"))
        .map_err(AppError::from)
}

/// 删文件；文件不存在时静默成功（幂等），其余 IO 错误上抛。
fn remove_if_present(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(AppError::from(
            anyhow::Error::new(err).context(format!("failed to remove image {path:?}")),
        )),
    }
}

/// 尽力删除空目录：`remove_dir` 仅在目录为空时成功，非空 / 不存在都返回 `Err`，一律忽略。
fn remove_dir_if_empty(dir: Option<&Path>) {
    if let Some(dir) = dir {
        let _ = std::fs::remove_dir(dir);
    }
}

/// 把原图 PNG 字节解码 → 生成缩略图（最长边 <= [`THUMBNAIL_MAX`]，保持比例）→ 重新编码 PNG。
fn encode_thumbnail(png_bytes: &[u8]) -> Result<Vec<u8>> {
    let image = RustImageData::from_bytes(png_bytes).map_err(clip_err)?;
    let thumb = image
        .thumbnail(THUMBNAIL_MAX, THUMBNAIL_MAX)
        .map_err(clip_err)?;
    Ok(thumb.to_png().map_err(clip_err)?.get_bytes().to_vec())
}

fn clip_err<E: std::fmt::Display>(err: E) -> AppError {
    AppError::Clipboard(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// 用 image crate 生成一张纯色 PNG 作测试输入。
    fn sample_png(w: u32, h: u32) -> Vec<u8> {
        let buf = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(buf)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    fn temp_store() -> (TempDir, ImageStore) {
        let dir = TempDir::new();
        let store = ImageStore::for_test(dir.0.join("resources").join("clipboard-images"));
        (dir, store)
    }

    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("ecopaste-storage-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    fn payload(w: u32, h: u32) -> ImagePayload {
        ImagePayload {
            bytes: sample_png(w, h),
            width: w,
            height: h,
        }
    }

    #[test]
    fn stores_origin_under_temp_with_source_and_time_name() {
        let (_dir, store) = temp_store();
        let p = payload(64, 48);
        let stored = store.store(&p, Some("Chrome")).unwrap();

        assert!(
            stored.file_name.starts_with("Chrome_"),
            "got {}",
            stored.file_name
        );
        assert!(stored.file_name.ends_with(".png"));
        assert_eq!(stored.width, 64);
        assert_eq!(stored.height, 48);
        assert!(stored.size > 0);

        let origin = store.origin_path(&stored.file_name);
        assert!(origin.exists(), "origin should exist: {origin:?}");
        assert!(
            origin.starts_with(store.temp_dir()),
            "origin should live under temp: {origin:?}"
        );
        let thumb = store.thumbnail_path(&stored.file_name);
        assert!(!thumb.exists(), "thumbnail should be lazy: {thumb:?}");
        assert_eq!(std::fs::read(&origin).unwrap(), p.bytes);
    }

    #[test]
    fn store_without_source_falls_back_to_unknown() {
        let (_dir, store) = temp_store();
        let stored = store.store(&payload(10, 10), None).unwrap();
        assert!(
            stored.file_name.starts_with("unknown_"),
            "got {}",
            stored.file_name
        );
    }

    #[test]
    fn sanitize_source_removes_illegal_chars() {
        assert_eq!(sanitize_source(Some(" a/b:c ")), "a_b_c");
        assert_eq!(sanitize_source(Some("..weird..")), "weird");
        assert_eq!(sanitize_source(None), "unknown");
        assert_eq!(sanitize_source(Some("   ")), "unknown");
    }

    #[test]
    fn ensure_thumbnail_generates_under_temp() {
        let (_dir, store) = temp_store();
        let stored = store.store(&payload(64, 48), Some("Chrome")).unwrap();
        let thumb = store.ensure_thumbnail(&stored.file_name).unwrap();
        assert!(thumb.exists());
        assert!(thumb.starts_with(store.temp_dir()));
    }

    #[test]
    fn remove_clears_temp_files() {
        let (_dir, store) = temp_store();
        let stored = store.store(&payload(32, 32), Some("Chrome")).unwrap();
        store.ensure_thumbnail(&stored.file_name).unwrap();
        assert!(store.origin_path(&stored.file_name).exists());
        assert!(store.thumbnail_path(&stored.file_name).exists());

        store.remove(&stored.file_name).unwrap();
        assert!(!store.origin_path(&stored.file_name).exists());
        assert!(!store.thumbnail_path(&stored.file_name).exists());
    }
}
