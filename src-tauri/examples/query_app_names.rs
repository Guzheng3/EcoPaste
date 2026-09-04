use std::path::PathBuf;

use sqlx::sqlite::SqlitePoolOptions;

#[tokio::main]
async fn main() {
    let db_path = PathBuf::from(
        r"C:\Users\Administrator\AppData\Local\com.ayangweb.eco-paste\prod\db\clipboard.db",
    );
    let url = format!("sqlite:{}?mode=ro", db_path.display());
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("failed to open db");

    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT name FROM clipboard_apps ORDER BY LENGTH(name) DESC LIMIT 10")
            .fetch_all(&pool)
            .await
            .expect("failed to query");

    for (name,) in &rows {
        println!("{} | len={}", name, name.len());
    }
}
