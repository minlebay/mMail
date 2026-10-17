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

/// `buttons` — кнопки уведомления (id, ключ перевода); клик по самому уведомлению — действие `default`.
fn show(
    app: &AppHandle,
    summary: String,
    body: String,
    buttons: &[(&'static str, &'static str)],
    on_action: impl FnOnce(&AppHandle, &str) + Send + 'static,
) {
    let app = app.clone();
    let buttons: Vec<(&'static str, String)> = buttons.iter().map(|(id, label)| (*id, crate::i18n::text(&app, label))).collect();
    // Ожидание действия блокирует — у каждого уведомления свой поток, пока его не закроют.
    std::thread::spawn(move || {
        let mut n = Notification::new();
        n.appname("mMail")
            .icon("mmail")
            .summary(&summary)
            .body(&body)
            .action("default", &crate::i18n::text(&app, "Открыть"))
            .hint(Hint::DesktopEntry("mmail".into()));
        for (id, label) in &buttons {
            n.action(id, label);
        }
        match n.show() {
            Ok(handle) => handle.wait_for_action(|action| {
                // «__closed» — уведомление закрыли или оно истекло.
                if action != "__closed" {
                    on_action(&app, action);
                }
            }),
            Err(e) => eprintln!("mMail: уведомление не показано: {e}"),
        }
    });
}

fn open_message(app: &AppHandle, id: Option<u32>, account: &str) {
    crate::tray::show_main(app);
    let _ = app.emit_to("main", "open-message", serde_json::json!({ "id": id, "account": account }));
}

/// Действие над письмами из уведомления — в фоне; ошибку показываем в окне, если оно открыто.
fn act_on(app: &AppHandle, ids: Vec<u32>, action: &str) {
    let (app, action) = (app.clone(), action.to_string());
    tauri::async_runtime::spawn(async move {
        for id in ids {
            let result = match action.as_str() {
                "read" => crate::commands::set_message_flags(app.clone(), app.state(), app.state(), id, Some(false), None).await,
                "trash" => crate::commands::trash_message(app.clone(), app.state(), app.state(), id).await,
                _ => Ok(()),
            };
            if let Err(e) = result {
                let _ = app.emit_to("main", "operation-error", e);
            }
        }
    });
}

/// Главное окно на виду и в фокусе — человек и так видит почту.
fn main_focused(app: &AppHandle) -> bool {
    app.get_webview_window("main").is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
}

/// «Отложить» письмо и «Напомнить через 5 мин» о событии.
const MAIL_SNOOZE: Duration = Duration::from_secs(15 * 60);
const EVENT_SNOOZE: Duration = Duration::from_secs(5 * 60);

/// Повторить уведомление позже (поток уведомления к этому времени уже закончился).
fn later(app: &AppHandle, delay: Duration, then: impl FnOnce(&AppHandle) + Send + 'static) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        then(&app);
    });
}

#[derive(Clone)]
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
    mail_notification(app, mail);
}

/// Отложенные письма возвращаются, только если они ещё есть и не прочитаны.
fn snooze_mail(app: &AppHandle, mail: Vec<NewMail>) {
    later(app, MAIL_SNOOZE, move |app| {
        let store = app.state::<crate::store::Store>();
        let mail: Vec<NewMail> = mail.into_iter().filter(|m| store.details(m.id).is_some_and(|d| d.unread)).collect();
        if !mail.is_empty() && app.state::<ConfigStore>().get().notify_mail.unwrap_or(true) {
            mail_notification(app, mail);
        }
    });
}

fn mail_notification(app: &AppHandle, mail: Vec<NewMail>) {
    if let [one] = mail.as_slice() {
        let summary = plain(&one.from, 80);
        let body = escape(&plain(&one.subject, 200));
        let (id, account) = (one.id, one.account.clone());
        let buttons = [("reply", "Ответить"), ("read", "Прочитано"), ("trash", "Удалить"), ("snooze", "Отложить")];
        show(app, summary, body, &buttons, move |app, action| match action {
            "snooze" => snooze_mail(app, mail),
            "reply" => {
                if let Err(e) = crate::compose::open_reply(app, id) {
                    eprintln!("mMail: ответ из уведомления: {e}");
                }
            }
            "read" | "trash" => act_on(app, vec![id], action),
            _ => open_message(app, Some(id), &account),
        });
        return;
    }
    let summary = crate::i18n::text(app, "Новых писем: {count}").replace("{count}", &mail.len().to_string());
    let body = mail.iter().take(4).map(|m| format!("<b>{}</b> — {}", escape(&plain(&m.from, 40)), escape(&plain(&m.subject, 60)))).collect::<Vec<_>>().join("\n");
    let ids: Vec<u32> = mail.iter().map(|m| m.id).collect();
    let account = mail[0].account.clone();
    show(app, summary, body, &[("read", "Все прочитаны"), ("snooze", "Отложить")], move |app, action| match action {
        "read" => act_on(app, ids, "read"),
        "snooze" => snooze_mail(app, mail),
        _ => open_message(app, None, &account),
    });
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
                    remind(&app, r.clone());
                }
            }
        } else {
            last_fetch = None;
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

fn remind(app: &AppHandle, r: crate::calendar::Reminder) {
    let (summary, body) = (plain(&r.title, 120), escape(&r.when(app)));
    show(app, summary, body, &[("snooze", "Напомнить через 5 мин")], move |app, action| {
        if action == "snooze" {
            later(app, EVENT_SNOOZE, move |app| remind(app, r));
        } else {
            crate::tray::show_main(app);
            let _ = app.emit_to("main", "open-date", r.date_key);
        }
    });
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
