//! Значок в трее (StatusNotifierItem): меню, отметка непрочитанных, режимы «панель / трей».
//! На Linux значок не получает кликов — всё через меню (KDE открывает его и по левому клику).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Listener, Manager, Wry};

use crate::config::{ConfigStore, TrayMode};
use crate::store::{Scope, Store};

const TRAY_ID: &str = "main";
const ICON: &[u8] = include_bytes!("../icons/tray/normal.png");
const ICON_UNREAD: &[u8] = include_bytes!("../icons/tray/unread.png");

/// Пункт меню с числом непрочитанных: заголовок-подсказка (у SNI на Linux нет всплывающих подсказок).
#[derive(Default)]
pub struct TrayState {
    unread_item: Mutex<Option<MenuItem<Wry>>>,
    /// Последнее показанное число; `usize::MAX` — ещё не показывали.
    shown: AtomicUsize,
}

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    app.state::<TrayState>().shown.store(usize::MAX, Ordering::Relaxed);
    apply(app)?;
    let handle = app.clone();
    app.listen_any("mailbox-changed", move |_| update_unread(&handle));
    Ok(())
}

/// Приводит трей и панель задач в соответствие с настройками. Вызывать в главном потоке.
pub fn apply(app: &AppHandle) -> tauri::Result<()> {
    let mode = app.state::<ConfigStore>().get().tray_mode();
    let has_tray = app.tray_by_id(TRAY_ID).is_some();
    if mode.tray() && !has_tray {
        create(app)?;
    } else if !mode.tray() && has_tray {
        app.remove_tray_by_id(TRAY_ID);
        *app.state::<TrayState>().unread_item.lock().unwrap() = None;
        // Без трея скрытое окно было бы не вернуть.
        show_main(app);
    }
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
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Выйти", true, None::<&str>)?,
        ],
    )?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(ICON)?)
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
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    let state = app.state::<TrayState>();
    *state.unread_item.lock().unwrap() = Some(unread);
    state.shown.store(usize::MAX, Ordering::Relaxed);
    update_unread(app);
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
    let state = app.state::<TrayState>();
    if state.shown.swap(count, Ordering::Relaxed) == count {
        return;
    }
    let icon = if count > 0 { ICON_UNREAD } else { ICON };
    if let Ok(image) = Image::from_bytes(icon) {
        let _ = tray.set_icon(Some(image));
    }
    let item = state.unread_item.lock().unwrap().clone();
    if let Some(item) = item {
        let _ = item.set_text(if count > 0 { format!("Непрочитанных: {count}") } else { "Нет непрочитанных".into() });
    }
}
