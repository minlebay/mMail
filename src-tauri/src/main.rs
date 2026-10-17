mod accounts;
mod attachments;
mod blur;
mod commands;
mod compose;
mod config;
mod context_menu;
mod db;
mod fake;
mod folders;
mod imap;
mod i18n;
mod links;
mod remote;
mod sanitize;
mod smtp;
mod store;
mod tray;
mod translation;

use tauri::webview::NewWindowResponse;
use tauri::window::Color;
use tauri::{AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// Должен совпадать с border-radius у `.window` в App.svelte.
const CORNER_RADIUS: i32 = 12;

fn main() {
    tauri::Builder::default()
        .manage(store::Store::default())
        .manage(imap::Imap::default())
        .manage(links::PendingLink::default())
        .manage(compose::ComposeState::default())
        .manage(tray::TrayState::default())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::list_folders,
            commands::list_messages,
            commands::get_message,
            commands::set_sender_trusted,
            commands::get_appearance,
            commands::set_appearance,
            commands::set_zoom,
            commands::save_attachment,
            commands::storage_info,
            commands::set_body_days,
            commands::clear_cache,
            commands::open_settings,
            commands::write_clipboard,
            commands::mark_message_read,
            commands::mark_folder_read,
            commands::trash_message,
            commands::message_translation_text,
            translation::translate_text,
            links::open_link,
            imap::sync_status,
            imap::list_accounts,
            imap::add_account,
            imap::remove_account,
            imap::unlock_store,
            smtp::send_message,
            compose::open_compose,
            compose::compose_init,
            compose::pick_attachments,
        ])
        .setup(|app| {
            app.manage(config::ConfigStore::load(app.path().app_config_dir()?));
            let window = glass_window(app.handle(), "main", "mMail")
                .inner_size(1200.0, 780.0)
                .min_inner_size(900.0, 560.0)
                .on_new_window({
                    // Клик по ссылке в письме: окно не открываем никогда, а просим UI подтвердить.
                    let app = app.handle().clone();
                    move |url, _| {
                        if links::openable(&url) {
                            let request = app.state::<links::PendingLink>().request(url);
                            let _ = app.emit_to("main", "link-request", request);
                        }
                        NewWindowResponse::Deny
                    }
                })
                .build()?;
            finish_window(app.handle(), &window)?;
            // Крестик в режиме трея прячет окно; само приложение закрывается пунктом «Выйти».
            let handle = app.handle().clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    if tray::hides_on_close(&handle) {
                        api.prevent_close();
                        if let Some(main) = handle.get_webview_window("main") {
                            let _ = main.hide();
                        }
                    }
                }
            });
            if let Err(e) = tray::init(app.handle()) {
                eprintln!("mMail: трей недоступен: {e}");
            }
            tauri::async_runtime::spawn(imap::start(app.handle().clone()));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running mMail");
}

/// Окно приложения: без рамки KDE (шапка и кнопки свои), прозрачное — фон размывает KWin.
pub(crate) fn glass_window<'a>(app: &'a AppHandle, label: &str, title: &str) -> WebviewWindowBuilder<'a, tauri::Wry, AppHandle> {
    WebviewWindowBuilder::new(app, label, WebviewUrl::default())
        .title(title)
        .decorations(false)
        .transparent(true)
        .background_color(Color(0, 0, 0, 0))
        .zoom_hotkeys_enabled(false)
        .on_navigation(is_app_url)
}

/// Общее для всех окон: масштаб из настроек, меню редактирования, размытие KWin.
pub(crate) fn finish_window(app: &AppHandle, window: &WebviewWindow) -> tauri::Result<()> {
    let zoom = app.state::<config::ConfigStore>().get().zoom.unwrap_or(1.0);
    let _ = window.set_zoom(zoom.clamp(commands::ZOOM_MIN, commands::ZOOM_MAX));

    // Редактирование и копирование доступны, навигация WebKit остаётся запрещена.
    let handle = app.clone();
    window.with_webview(move |webview| {
        crate::context_menu::install(&webview.inner(), handle);
    })?;

    if let Err(e) = blur::enable(window, CORNER_RADIUS) {
        eprintln!("mMail: размытие KWin недоступно: {e}");
    }
    Ok(())
}

/// Webview может показывать только само приложение: любая другая навигация режется.
fn is_app_url(url: &Url) -> bool {
    match url.scheme() {
        "tauri" => true,
        "about" => matches!(url.path(), "blank" | "srcdoc"),
        "http" if cfg!(debug_assertions) => {
            url.host_str() == Some("127.0.0.1") && url.port() == Some(1420)
        }
        _ => false,
    }
}
