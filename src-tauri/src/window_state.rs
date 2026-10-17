//! Размер (и на X11 — положение) окон между запусками: главное окно и окно письма.
//! На Wayland клиент не может выбрать своё положение на экране — это решает KWin.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewWindow, WebviewWindowBuilder, WindowEvent, Wry};

use crate::config::ConfigStore;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Geometry {
    /// Логические пиксели (не зависят от масштаба экрана).
    pub width: f64,
    pub height: f64,
    /// Только X11: на Wayland положение не сохраняется.
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub maximized: bool,
}

fn wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// Сохранённый размер окна, не меньше минимального.
pub fn restore<'a>(
    app: &AppHandle,
    kind: &str,
    builder: WebviewWindowBuilder<'a, Wry, AppHandle>,
    min: (f64, f64),
) -> WebviewWindowBuilder<'a, Wry, AppHandle> {
    let Some(g) = app.state::<ConfigStore>().get().windows.get(kind).copied() else { return builder };
    if !(g.width.is_finite() && g.height.is_finite()) || g.width <= 0.0 || g.height <= 0.0 {
        return builder;
    }
    let mut builder = builder.inner_size(g.width.max(min.0), g.height.max(min.1));
    if let (Some(x), Some(y), false) = (g.x, g.y, wayland()) {
        builder = builder.position(x, y);
    }
    builder.maximized(g.maximized)
}

/// Следит за размером окна и сохраняет его в настройки (не чаще раза в полсекунды).
pub fn track(app: &AppHandle, window: &WebviewWindow, kind: &'static str) {
    // Счётчик на окно: сохраняем только после последнего события серии (перетаскивание рамки).
    let generation_counter = Arc::new(AtomicU64::new(0));
    let (app, handle) = (app.clone(), window.clone());
    window.on_window_event(move |event| {
        if !matches!(event, WindowEvent::Resized(_) | WindowEvent::Moved(_)) {
            return;
        }
        let generation = generation_counter.fetch_add(1, Ordering::Relaxed) + 1;
        let (app, window, counter) = (app.clone(), handle.clone(), generation_counter.clone());
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if counter.load(Ordering::Relaxed) == generation {
                save(&app, &window, kind);
            }
        });
    });
}

fn save(app: &AppHandle, window: &WebviewWindow, kind: &str) {
    let Ok(scale) = window.scale_factor() else { return };
    let maximized = window.is_maximized().unwrap_or(false);
    let config = app.state::<ConfigStore>();
    let previous = config.get().windows.get(kind).copied().unwrap_or_default();
    // У развёрнутого окна запоминаем только сам факт: размер — тот, что был до разворота.
    let mut g = Geometry { maximized, ..previous };
    if !maximized {
        let Ok(size) = window.inner_size() else { return };
        g.width = (f64::from(size.width) / scale).round();
        g.height = (f64::from(size.height) / scale).round();
        if !wayland() {
            if let Ok(pos) = window.outer_position() {
                g.x = Some(f64::from(pos.x) / scale);
                g.y = Some(f64::from(pos.y) / scale);
            }
        }
    }
    if g != previous {
        if let Err(e) = config.update(|c| {
            c.windows.insert(kind.to_string(), g);
        }) {
            eprintln!("mMail: размер окна не сохранён: {e}");
        }
    }
}
