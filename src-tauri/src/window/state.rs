use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, RwLock};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize};

use crate::core::Result;

const STATE_FILENAME: &str = "window-state.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowState {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

pub struct WindowStateStore {
    path: RwLock<PathBuf>,
    states: Mutex<HashMap<String, WindowState>>,
}

impl WindowStateStore {
    pub fn new(app: &AppHandle) -> Result<Self> {
        let dir = crate::core::paths::state_dir(app)?;

        fs::create_dir_all(&dir).with_context(|| format!("failed to create dir at {dir:?}"))?;

        let path = dir.join(STATE_FILENAME);

        let states = if path.exists() {
            match fs::read_to_string(&path) {
                Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
                    log::warn!("failed to parse window state at {path:?}, using defaults: {e}");
                    HashMap::new()
                }),
                Err(e) => {
                    log::warn!("failed to read window state at {path:?}, using defaults: {e}");
                    HashMap::new()
                }
            }
        } else {
            HashMap::new()
        };

        log::info!("window state store ready at {path:?}");
        Ok(Self {
            path: RwLock::new(path),
            states: Mutex::new(states),
        })
    }

    pub fn save(&self, label: &str, state: WindowState) -> Result<()> {
        let mut states = self.states.lock().unwrap_or_else(|poisoned| {
            log::error!("window state mutex poisoned on save, recovering");
            poisoned.into_inner()
        });
        states.insert(label.to_owned(), state);
        let json =
            serde_json::to_string_pretty(&*states).context("failed to serialize window states")?;
        let path = self.path();
        fs::write(&path, json)
            .with_context(|| format!("failed to write window state to {:?}", path))?;
        Ok(())
    }

    pub fn get(&self, label: &str) -> Option<WindowState> {
        let states = self.states.lock().unwrap_or_else(|poisoned| {
            log::error!("window state mutex poisoned on get, recovering");
            poisoned.into_inner()
        });
        states.get(label).cloned()
    }

    /// 数据目录热切换后重新绑定窗口状态文件，并重新读取新目录里的状态。
    pub fn rebase(&self, app: &AppHandle) -> Result<()> {
        let dir = crate::core::paths::state_dir(app)?;
        fs::create_dir_all(&dir).with_context(|| format!("failed to create dir at {dir:?}"))?;
        let path = dir.join(STATE_FILENAME);
        let next_states = load_states(&path);

        *self.path.write().expect("window state path poisoned") = path;
        *self.states.lock().unwrap_or_else(|poisoned| {
            log::error!("window state mutex poisoned on rebase, recovering");
            poisoned.into_inner()
        }) = next_states;
        Ok(())
    }

    fn path(&self) -> PathBuf {
        self.path
            .read()
            .expect("window state path poisoned")
            .clone()
    }
}

fn load_states(path: &PathBuf) -> HashMap<String, WindowState> {
    if !path.exists() {
        return HashMap::new();
    }

    match fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
            log::warn!("failed to parse window state at {path:?}, using defaults: {e}");
            HashMap::new()
        }),
        Err(e) => {
            log::warn!("failed to read window state at {path:?}, using defaults: {e}");
            HashMap::new()
        }
    }
}

/// 读取窗口当前的实时几何（`outer_position` + `inner_size`）并落盘。
/// 在隐藏 / 关闭 / 退出等可靠生命周期点调用即可捕获用户的移动与缩放。
pub fn save_window_state(app: &AppHandle, label: &str) -> Result<()> {
    let window = app
        .get_webview_window(label)
        .ok_or_else(|| anyhow::anyhow!("window not found: {label}"))?;

    let pos = window.outer_position().map_err(|e| anyhow::anyhow!(e))?;
    let size = window.inner_size().map_err(|e| anyhow::anyhow!(e))?;

    let store = app.state::<WindowStateStore>();
    store.save(
        label,
        WindowState {
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
        },
    )
}

/// 恢复窗口的尺寸 + 位置。无存档返回 `Ok(false)`。
///
/// 保留用户保存的尺寸，但会**钳制到目标屏幕内**：若存档尺寸超过目标显示器
/// （分辨率 / 缩放变化后可能出现），把宽高压到屏幕边界内而不是跳过，避免窗口
/// 超出屏幕导致显示不完整；目标屏取「存档位置所在显示器」，被拔出则退到光标所在屏。
pub fn restore_window_state(app: &AppHandle, label: &str) -> Result<bool> {
    let store = app.state::<WindowStateStore>();
    let Some(state) = store.get(label) else {
        return Ok(false);
    };

    let window = app
        .get_webview_window(label)
        .ok_or_else(|| anyhow::anyhow!("window not found: {label}"))?;

    let monitors = window
        .available_monitors()
        .map_err(|e| anyhow::anyhow!(e))?;

    // 目标屏：优先「存档左上角所在显示器」；被拔出则退到光标所在屏；再退到首屏。
    let target = monitors
        .iter()
        .find(|m| {
            let p = m.position();
            let size = m.size();
            state.x >= p.x
                && state.x < p.x + size.width as i32
                && state.y >= p.y
                && state.y < p.y + size.height as i32
        })
        .cloned()
        .or_else(|| super::position::cursor_monitor(&window))
        .or_else(|| monitors.first().cloned());

    let Some(target) = target else {
        // 无任何可用显示器（罕见）：仍恢复既有尺寸，位置保持默认。
        window
            .set_size(PhysicalSize::new(state.width, state.height))
            .map_err(|e| anyhow::anyhow!(e))?;
        return Ok(true);
    };

    let mon_pos = *target.position();
    let mon_size = *target.size();

    // 保留用户尺寸，但不超过目标屏：分辨率变化后旧存档可能大于新屏幕，
    // 这里压到屏幕边界内，而不是把窗口整体作废缩回默认。
    let width = state.width.min(mon_size.width);
    let height = state.height.min(mon_size.height);
    window
        .set_size(PhysicalSize::new(width, height))
        .map_err(|e| anyhow::anyhow!(e))?;

    // 把窗口左上角夹回目标屏内，避免窗口主干落到不可见区域。
    let max_x = (mon_pos.x + mon_size.width as i32 - width as i32).max(mon_pos.x);
    let max_y = (mon_pos.y + mon_size.height as i32 - height as i32).max(mon_pos.y);
    let x = state.x.clamp(mon_pos.x, max_x);
    let y = state.y.clamp(mon_pos.y, max_y);
    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|e| anyhow::anyhow!(e))?;

    Ok(true)
}
