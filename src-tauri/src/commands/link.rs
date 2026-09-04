use anyhow::Context;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use crate::core::{AppError, Result};

/// 打开经过白名单校验的外部网页 URL。
#[tauri::command]
pub async fn open_external_url(app: AppHandle, url: String) -> Result<()> {
    let normalized = url.trim();
    if !(normalized.starts_with("https://") || normalized.starts_with("http://")) {
        let lang = crate::i18n::current_language(&app);
        let message =
            crate::i18n::commands::label(lang, crate::i18n::commands::Key::ExternalUrlUnsupported);

        return Err(AppError::Other(anyhow::anyhow!(message)));
    }

    app.opener()
        .open_url(normalized, None::<&str>)
        .context("failed to open external url")?;

    Ok(())
}

/// 打开一个已提取的实体：链接按白名单打开，邮箱转 `mailto:` 唤起邮件客户端。
/// 由前端实体下拉框「打开」按钮触发；键入走 `fill_selected_text`。
#[tauri::command]
pub async fn open_entity_link(app: AppHandle, kind: String, value: String) -> Result<()> {
    let opened = match kind.as_str() {
        "email" => {
            let email = value.trim();
            if email.is_empty() {
                false
            } else {
                app.opener()
                    .open_url(format!("mailto:{email}"), None::<&str>)
                    .context("failed to open email client")?;
                true
            }
        }
        "url" => {
            let mut url = value.trim().to_owned();
            if url.starts_with("www.") {
                url = format!("https://{url}");
            }
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                let lang = crate::i18n::current_language(&app);
                let message = crate::i18n::commands::label(
                    lang,
                    crate::i18n::commands::Key::ExternalUrlUnsupported,
                );
                return Err(AppError::Other(anyhow::anyhow!(message)));
            }
            app.opener()
                .open_url(url, None::<&str>)
                .context("failed to open url")?;
            true
        }
        _ => false,
    };

    if !opened {
        return Err(AppError::Other(anyhow::anyhow!(
            "unknown entity kind: {kind}"
        )));
    }
    Ok(())
}
