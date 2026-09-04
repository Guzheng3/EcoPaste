//! 历史清理后台任务：按 `clipboard.history.retention` + `maxCount` 定期裁剪。
//!
//! 调度为固定时间点：启动（开机）立即执行一次，之后每天跨过午夜 00:00 后再执行一次。
//! 每次执行都从 `SettingsStore` 取最新配置，用户在偏好里调时长 / 上限后不必重启即可生效。
//! 置顶与收藏项一律保留（由 [`cleanup_history`] 保证）。
//! 敏感条目 TTL 在每个调度心跳里单独检查（见 [`run_sensitive_cleanup`]）。
//! 图片没有独立生命周期：跟随数据库记录，记录删除时由调用方连带删盘上文件。

use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, Local, Utc};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use super::storage::ImageStore;
use super::watcher::CLIPBOARD_UPDATED_EVENT;
use crate::db::items::{cleanup_history, cleanup_sensitive_expired};
use crate::settings::{Retention, RetentionUnit, SettingsStore};

/// 调度器检查设置与到期状态的频率；敏感条目 TTL 可短至 1 小时，需每个心跳都检查。
const SCHEDULER_TICK_INTERVAL: Duration = Duration::from_secs(60);

/// 启动后台清理任务：开机（启动）立即清理一次，之后每个调度心跳做敏感条目过期清理，
/// 并仅当跨入新的一天（每晚午夜后第一个心跳）时执行历史容量清理。
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // 开机首次运行：立即清理一次（历史 + 敏感）。
        run_once(&app).await;
        run_sensitive_cleanup(&app).await;

        // 开机已跑过，记录当天，避免同一天内重复；跨天后才触发下一次。
        let mut last_cleanup_date = Local::now().date_naive();

        let mut ticker = tokio::time::interval(SCHEDULER_TICK_INTERVAL);
        ticker.tick().await;

        loop {
            ticker.tick().await;

            // 敏感条目 TTL 可短至 1 小时，需在每个心跳都检查到期（DELETE 有索引，代价低）。
            run_sensitive_cleanup(&app).await;

            let today = Local::now().date_naive();
            if today > last_cleanup_date {
                // 跨入新的一天（每晚 00:00 后第一个心跳），执行一次历史清理。
                run_once(&app).await;
                last_cleanup_date = today;
            }
        }
    });
}

async fn run_once(app: &AppHandle) {
    let history = match app.try_state::<SettingsStore>() {
        Some(store) => store.snapshot().clipboard.history,
        None => return,
    };

    let cutoff = retention_cutoff(&history.retention, Utc::now());
    let max = (history.max_count > 0).then_some(history.max_count);

    if cutoff.is_none() && max.is_none() {
        return;
    }

    let pool = app.state::<crate::db::DatabaseState>().pool().await;
    match cleanup_history(&pool, cutoff, max).await {
        Ok(outcome) if outcome.removed == 0 => {}
        Ok(outcome) => {
            remove_images(app, &outcome.image_files);
            log::info!("history cleanup removed {} item(s)", outcome.removed);
            if let Err(err) = app.emit(
                CLIPBOARD_UPDATED_EVENT,
                json!({ "cleanup": outcome.removed }),
            ) {
                log::warn!("emit cleanup event failed: {err}");
            }
        }
        Err(err) => log::warn!("history cleanup failed: {err}"),
    }
}

/// 删除 TTL 已过期的敏感条目（收藏项豁免，见 [`cleanup_sensitive_expired`]）。
/// 失败只记日志；被删图片文件的落盘清理复用 [`remove_images`]。
async fn run_sensitive_cleanup(app: &AppHandle) {
    let pool = app.state::<crate::db::DatabaseState>().pool().await;
    let now = Utc::now();
    match cleanup_sensitive_expired(&pool, now).await {
        Ok(outcome) if outcome.removed == 0 => {}
        Ok(outcome) => {
            if !outcome.image_files.is_empty() {
                remove_images(app, &outcome.image_files);
            }
            log::info!("sensitive cleanup removed {} item(s)", outcome.removed);
            if let Err(err) = app.emit(
                CLIPBOARD_UPDATED_EVENT,
                json!({ "cleanup": outcome.removed }),
            ) {
                log::warn!("emit sensitive cleanup event failed: {err}");
            }
        }
        Err(err) => log::warn!("sensitive cleanup failed: {err}"),
    }
}

/// 删除被清理图片记录的落盘文件（原图 + 缩略图）。`ImageStore` 未注册或单个文件删除失败
/// 都只记日志、不阻断——清理本身已成功，残留文件最坏只是占用磁盘，不影响功能。
fn remove_images(app: &AppHandle, file_names: &[String]) {
    if file_names.is_empty() {
        return;
    }
    let Some(store) = app.try_state::<ImageStore>() else {
        log::warn!(
            "image store unavailable; skip removing {} image file(s)",
            file_names.len()
        );
        return;
    };
    for file_name in file_names {
        if let Err(err) = store.remove(file_name) {
            log::warn!("remove cleaned image {file_name} failed: {err}");
        }
    }
}

/// `Retention` → 绝对截止时间。`Forever` 或 `value == 0` 表示禁用。
/// 月份近似按 30 天处理（与前端展示口径一致，不引日历库）。
fn retention_cutoff(r: &Retention, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    if r.value == 0 {
        return None;
    }
    let dur = match r.unit {
        RetentionUnit::Forever => return None,
        RetentionUnit::Hours => ChronoDuration::hours(r.value as i64),
        RetentionUnit::Days => ChronoDuration::days(r.value as i64),
        RetentionUnit::Weeks => ChronoDuration::weeks(r.value as i64),
        RetentionUnit::Months => ChronoDuration::days((r.value as i64) * 30),
    };
    Some(now - dur)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    #[test]
    fn retention_cutoff_returns_none_when_disabled() {
        assert!(retention_cutoff(
            &Retention {
                value: 0,
                unit: RetentionUnit::Days
            },
            now()
        )
        .is_none());
        assert!(retention_cutoff(
            &Retention {
                value: 7,
                unit: RetentionUnit::Forever
            },
            now()
        )
        .is_none());
    }

    #[test]
    fn retention_cutoff_subtracts_by_unit() {
        let n = now();
        assert_eq!(
            retention_cutoff(
                &Retention {
                    value: 2,
                    unit: RetentionUnit::Hours
                },
                n
            ),
            Some(n - ChronoDuration::hours(2))
        );
        assert_eq!(
            retention_cutoff(
                &Retention {
                    value: 3,
                    unit: RetentionUnit::Days
                },
                n
            ),
            Some(n - ChronoDuration::days(3))
        );
        assert_eq!(
            retention_cutoff(
                &Retention {
                    value: 1,
                    unit: RetentionUnit::Weeks
                },
                n
            ),
            Some(n - ChronoDuration::weeks(1))
        );
        assert_eq!(
            retention_cutoff(
                &Retention {
                    value: 1,
                    unit: RetentionUnit::Months
                },
                n
            ),
            Some(n - ChronoDuration::days(30))
        );
    }
}
