use anyhow::Context;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::ConnectOptions;
use sqlx::SqlitePool;
use tauri::AppHandle;

use crate::core::Result;
use crate::db::db_path;

/// 升版时若某个已被应用过的迁移文件被原地编辑过（例如换了 BOM/换行/注释），
/// sqlx 会因“已应用但被修改”校验失败而拒绝启动、把现有用户数据库卡住。
/// 这里在校验前把“已成功应用但校验和不一致”的记录改成当前嵌入文件的新校验和，
/// 从而自愈一次——这要求迁移语义未变、落库 schema 已一致，仅文件字节变化。
async fn heal_stale_checksums(pool: &SqlitePool, migrator: &sqlx::migrate::Migrator) -> Result<()> {
    let table_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations')",
    )
    .fetch_one(pool)
    .await
    .context("check _sqlx_migrations table")?;

    if !table_exists {
        return Ok(());
    }

    for migration in migrator.iter() {
        let version = migration.version;
        let stored: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT checksum FROM _sqlx_migrations WHERE version = ? AND success = 1",
        )
        .bind(version)
        .fetch_optional(pool)
        .await
        .context("query migration checksum")?;

        if let Some(stored) = stored {
            let expected = migration.checksum.as_ref();
            if stored != expected {
                sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?")
                    .bind(expected)
                    .bind(version)
                    .execute(pool)
                    .await
                    .context("rewrite stale migration checksum")?;
                log::warn!("self-healed stale checksum for applied migration {version}");
            }
        }
    }

    Ok(())
}

pub async fn init(app: &AppHandle) -> Result<SqlitePool> {
    let path = db_path(app)?;

    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .disable_statement_logging();

    let pool = SqlitePoolOptions::new()
        .connect_with(options)
        .await
        .with_context(|| format!("failed to open sqlite database at {path:?}"))?;

    let migrator = sqlx::migrate!("./migrations");
    heal_stale_checksums(&pool, &migrator).await?;
    migrator
        .run(&pool)
        .await
        .context("failed to run sqlite migrations")?;

    log::info!("sqlite pool ready at {path:?}");
    Ok(pool)
}
