//! Светлая или тёмная схема системы — для режима «Как в системе».
//!
//! Источник — xdg-desktop-portal (`org.freedesktop.appearance color-scheme`): KDE публикует там
//! выбранную схему и шлёт сигнал SettingChanged, когда её меняют. Без портала — по цвету окон
//! в kdeglobals.

use std::sync::atomic::{AtomicBool, Ordering};

use gtk::gio;
use gtk::glib::prelude::*;
use tauri::{AppHandle, Emitter, Manager};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const NAMESPACE: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";

static SYSTEM_DARK: AtomicBool = AtomicBool::new(true);

pub fn system_dark() -> bool {
    SYSTEM_DARK.load(Ordering::Relaxed)
}

/// 1 — тёмная, 2 — светлая, 0 — «нет предпочтения».
fn from_portal(value: u32) -> Option<bool> {
    match value {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

/// Значение может прийти как `u` или завёрнутым в `v` (старый метод Read) — разворачиваем.
fn as_u32(mut value: gtk::glib::Variant) -> Option<u32> {
    while let Some(inner) = value.as_variant() {
        value = inner;
    }
    value.get::<u32>()
}

/// Вызывать из главного потока: подписка на сигнал работает в главном цикле GLib.
pub fn init(app: &AppHandle) {
    let bus = match gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>) {
        Ok(bus) => bus,
        Err(e) => {
            eprintln!("mMail: D-Bus недоступен ({e}), схема — по kdeglobals");
            SYSTEM_DARK.store(crate::tray::dark_panel(), Ordering::Relaxed);
            return;
        }
    };
    let read = bus
        .call_sync(Some(PORTAL), PATH, SETTINGS, "ReadOne", Some(&(NAMESPACE, KEY).to_variant()), None, gio::DBusCallFlags::NONE, 2000, None::<&gio::Cancellable>)
        .or_else(|_| bus.call_sync(Some(PORTAL), PATH, SETTINGS, "Read", Some(&(NAMESPACE, KEY).to_variant()), None, gio::DBusCallFlags::NONE, 2000, None::<&gio::Cancellable>));
    let dark = read.ok().and_then(|r| as_u32(r.child_value(0))).and_then(from_portal).unwrap_or_else(crate::tray::dark_panel);
    SYSTEM_DARK.store(dark, Ordering::Relaxed);

    let app = app.clone();
    #[allow(deprecated)] // signal_subscribe: замена (subscribe_to_signal) появилась только в gio 0.19
    bus.signal_subscribe(Some(PORTAL), Some(SETTINGS), Some("SettingChanged"), Some(PATH), Some(NAMESPACE), gio::DBusSignalFlags::NONE, move |_, _, _, _, _, params| {
        if params.child_value(1).get::<String>().as_deref() != Some(KEY) {
            return;
        }
        let dark = as_u32(params.child_value(2)).and_then(from_portal).unwrap_or_else(crate::tray::dark_panel);
        if SYSTEM_DARK.swap(dark, Ordering::Relaxed) != dark {
            let config = app.state::<crate::config::ConfigStore>();
            let _ = app.emit("appearance-changed", crate::commands::get_appearance(config));
        }
    });
    // Подписка живёт, пока жив D-Bus-клиент; держим его до конца работы приложения.
    std::mem::forget(bus);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_values() {
        assert_eq!(from_portal(1), Some(true));
        assert_eq!(from_portal(2), Some(false));
        assert_eq!(from_portal(0), None);
        let wrapped = gtk::glib::Variant::from_variant(&gtk::glib::Variant::from_variant(&2u32.to_variant()));
        assert_eq!(as_u32(wrapped), Some(2));
    }
}
