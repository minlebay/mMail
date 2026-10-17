mod blur;
mod commands;
mod config;
mod fake;
mod imap;
mod links;
mod remote;
mod sanitize;
mod store;

use tauri::webview::NewWindowResponse;
use tauri::window::Color;
use tauri::{Emitter, Manager, Url, WebviewUrl, WebviewWindowBuilder};

/// Должен совпадать с border-radius у `.window` в App.svelte.
const CORNER_RADIUS: i32 = 12;

fn main() {
    tauri::Builder::default()
        .manage(store::Store::default())
        .manage(imap::Imap::default())
        .manage(links::PendingLink::default())
        .invoke_handler(tauri::generate_handler![
            commands::list_folders,
            commands::list_messages,
            commands::get_message,
            commands::set_sender_trusted,
            links::open_link,
            imap::sync_status,
        ])
        .setup(|app| {
            app.manage(config::ConfigStore::load(app.path().app_config_dir()?));
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                .title("mMail")
                .inner_size(1200.0, 780.0)
                .min_inner_size(900.0, 560.0)
                // Рамки KDE нет: шапка и кнопки свои (TitleBar.svelte), фон под окном размывает KWin.
                .decorations(false)
                .transparent(true)
                .background_color(Color(0, 0, 0, 0))
                .on_navigation(is_app_url)
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

            if let Err(e) = blur::enable(&window, CORNER_RADIUS) {
                eprintln!("mMail: размытие KWin недоступно: {e}");
            }
            tauri::async_runtime::spawn(imap::start(app.handle().clone()));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running mMail");
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
