//! Уведомления рабочего стола (freedesktop; в KDE — уведомления Plasma): новые письма
//! и напоминания о событиях календаря. Клик по уведомлению открывает mMail на нужном месте.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use chrono::Utc;
use notify_rust::{Hint, Notification};
use tauri::{AppHandle, Emitter, Manager};

use crate::config::ConfigStore;

/// Тело уведомления KDE понимает разметку (ссылки, картинки) — а тема письма и название события
/// это чужой текст. Экранируем всё, чтобы письмом нельзя было подсунуть ссылку или картинку-трекер.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// Заголовок — простой текст по спецификации; убираем переводы строк и ограничиваем длину.
fn plain(text: &str, limit: usize) -> String {
    let one_line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= limit {
        one_line
    } else {
        format!("{}…", one_line.chars().take(limit).collect::<String>())
    }
}

fn show(app: &AppHandle, summary: String, body: String, on_click: impl FnOnce(&AppHandle) + Send + 'static) {
    let app = app.clone();
    // Ожидание клика блокирует — у каждого уведомления свой поток, пока его не закроют.
    std::thread::spawn(move || {
        let shown = Notification::new()
            .appname("mMail")
            .icon("mmail")
            .summary(&summary)
            .body(&body)
            .action("default", &crate::i18n::text(&app, "Открыть"))
            .hint(Hint::DesktopEntry("mmail".into()))
            .show();
        match shown {
            Ok(handle) => handle.wait_for_action(|action| {
                if action == "default" {
                    on_click(&app);
                }
            }),
            Err(e) => eprintln!("mMail: уведомление не показано: {e}"),
        }
    });
}

/// Главное окно на виду и в фокусе — человек и так видит почту.
fn main_focused(app: &AppHandle) -> bool {
    app.get_webview_window("main").is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
}

pub struct NewMail {
    pub id: u32,
    pub account: String,
    pub from: String,
    pub subject: String,
}

/// Новые непрочитанные письма во «Входящих»: одно — с отправителем и темой, несколько — сводкой.
pub fn new_mail(app: &AppHandle, mail: Vec<NewMail>) {
    if mail.is_empty() || !app.state::<ConfigStore>().get().notify_mail.unwrap_or(true) || main_focused(app) {
        return;
    }
    let open = |id: Option<u32>, account: String| {
        move |app: &AppHandle| {
            crate::tray::show_main(app);
            let _ = app.emit_to("main", "open-message", serde_json::json!({ "id": id, "account": account }));
        }
    };
    if let [one] = mail.as_slice() {
        let summary = plain(&one.from, 80);
        let body = escape(&plain(&one.subject, 200));
        show(app, summary, body, open(Some(one.id), one.account.clone()));
        return;
    }
    let summary = crate::i18n::text(app, "Новых писем: {count}").replace("{count}", &mail.len().to_string());
    let body = mail.iter().take(4).map(|m| format!("<b>{}</b> — {}", escape(&plain(&m.from, 40)), escape(&plain(&m.subject, 60)))).collect::<Vec<_>>().join("\n");
    show(app, summary, body, open(None, mail[0].account.clone()));
}

/// Фоновая проверка напоминаний календаря: список событий — раз в 5 минут, сработавшие — каждые 30 секунд.
pub async fn reminders_loop(app: AppHandle) {
    let fired: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
    let mut upcoming: Vec<crate::calendar::Reminder> = Vec::new();
    let mut last_fetch: Option<std::time::Instant> = None;
    loop {
        let config = app.state::<ConfigStore>().get();
        if config.notify_events.unwrap_or(true) && config.google_account.is_some() {
            if last_fetch.is_none_or(|t| t.elapsed() >= Duration::from_secs(300)) {
                match crate::calendar::upcoming_reminders(&app).await {
                    Ok(list) => upcoming = list,
                    Err(e) => eprintln!("mMail: напоминания календаря: {e}"),
                }
                last_fetch = Some(std::time::Instant::now());
            }
            let now = Utc::now();
            for r in &upcoming {
                // Сработало в последние 2 минуты (компьютер мог спать — старые не показываем).
                let due = r.fire_at <= now && now - r.fire_at < chrono::Duration::minutes(2);
                if due && fired.lock().unwrap().insert(r.key.clone()) {
                    let date = r.date_key.clone();
                    show(&app, plain(&r.title, 120), escape(&r.when), move |app| {
                        crate::tray::show_main(app);
                        let _ = app.emit_to("main", "open-date", date);
                    });
                }
            }
        } else {
            last_fetch = None;
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreign_text_cannot_add_markup() {
        let body = escape("Счёт <a href=\"https://phish.example\">оплатить</a><img src=\"https://t.example/p\">");
        assert!(!body.contains('<') && !body.contains('>'), "{body}");
        assert_eq!(plain("  a\n\n b  ", 80), "a b");
        assert_eq!(plain("абвгд", 3), "абв…");
    }
}
