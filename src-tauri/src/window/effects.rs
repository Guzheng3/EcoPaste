//! 窗口材质效果（毛玻璃）。
//!
//! 思路照搬 TieZ（tiez-clipboard）：用 `window-vibrancy` 把系统材质挂到无边框透明窗口上
//! （Windows = DWM Acrylic，macOS = NSVisualEffectView），前端再用半透明 + backdrop-filter
//! 的层叠（见 `src/styles/global.scss` 的 `vibrancy-acrylic`）完整复刻毛玻璃观感。两层配合：
//! Rust 侧负责真实桌面模糊，前端负责窗口内玻璃面与炫光。
//!
//! 平台边界：
//! - Windows：Acrylic 走 `SetWindowCompositionAttribute`，Win10 1803+ 与 Win11 均可用。
//! - macOS：Acrylic 映射为 NSVisualEffectView 毛玻璃，按明暗主题选材质。

use tauri::{AppHandle, Manager, Theme, WebviewWindow};

use crate::settings::{SettingsStore, WindowEffect};

use super::{CLIPBOARD_PREVIEW_WINDOW_LABEL, CLIPBOARD_WINDOW_LABEL};

/// 参与材质效果的窗口：均为无边框透明悬浮窗。
/// preference（带系统装饰的标准窗口）、onboarding（固定深色引导流程）与 context-menu 系列
/// （Windows 原生菜单替代）不参与，避免装饰窗口与深色小菜单出现渲染怪异。
const EFFECT_WINDOW_LABELS: [&str; 2] = [CLIPBOARD_WINDOW_LABEL, CLIPBOARD_PREVIEW_WINDOW_LABEL];

/// 按当前设置给所有受支持的窗口应用 / 清除材质效果。
/// 单个窗口失败只记日志，不阻断其它窗口与设置流程。
pub fn apply_all(app: &AppHandle) {
    let effect = app
        .state::<SettingsStore>()
        .snapshot()
        .appearance
        .window_effect;

    for label in EFFECT_WINDOW_LABELS {
        let Some(window) = app.get_webview_window(label) else {
            continue;
        };

        apply(&window, effect);
    }
}

/// 窗口 ready 时按 label 定向应用；非效果窗口是 no-op。
/// 挂在生命周期 `on_ready`，覆盖「启动预创建 + 按需重建」两类窗口。
pub fn apply_for_label(app: &AppHandle, label: &str) {
    if !EFFECT_WINDOW_LABELS.contains(&label) {
        return;
    }

    let effect = app
        .state::<SettingsStore>()
        .snapshot()
        .appearance
        .window_effect;

    if let Some(window) = app.get_webview_window(label) {
        apply(&window, effect);
    }
}

/// 给单个窗口应用材质效果；`None` 时清除已有效果。
/// 明暗由原生窗口主题决定（`appearance.theme = auto` 时跟随系统实时值）。
pub fn apply(window: &WebviewWindow, effect: WindowEffect) {
    // clear_vibrancy 与 apply_* 的错误类型不同（tauri::Error vs window_vibrancy::Error），
    // 故拆成两个分支分别记日志，而不强行统一类型。
    if effect == WindowEffect::None {
        if let Err(err) = window_vibrancy::clear_vibrancy(window) {
            log::warn!("clear window effect failed: {err}");
        }
        return;
    }

    let dark = window.theme().is_ok_and(|theme| theme == Theme::Dark);

    if let Err(err) = apply_effect(window, effect, dark) {
        // 老系统对 Acrylic 支持不佳会走到这里：效果保持未应用状态，属预期回落而非故障。
        log::warn!("apply window effect {effect:?} failed: {err}");
    }
}

#[cfg(target_os = "macos")]
fn apply_effect(
    window: &WebviewWindow,
    _effect: WindowEffect,
    dark: bool,
) -> Result<(), window_vibrancy::Error> {
    // HudWindow 是深色系统材质，Popover 是浅色弹出层材质；按主题挑选保证文字对比度。
    let material = if dark {
        window_vibrancy::NSVisualEffectMaterial::HudWindow
    } else {
        window_vibrancy::NSVisualEffectMaterial::Popover
    };

    window_vibrancy::apply_vibrancy(window, material, None, None)
}

#[cfg(target_os = "windows")]
fn apply_effect(
    window: &WebviewWindow,
    effect: WindowEffect,
    dark: bool,
) -> Result<(), window_vibrancy::Error> {
    match effect {
        // 叠一层近实底色的 tint，让亚克力上的正文保持可读；数值照搬 TieZ。
        WindowEffect::Acrylic => window_vibrancy::apply_acrylic(
            window,
            Some(if dark {
                (30, 30, 30, 40)
            } else {
                (240, 240, 240, 40)
            }),
        ),
        // None 在 apply() 里已被拦截，不会进入本函数。
        WindowEffect::None => Ok(()),
    }
}
