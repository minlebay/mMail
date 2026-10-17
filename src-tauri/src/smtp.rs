//! Отправка писем: SMTP с implicit TLS (465), пароль приложения из KWallet.

use std::net::Ipv4Addr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lettre::message::header::ContentType;
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::extension::ClientId;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde::Deserialize;
use tauri::State;

use crate::accounts::{self, Account};
use crate::imap::Imap;
use crate::store::Folder;

/// Не даём случайно отправить гигантское письмо или разослать по огромному списку.
const MAX_RECIPIENTS: usize = 50;
const MAX_BODY: usize = 1024 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    from: String,
    to: String,
    cc: String,
    subject: String,
    body: String,
}

/// Адреса через запятую или точку с запятой; каждый проверяется парсером lettre
/// (заодно это защищает от подстановки лишних заголовков через переводы строк).
fn parse_list(list: &str) -> Result<Vec<Mailbox>, String> {
    list.split([',', ';'])
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(|a| a.parse::<Mailbox>().map_err(|_| format!("некорректный адрес: {a}")))
        .collect()
}

fn build(account: &Account, draft: &Draft) -> Result<Message, String> {
    let to = parse_list(&draft.to)?;
    let cc = parse_list(&draft.cc)?;
    if to.is_empty() {
        return Err("укажите получателя".into());
    }
    if to.len() + cc.len() > MAX_RECIPIENTS {
        return Err(format!("не больше {MAX_RECIPIENTS} получателей"));
    }
    if draft.body.len() > MAX_BODY {
        return Err("письмо слишком большое".into());
    }

    let from: Mailbox = account.email.parse().map_err(|_| "некорректный адрес отправителя")?;
    let domain = account.email.rsplit_once('@').map_or("localhost", |(_, d)| d);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    // Message-ID на домене ящика: без имени компьютера и без User-Agent.
    let mut builder = Message::builder()
        .from(from)
        .subject(draft.subject.trim())
        .message_id(Some(format!("<{nanos:x}.{:x}@{domain}>", std::process::id())));
    for mailbox in to {
        builder = builder.to(mailbox);
    }
    for mailbox in cc {
        builder = builder.cc(mailbox);
    }
    builder.header(ContentType::TEXT_PLAIN).body(draft.body.clone()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn send_message(imap: State<'_, Imap>, draft: Draft) -> Result<(), String> {
    let account = Account::new(&draft.from).map_err(|e| e.to_string())?;
    let message = build(&account, &draft)?;
    let password = accounts::password(&account.email).await.map_err(|e| e.to_string())?;

    // relay(): порт 465 и TLS сразу при подключении, сертификат проверяется по корням Mozilla.
    let transport = AsyncSmtpTransport::<Tokio1Executor>::relay(account.provider.smtp_host)
        .map_err(|e| e.to_string())?
        .credentials(Credentials::new(account.email.clone(), password.to_string()))
        .hello_name(ClientId::Ipv4(Ipv4Addr::LOCALHOST))
        .timeout(Some(Duration::from_secs(30)))
        .build();
    drop(password);
    transport.send(message.clone()).await.map_err(|e| format!("не удалось отправить: {e}"))?;

    // Gmail кладёт отправленное в «Отправленные» сам, Яндексу — кладём копию через IMAP.
    if !account.provider.saves_sent {
        if let Err(e) = imap.append(&account.email, Folder::Sent, &message.formatted(), "(\\Seen)").await {
            eprintln!("mMail: письмо отправлено, но копия в «Отправленные» не сохранена: {e}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(to: &str, subject: &str) -> Draft {
        Draft { from: "me@yandex.ru".into(), to: to.into(), cc: String::new(), subject: subject.into(), body: "hi".into() }
    }

    #[test]
    fn rejects_header_injection_and_bad_addresses() {
        let account = Account::new("me@yandex.ru").unwrap();
        assert!(build(&account, &draft("a@b.example\r\nBcc: victim@c.example", "x")).is_err());
        assert!(build(&account, &draft("not an address", "x")).is_err());
        assert!(build(&account, &draft("", "x")).is_err());
        let ok = build(&account, &draft("a@b.example; Имя <c@d.example>", "Тема\r\nBcc: x@y.example")).unwrap();
        let raw = String::from_utf8(ok.formatted()).unwrap();
        assert!(!raw.contains("\r\nBcc:"), "перевод строки в теме не должен давать новый заголовок: {raw}");
        assert!(!raw.to_ascii_lowercase().contains("user-agent"));
    }
}
