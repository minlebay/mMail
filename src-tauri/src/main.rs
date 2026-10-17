mod accounts;
mod attachments;
mod blur;
mod commands;
mod compose;
mod config;
mod db;
mod fake;
mod imap;
mod links;
mod remote;
mod sanitize;
mod smtp;
mod store;

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

/// Общее для всех окон: масштаб из настроек, без контекстного меню WebKit, размытие KWin.
pub(crate) fn finish_window(app: &AppHandle, window: &WebviewWindow) -> tauri::Result<()> {
    let zoom = app.state::<config::ConfigStore>().get().zoom.unwrap_or(1.0);
    let _ = window.set_zoom(zoom.clamp(commands::ZOOM_MIN, commands::ZOOM_MAX));

    // Никакого контекстного меню WebKit (Назад/Обновить/Inspect) — ни в UI, ни в письмах.
    window.with_webview(|webview| {
        use webkit2gtk::WebViewExt;
        webview.inner().connect_context_menu(|_, _, _, _| true);
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
