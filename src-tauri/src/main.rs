mod accounts;
mod attachments;
mod blur;
mod commands;
mod calendar;
mod cli;
mod compose;
mod config;
mod context_menu;
mod db;
mod fake;
mod google;
mod folders;
mod imap;
mod i18n;
mod links;
mod memory;
mod notify;
mod remote;
mod sanitize;
mod scheme;
mod smtp;
mod store;
mod tray;
mod window_pins;
mod window_state;
mod translation;

use tauri::webview::NewWindowResponse;
use tauri::window::Color;
use tauri::{AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// Должен совпадать с border-radius у `.window` в App.svelte.
const CORNER_RADIUS: i32 = 12;

fn main() {
    memory::limit_arenas();
    tauri::Builder::default()
        .manage(store::Store::default())
        .manage(imap::Imap::default())
        .manage(links::PendingLink::default())
        .manage(compose::ComposeState::default())
        .manage(tray::TrayState::default())
        .manage(window_pins::PinState::default())
        .manage(google::Google::default())
        // Второй запуск не создаёт копию, а передаёт аргументы работающему mMail (D-Bus):
        // без аргументов — показать окно, `-compose …` / `mailto:…` — открыть новое письмо.
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| match cli::parse(&argv, std::path::Path::new(&cwd)) {
            Ok(Some(prefill)) => compose::open_from_background(app, prefill),
            Ok(None) => {
                let handle = app.clone();
                let _ = app.run_on_main_thread(move || tray::show_main(&handle));
            }
            Err(e) => notify::cli_error(app, &e),
        }))
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
            commands::set_show_all_accounts,
            commands::set_notifications,
            commands::set_title_buttons,
            window_pins::window_pins,
            window_pins::set_window_pin,
            commands::write_clipboard,
            context_menu::context_menu_items,
            google::google_status,
            google::google_connect,
            google::google_disconnect,
            calendar::calendar_list,
            calendar::calendar_set_visible,
            calendar::calendar_events,
            calendar::calendar_save_event,
            calendar::calendar_delete_event,
            commands::mark_message_read,
            commands::mark_folder_read,
            commands::set_message_flags,
            commands::trash_message,
            translation::translate_message,
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
            compose::clean_compose_html,
        ])
        .setup(|app| {
            app.manage(config::ConfigStore::load(app.path().app_config_dir()?));
            scheme::init(app.handle());
            let builder = glass_window(app.handle(), "main", "mMail").inner_size(1200.0, 780.0).min_inner_size(900.0, 560.0);
            let window = window_state::restore(app.handle(), "main", builder, (900.0, 560.0))
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
            window_state::track(app.handle(), &window, "main");
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
            // Первый запуск сразу с письмом (`mmail -compose …`, ссылка mailto: из браузера).
            let argv: Vec<String> = std::env::args().collect();
            match cli::parse(&argv, &std::env::current_dir().unwrap_or_default()) {
                Ok(Some(prefill)) => compose::open_compose(app.handle().clone(), app.state(), prefill)?,
                Ok(None) => {}
                Err(e) => notify::cli_error(app.handle(), &e),
            }
            tauri::async_runtime::spawn(imap::start(app.handle().clone()));
            tauri::async_runtime::spawn(notify::reminders_loop(app.handle().clone()));
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
    let (handle, label) = (app.clone(), window.label().to_string());
    window.with_webview(move |webview| {
        crate::context_menu::install(&webview.inner(), handle, label);
    })?;

    if let Err(e) = blur::enable(window, CORNER_RADIUS) {
        eprintln!("mMail: размытие KWin недоступно: {e}");
    }
    window_pins::restore(app, window);
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
