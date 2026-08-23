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
//!
//! 透光度（`appearance.acrylic_opacity`）只影响系统层的 tint alpha；模糊度（`appearance.acrylic_blur`）
//! 由前端 `backdrop-filter` 控制——DWM Acrylic 的 blur radius 是系统固定参数，前端 CSS 变量透出。

use tauri::{AppHandle, Manager, Theme, WebviewWindow};

use crate::settings::{opacity_to_alpha, SettingsStore, WindowEffect};

use super::{CLIPBOARD_PREVIEW_WINDOW_LABEL, CLIPBOARD_WINDOW_LABEL};

/// 参与材质效果的窗口：均为无边框透明悬浮窗。
/// preference（带系统装饰的标准窗口）、onboarding（固定深色引导流程）、context-menu 系列
/// （Windows 原生菜单替代）以及 `copied` 复制成功小气泡（自带胶囊样式，不参与）不参与，
/// 避免装饰窗口与深色小菜单出现渲染怪异。
const EFFECT_WINDOW_LABELS: [&str; 2] = [CLIPBOARD_WINDOW_LABEL, CLIPBOARD_PREVIEW_WINDOW_LABEL];

/// 按当前设置给所有受支持的窗口应用 / 清除材质效果。
/// 单个窗口失败只记日志，不阻断其它窗口与设置流程。
pub fn apply_all(app: &AppHandle) {
    let snapshot = app.state::<SettingsStore>().snapshot();
    let effect = snapshot.appearance.window_effect;
    let opacity = snapshot.appearance.acrylic_opacity;

    for label in EFFECT_WINDOW_LABELS {
        let Some(window) = app.get_webview_window(label) else {
            continue;
        };

        apply(&window, effect, opacity);
    }
}

/// 窗口 ready 时按 label 定向应用；非效果窗口是 no-op。
/// 挂在生命周期 `on_ready`，覆盖「启动预创建 + 按需重建」两类窗口。
pub fn apply_for_label(app: &AppHandle, label: &str) {
    if !EFFECT_WINDOW_LABELS.contains(&label) {
        return;
    }

    let snapshot = app.state::<SettingsStore>().snapshot();
    let effect = snapshot.appearance.window_effect;
    let opacity = snapshot.appearance.acrylic_opacity;

    if let Some(window) = app.get_webview_window(label) {
        apply(&window, effect, opacity);
    }
}

/// 给单个窗口应用材质效果；`None` 时清除已有效果。
/// 明暗由原生窗口主题决定（`appearance.theme = auto` 时跟随系统实时值）。
pub fn apply(window: &WebviewWindow, effect: WindowEffect, opacity: u8) {
    // clear_vibrancy 与 apply_* 的错误类型不同（tauri::Error vs window_vibrancy::Error），
    // 故拆成两个分支分别记日志，而不强行统一类型。
    if effect == WindowEffect::None {
        if let Err(err) = window_vibrancy::clear_vibrancy(window) {
            log::warn!("clear window effect failed: {err}");
        }
        return;
    }

    let dark = window.theme().is_ok_and(|theme| theme == Theme::Dark);

    if let Err(err) = apply_effect(window, effect, dark, opacity) {
        // 老系统对 Acrylic 支持不佳会走到这里：效果保持未应用状态，属预期回落而非故障。
        log::warn!("apply window effect {effect:?} failed: {err}");
    }
}

#[cfg(target_os = "macos")]
fn apply_effect(
    window: &WebviewWindow,
    _effect: WindowEffect,
    dark: bool,
    _opacity: u8,
) -> Result<(), window_vibrancy::Error> {
    // HudWindow 是深色系统材质，Popover 是浅色弹出层材质；按主题挑选保证文字对比度。
    // macOS 的 NSVisualEffectMaterial 由系统决定外观与强度，用户的透光度设置仅影响前端 CSS。
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
    opacity: u8,
) -> Result<(), window_vibrancy::Error> {
    match effect {
        // 叠一层 tint，让亚克力上的正文保持可读。alpha 由「透光度（0-100）」反推：
        // 透光度 0 → alpha 255（完全不透明底色），透光度 100 → alpha 0（完全透明）。
        // 透光度 50 时 alpha ≈ 128，与 TieZ 默认观感保持一致。
        WindowEffect::Acrylic => {
            let alpha = opacity_to_alpha(opacity);
            let tint = if dark {
                (30, 30, 30, alpha)
            } else {
                (240, 240, 240, alpha)
            };
            window_vibrancy::apply_acrylic(window, Some(tint))
        }
        // None 在 apply() 里已被拦截，不会进入本函数。
        WindowEffect::None => Ok(()),
    }
}
