//! Отправка писем: SMTP с implicit TLS (465), пароль приложения из KWallet.

use std::net::Ipv4Addr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::extension::ClientId;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};

use crate::accounts::{self, Account};
use crate::compose::{ComposeState, MAX_ATTACHMENTS_BYTES};
use crate::imap::Imap;
use crate::sanitize;
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
    /// Текстовая версия (всегда).
    body: String,
    /// HTML-версия, если письмо писали с форматированием.
    #[serde(default)]
    html: Option<String>,
    /// id файлов, выбранных через pick_attachments.
    #[serde(default)]
    attachments: Vec<u64>,
    #[serde(default)]
    in_reply_to: Option<String>,
    #[serde(default)]
    references: Vec<String>,
}

pub struct File {
    name: String,
    mime: String,
    bytes: Vec<u8>,
}

/// Message-ID в виде `<…>`; всё, что похоже на попытку дописать заголовок, отбрасываем.
fn message_id(raw: &str) -> Option<String> {
    let id = raw.trim().trim_start_matches('<').trim_end_matches('>');
    let ok = !id.is_empty() && id.len() < 512 && id.chars().all(|c| c.is_ascii_graphic() && c != '<' && c != '>');
    ok.then(|| format!("<{id}>"))
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

fn build(account: &Account, draft: &Draft, files: Vec<File>) -> Result<Message, String> {
    let to = parse_list(&draft.to)?;
    let cc = parse_list(&draft.cc)?;
    if to.is_empty() {
        return Err("укажите получателя".into());
    }
    if to.len() + cc.len() > MAX_RECIPIENTS {
        return Err(format!("не больше {MAX_RECIPIENTS} получателей"));
    }
    if draft.body.len() > MAX_BODY || draft.html.as_ref().is_some_and(|h| h.len() > MAX_BODY) {
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
    if let Some(id) = draft.in_reply_to.as_deref().and_then(message_id) {
        builder = builder.in_reply_to(id);
    }
    let references: Vec<_> = draft.references.iter().filter_map(|r| message_id(r)).collect();
    if !references.is_empty() {
        builder = builder.references(references.join(" "));
    }

    // HTML из редактора ещё раз чистим тем же санитайзером, что и входящие.
    let text = SinglePart::plain(draft.body.clone());
    let content = match &draft.html {
        Some(html) => {
            let clean = sanitize::clean_html(html, true).html;
            let document = format!("<!doctype html><html><head><meta charset=\"utf-8\"></head><body>{clean}</body></html>");
            MultiPart::alternative().singlepart(text).singlepart(SinglePart::html(document))
        }
        None if files.is_empty() => return builder.singlepart(text).map_err(|e| e.to_string()),
        None => MultiPart::mixed().singlepart(text),
    };
    if files.is_empty() {
        return builder.multipart(content).map_err(|e| e.to_string());
    }
    let mut mixed = MultiPart::mixed().multipart(content);
    for file in files {
        let mime = ContentType::parse(&file.mime).unwrap_or(ContentType::parse("application/octet-stream").unwrap());
        mixed = mixed.singlepart(Attachment::new(file.name).body(file.bytes, mime));
    }
    builder.multipart(mixed).map_err(|e| e.to_string())
}

/// Читает выбранные файлы (пути знает только ядро) с проверкой общего размера.
async fn load_files(compose: &ComposeState, ids: &[u64]) -> Result<Vec<File>, String> {
    let mut files = Vec::new();
    let mut total = 0u64;
    for &id in ids {
        let path = compose.file(id).ok_or("вложение не найдено — прикрепите файл заново")?;
        let bytes = tokio::fs::read(&path).await.map_err(|e| format!("{}: {e}", path.display()))?;
        total += bytes.len() as u64;
        if total > MAX_ATTACHMENTS_BYTES {
            return Err("вложения больше 25 МБ".into());
        }
        files.push(File {
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into()),
            mime: mime_guess::from_path(&path).first_or_octet_stream().to_string(),
            bytes,
        });
    }
    Ok(files)
}

#[tauri::command]
pub async fn send_message(
    app: AppHandle,
    imap: State<'_, Imap>,
    compose: State<'_, ComposeState>,
    draft: Draft,
) -> Result<(), String> {
    let account = Account::new(&draft.from).map_err(|e| e.to_string())?;
    let files = load_files(&compose, &draft.attachments).await?;
    let message = build(&account, &draft, files)?;
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
    let _ = app.emit_to("main", "message-sent", ());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(to: &str, subject: &str) -> Draft {
        Draft {
            from: "me@yandex.ru".into(),
            to: to.into(),
            cc: String::new(),
            subject: subject.into(),
            body: "hi".into(),
            html: None,
            attachments: Vec::new(),
            in_reply_to: None,
            references: Vec::new(),
        }
    }

    fn build_plain(account: &Account, draft: &Draft) -> Result<Message, String> {
        build(account, draft, Vec::new())
    }

    #[test]
    fn reply_headers_and_html_are_safe() {
        let account = Account::new("me@yandex.ru").unwrap();
        let mut d = draft("a@b.example", "Re: x");
        d.in_reply_to = Some("abc@host.example".into());
        d.references = vec!["old@host.example".into(), "bad\r\nBcc: x@y.example".into()];
        d.html = Some(r#"<p>hi</p><script>alert(1)</script><img src="x" onerror="alert(2)">"#.into());
        let raw = String::from_utf8(build(&account, &d, vec![File { name: "a.txt".into(), mime: "text/plain".into(), bytes: b"x".to_vec() }]).unwrap().formatted()).unwrap();
        assert!(raw.contains("In-Reply-To: <abc@host.example>"));
        assert!(raw.contains("References: <old@host.example>"));
        assert!(!raw.contains("Bcc:") && !raw.contains("<script") && !raw.contains("onerror"), "{raw}");
        assert!(raw.contains("multipart/mixed") && raw.contains("multipart/alternative") && raw.contains("a.txt"));
    }

    #[test]
    fn rejects_header_injection_and_bad_addresses() {
        let account = Account::new("me@yandex.ru").unwrap();
        assert!(build_plain(&account, &draft("a@b.example\r\nBcc: victim@c.example", "x")).is_err());
        assert!(build_plain(&account, &draft("not an address", "x")).is_err());
        assert!(build_plain(&account, &draft("", "x")).is_err());
        let ok = build_plain(&account, &draft("a@b.example; Имя <c@d.example>", "Тема\r\nBcc: x@y.example")).unwrap();
        let raw = String::from_utf8(ok.formatted()).unwrap();
        assert!(!raw.contains("\r\nBcc:"), "перевод строки в теме не должен давать новый заголовок: {raw}");
        assert!(!raw.to_ascii_lowercase().contains("user-agent"));
    }
}
