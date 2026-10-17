//! Значок в трее (StatusNotifierItem): меню, отметка непрочитанных, режимы «панель / трей».
//! На Linux значок не получает кликов — всё через меню (KDE открывает его и по левому клику).

use std::sync::Mutex;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Listener, Manager, Wry};

use crate::config::{ConfigStore, TrayMode};
use crate::store::{Scope, Store};

const TRAY_ID: &str = "main";
/// Значки: (обычный, с меткой непрочитанных).
const COLOR: (&[u8], &[u8]) = (include_bytes!("../icons/tray/normal.png"), include_bytes!("../icons/tray/unread.png"));
const MONO_LIGHT: (&[u8], &[u8]) =
    (include_bytes!("../icons/tray/mono-light.png"), include_bytes!("../icons/tray/mono-light-unread.png"));
const MONO_DARK: (&[u8], &[u8]) =
    (include_bytes!("../icons/tray/mono-dark.png"), include_bytes!("../icons/tray/mono-dark-unread.png"));

/// Пункт меню с числом непрочитанных: заголовок-подсказка (у SNI на Linux нет всплывающих подсказок).
#[derive(Default)]
pub struct TrayState {
    unread_item: Mutex<Option<MenuItem<Wry>>>,
    /// Что сейчас показано: число непрочитанных и картинка; `None` — ещё ничего.
    shown: Mutex<Option<(usize, usize)>>,
}

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    apply(app)?;
    let handle = app.clone();
    app.listen_any("mailbox-changed", move |_| update_unread(&handle));
    Ok(())
}

/// Приводит трей и панель задач в соответствие с настройками. Вызывать в главном потоке.
pub fn apply(app: &AppHandle) -> tauri::Result<()> {
    let mode = app.state::<ConfigStore>().get().tray_mode();
    // Значок создаётся один раз и дальше только прячется: удалённый appindicator не освобождает
    // свой путь в D-Bus, и пересозданный значок оставался бы мёртвым, без меню.
    match app.tray_by_id(TRAY_ID) {
        Some(tray) => tray.set_visible(mode.tray())?,
        None if mode.tray() => create(app)?,
        None => {}
    }
    let hidden = app.get_webview_window("main").is_some_and(|w| w.is_visible().is_ok_and(|v| !v));
    if !mode.tray() && hidden {
        // Без трея скрытое окно было бы не вернуть.
        show_main(app);
    }
    update_unread(app);
    if let Some(window) = app.get_webview_window("main") {
        crate::blur::set_skip_taskbar(&window, mode == TrayMode::Tray);
    }
    Ok(())
}

/// Крестик главного окна прячет его в трей, а не закрывает приложение.
pub fn hides_on_close(app: &AppHandle) -> bool {
    let config = app.state::<ConfigStore>().get();
    config.tray_mode().tray() && config.close_to_tray.unwrap_or(true) && app.tray_by_id(TRAY_ID).is_some()
}

fn create(app: &AppHandle) -> tauri::Result<()> {
    let unread = MenuItem::with_id(app, "unread", "Нет непрочитанных", false, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &unread,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "show", "Открыть mMail", true, None::<&str>)?,
            &MenuItem::with_id(app, "compose", "Написать письмо", true, None::<&str>)?,
            &MenuItem::with_id(app, "settings", "Настройки", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Выйти", true, None::<&str>)?,
        ],
    )?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(COLOR.0)?)
        .title("mMail")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main(app),
            "compose" => {
                let state = app.state::<crate::compose::ComposeState>();
                if let Err(e) = crate::compose::open_compose(app.clone(), state, Default::default()) {
                    eprintln!("mMail: окно письма не открылось: {e}");
                }
            }
            "settings" => {
                if let Err(e) = crate::commands::open_settings(app.clone()) {
                    eprintln!("mMail: окно настроек не открылось: {e}");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    *app.state::<TrayState>().unread_item.lock().unwrap() = Some(unread);
    Ok(())
}

pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Значок с меткой и пункт меню, если число непрочитанных во «Входящих» изменилось.
fn update_unread(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let count = app.state::<Store>().unread(Scope::All, "inbox");
    let set = match app.state::<ConfigStore>().get().tray_icon.as_deref() {
        Some("light") => MONO_LIGHT,
        Some("dark") => MONO_DARK,
        Some("auto") if dark_panel() => MONO_LIGHT,
        Some("auto") => MONO_DARK,
        _ => COLOR,
    };
    let icon = if count > 0 { set.1 } else { set.0 };
    let state = app.state::<TrayState>();
    let key = (count, icon.as_ptr() as usize);
    if state.shown.lock().unwrap().replace(key) == Some(key) {
        return;
    }
    if let Ok(image) = Image::from_bytes(icon) {
        let _ = tray.set_icon(Some(image));
    }
    let item = state.unread_item.lock().unwrap().clone();
    if let Some(item) = item {
        let _ = item.set_text(if count > 0 { format!("Непрочитанных: {count}") } else { "Нет непрочитанных".into() });
    }
}

/// Тёмная ли тема KDE: по цвету фона окон из `~/.config/kdeglobals` (панель Plasma следует ему же).
fn dark_panel() -> bool {
    let Some(path) = dirs_config().map(|d| d.join("kdeglobals")) else { return true };
    let Ok(text) = std::fs::read_to_string(path) else { return true };
    let mut in_window = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_window = line == "[Colors:Window]";
        } else if let Some(rgb) = line.strip_prefix("BackgroundNormal=").filter(|_| in_window) {
            let c: Vec<f32> = rgb.split(',').filter_map(|v| v.trim().parse().ok()).collect();
            if let [r, g, b, ..] = c[..] {
                return 0.2126 * r + 0.7152 * g + 0.0722 * b < 128.0;
            }
        }
    }
    true
}

fn dirs_config() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
}
