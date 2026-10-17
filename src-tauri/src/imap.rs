//! IMAP: аккаунт из KWallet, TLS-подключение, синхронизация заголовков и загрузка писем.

use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use async_imap::types::{Fetch, Flag, NameAttribute};
use futures::TryStreamExt;
use mail_parser::MessageParser;
use secret_service::{EncryptionType, SecretService};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{self, ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;
use zeroize::Zeroizing;

use crate::fake;
use crate::store::{Auth, Folder, Message, Store};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
type Session = async_imap::Session<TlsStream<TcpStream>>;

/// Писем за один FETCH: каждая порция сразу появляется в списке.
const BATCH: u32 = 200;
const TIMEOUT: Duration = Duration::from_secs(20);

struct Provider {
    imap_host: &'static str,
    /// Authserv-id серверов провайдера: верим только их Authentication-Results.
    trusted_authserv: &'static [&'static str],
}

const YANDEX: Provider = Provider { imap_host: "imap.yandex.com", trusted_authserv: &[".yandex.net", ".yandex.ru"] };
const GMAIL: Provider = Provider { imap_host: "imap.gmail.com", trusted_authserv: &["mx.google.com"] };

fn provider_for(email: &str) -> Option<&'static Provider> {
    let domain = email.rsplit_once('@')?.1.to_ascii_lowercase();
    match domain.as_str() {
        "gmail.com" | "googlemail.com" => Some(&GMAIL),
        "yandex.ru" | "yandex.com" | "ya.ru" | "yandex.by" | "yandex.kz" | "narod.ru" => Some(&YANDEX),
        _ => None,
    }
}

pub struct Account {
    email: String,
    provider: &'static Provider,
}

/// Аккаунт и пароль приложения из KWallet (Secret Service): запись с атрибутами
/// `service=mmail`, `account=<адрес>`. Пароль не хранится дольше входа на сервер.
async fn load_account() -> Result<Option<(Account, Zeroizing<String>)>> {
    let ss = SecretService::connect(EncryptionType::Dh).await?;
    let found = ss.search_items(HashMap::from([("service", "mmail")])).await?;
    let Some(item) = found.unlocked.into_iter().next().or(found.locked.into_iter().next()) else {
        return Ok(None);
    };
    item.unlock().await?;
    let email = item.get_attributes().await?.remove("account").ok_or("в записи KWallet нет атрибута account")?;
    let provider = provider_for(&email).ok_or_else(|| format!("неизвестный провайдер для {email}"))?;
    let secret = Zeroizing::new(item.get_secret().await?);
    let password = Zeroizing::new(String::from_utf8(secret.to_vec())?);
    Ok(Some((Account { email, provider }, password)))
}

async fn connect(account: &Account, password: &str) -> Result<Session> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    // Только TLS 1.2+ и проверка сертификата по корням Mozilla; implicit TLS на 993, без STARTTLS.
    let config = ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let host = account.provider.imap_host;
    let tcp = tokio::time::timeout(TIMEOUT, TcpStream::connect((host, 993))).await??;
    let tls = TlsConnector::from(Arc::new(config)).connect(ServerName::try_from(host)?, tcp).await?;

    let mut client = async_imap::Client::new(tls);
    client.read_response().await?.ok_or("сервер закрыл соединение до приветствия")?;
    let session = client.login(&account.email, password).await.map_err(|(e, _)| e)?;
    Ok(session)
}

/// Имена стандартных папок на сервере по атрибутам SPECIAL-USE (RFC 6154).
async fn discover_folders(session: &mut Session) -> Result<HashMap<Folder, String>> {
    let names: Vec<_> = session.list(Some(""), Some("*")).await?.try_collect().await?;
    let mut folders = HashMap::from([(Folder::Inbox, "INBOX".to_string())]);
    for name in &names {
        let folder = name.attributes().iter().find_map(|a| match a {
            NameAttribute::Sent => Some(Folder::Sent),
            NameAttribute::Drafts => Some(Folder::Drafts),
            NameAttribute::Trash => Some(Folder::Trash),
            _ => None,
        });
        if let Some(folder) = folder {
            folders.insert(folder, name.name().to_string());
        }
    }
    Ok(folders)
}

/// Курсор синхронизации папки: идём от самых новых писем (большие номера) к старым.
struct Cursor {
    folder: Folder,
    name: String,
    total: u32,
    /// Номер последнего ещё не загруженного письма; 0 — папка загружена целиком.
    next: u32,
}

async fn fetch_batch(session: &mut Session, cursor: &mut Cursor) -> Result<Vec<Message>> {
    if cursor.next == 0 {
        return Ok(Vec::new());
    }
    session.examine(&cursor.name).await?;
    let first = cursor.next.saturating_sub(BATCH - 1).max(1);
    let started = std::time::Instant::now();
    // ENVELOPE сервер берёт из индекса; полные заголовки Яндекс отдаёт заметно медленнее.
    let fetches: Vec<Fetch> = session
        .fetch(format!("{first}:{}", cursor.next), "(UID FLAGS INTERNALDATE ENVELOPE)")
        .await?
        .try_collect()
        .await?;
    eprintln!("mMail: {} {first}:{} за {:?}", cursor.name, cursor.next, started.elapsed());
    cursor.next = first - 1;
    Ok(fetches.iter().filter_map(|f| to_message(f, cursor.folder)).collect())
}

fn to_message(fetch: &Fetch, folder: Folder) -> Option<Message> {
    let env = fetch.envelope()?;
    // Поля ENVELOPE приходят как есть (RFC 2047, «=?utf-8?B?…?=»): собираем мини-заголовок
    // и отдаём его mail-parser, чтобы он раскодировал так же, как полное письмо.
    let mut block = Vec::new();
    let mut line = |name: &[u8], value: &[u8]| {
        block.extend_from_slice(name);
        block.extend_from_slice(value);
        block.extend_from_slice(b"\r\n");
    };
    if let Some(subject) = &env.subject {
        line(b"Subject: ", subject);
    }
    if let Some(date) = &env.date {
        line(b"Date: ", date);
    }
    if let Some(addr) = env.from.as_ref().and_then(|list| list.first()) {
        let mut value = addr.name.as_deref().unwrap_or_default().to_vec();
        value.extend_from_slice(b" <");
        value.extend_from_slice(addr.mailbox.as_deref().unwrap_or_default());
        value.push(b'@');
        value.extend_from_slice(addr.host.as_deref().unwrap_or_default());
        value.push(b'>');
        line(b"From: ", &value);
    }
    block.extend_from_slice(b"\r\n");
    let parsed = MessageParser::default().parse_headers(&block)?;

    let from = parsed.from().and_then(|a| a.first());
    let from_addr = from.and_then(|a| a.address()).unwrap_or_default().to_string();
    let from_name = from.and_then(|a| a.name()).map(str::to_string).unwrap_or_else(|| from_addr.clone());
    let timestamp = parsed
        .date()
        .map(|d| d.to_timestamp())
        .or_else(|| fetch.internal_date().map(|d| d.timestamp()))
        .unwrap_or_default();
    let flags: Vec<_> = fetch.flags().collect();

    Some(Message {
        folder,
        uid: fetch.uid,
        starred: flags.contains(&Flag::Flagged),
        unread: !flags.contains(&Flag::Seen),
        from_name,
        from_addr,
        subject: parsed.subject().unwrap_or("(без темы)").to_string(),
        timestamp,
        // Проверка отправителя — по полным заголовкам, когда письмо открывают.
        auth: Auth::Unknown,
        html: None,
    })
}

/// Вердикт по самому верхнему Authentication-Results от серверов провайдера.
/// Заголовки с чужим authserv-id мог вписать сам отправитель — их игнорируем.
fn auth_verdict(headers: impl Iterator<Item = String>, trusted: &[&str]) -> Auth {
    for header in headers {
        let value = header.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase();
        let authserv = value.split(';').next().unwrap_or_default().split_whitespace().next().unwrap_or_default();
        if !trusted.iter().any(|t| authserv == t.trim_start_matches('.') || authserv.ends_with(t)) {
            continue;
        }
        let has = |result: &str| value.split([';', ' ']).any(|token| token == result);
        return if has("dmarc=pass") {
            Auth::Pass
        } else if has("dmarc=fail") || (has("spf=fail") && !has("dkim=pass")) {
            Auth::Fail
        } else {
            Auth::Unknown
        };
    }
    Auth::Unknown
}

#[derive(Clone, Serialize)]
pub struct SyncStatus {
    state: &'static str,
    message: String,
}

/// Живое подключение для загрузки писем по требованию и последний статус синхронизации.
pub struct Imap {
    inner: Mutex<Option<Connected>>,
    status: std::sync::Mutex<SyncStatus>,
}

struct Connected {
    account: Account,
    folders: HashMap<Folder, String>,
    session: Option<Session>,
}

impl Default for Imap {
    fn default() -> Self {
        Self {
            inner: Mutex::new(None),
            status: std::sync::Mutex::new(SyncStatus { state: "busy", message: "Подключение…".into() }),
        }
    }
}

fn status(app: &AppHandle, state: &'static str, message: impl Into<String>) {
    let status = SyncStatus { state, message: message.into() };
    eprintln!("mMail: [{state}] {}", status.message);
    *app.state::<Imap>().status.lock().unwrap() = status.clone();
    let _ = app.emit("sync-status", status);
}

/// Для UI, который подписался на события позже, чем ядро начало синхронизацию.
#[tauri::command]
pub fn sync_status(imap: tauri::State<'_, Imap>) -> SyncStatus {
    imap.status.lock().unwrap().clone()
}

/// Запуск: есть аккаунт в KWallet — синхронизируем его, нет — показываем демо-данные.
pub async fn start(app: AppHandle) {
    status(&app, "busy", "Подключение…");
    match sync(&app).await {
        Ok(true) => status(&app, "ok", ""),
        Ok(false) => {
            app.state::<Store>().replace_all(fake::generate(10_000));
            let _ = app.emit("mailbox-changed", ());
            status(&app, "demo", "Демо-данные: аккаунт в KWallet не найден");
        }
        Err(e) => status(&app, "error", format!("Ошибка синхронизации: {e}")),
    }
}

async fn sync(app: &AppHandle) -> Result<bool> {
    let Some((account, password)) = load_account().await? else { return Ok(false) };
    status(app, "busy", format!("Вход в {}…", account.email));
    // Два подключения: фоновая синхронизация не задерживает открытие писем.
    let mut session = connect(&account, &password).await?;
    let reader = connect(&account, &password).await?;
    drop(password);

    let folders = discover_folders(&mut session).await?;
    let mut cursors = Vec::new();
    for folder in [Folder::Inbox, Folder::Sent, Folder::Drafts, Folder::Trash] {
        let Some(name) = folders.get(&folder) else { continue };
        let total = session.examine(name).await?.exists;
        cursors.push(Cursor { folder, name: name.clone(), total, next: total });
    }
    *app.state::<Imap>().inner.lock().await = Some(Connected { account, folders, session: Some(reader) });

    let store = app.state::<Store>();
    let step = |cursor: &Cursor, messages: Vec<Message>| {
        store.append(messages);
        let _ = app.emit("mailbox-changed", ());
        let done = cursor.total - cursor.next;
        let label = match cursor.folder {
            Folder::Inbox => "Входящие",
            Folder::Sent => "Отправленные",
            Folder::Drafts => "Черновики",
            Folder::Trash => "Корзина",
        };
        status(app, "busy", format!("Синхронизация: {label} {done} / {}", cursor.total));
    };
    // Сначала свежие письма всех папок, потом в фоне — всё остальное, входящие первыми.
    for cursor in &mut cursors {
        let messages = fetch_batch(&mut session, cursor).await?;
        step(cursor, messages);
    }
    for cursor in &mut cursors {
        while cursor.next > 0 {
            let messages = fetch_batch(&mut session, cursor).await?;
            step(cursor, messages);
        }
    }
    let _ = session.logout().await;
    Ok(true)
}

impl Imap {
    /// Исходный HTML письма и проверка отправителя (BODY.PEEK — письмо не помечается прочитанным).
    pub async fn open(&self, folder: Folder, uid: u32) -> Result<(String, Auth)> {
        let mut guard = self.inner.lock().await;
        let conn = guard.as_mut().ok_or("нет подключения к серверу")?;
        let name = conn.folders.get(&folder).ok_or("папка не найдена на сервере")?.clone();

        // Одна попытка переподключения: соединение могло оборваться, пока приложение простаивало.
        for attempt in 0..2 {
            if conn.session.is_none() {
                let (account, password) = load_account().await?.ok_or("аккаунт пропал из KWallet")?;
                conn.session = Some(connect(&account, &password).await?);
                conn.account = account;
            }
            let session = conn.session.as_mut().unwrap();
            match fetch_raw(session, &name, uid).await {
                Ok(raw) => {
                    let parsed = MessageParser::default().parse(&raw).ok_or("не удалось разобрать письмо")?;
                    let auth_results = parsed
                        .headers()
                        .iter()
                        .filter(|h| h.name().eq_ignore_ascii_case("Authentication-Results"))
                        .map(|h| String::from_utf8_lossy(&raw[h.offset_start as usize..h.offset_end as usize]).into_owned());
                    let auth = auth_verdict(auth_results, conn.account.provider.trusted_authserv);
                    let html = parsed.body_html(0).map(|h| h.into_owned()).unwrap_or_default();
                    return Ok((html, auth));
                }
                Err(e) if attempt == 0 => {
                    eprintln!("mMail: IMAP-сессия сброшена ({e}), переподключаюсь");
                    conn.session = None;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }
}

async fn fetch_raw(session: &mut Session, folder: &str, uid: u32) -> Result<Vec<u8>> {
    session.examine(folder).await?;
    let fetches: Vec<Fetch> = session.uid_fetch(uid.to_string(), "BODY.PEEK[]").await?.try_collect().await?;
    let body = fetches.first().and_then(|f| f.body()).ok_or("сервер не вернул письмо")?;
    Ok(body.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verdict(headers: &[&str]) -> Auth {
        auth_verdict(headers.iter().map(|h| h.to_string()), YANDEX.trusted_authserv)
    }

    #[test]
    fn trusts_only_provider_results() {
        let forged = "evil.example; dmarc=pass";
        let real = "mxfront12.mail.yandex.net;\r\n\tspf=fail smtp.mail=a@b; dkim=none";
        assert!(verdict(&[forged, real]) == Auth::Fail, "подделка отправителя не должна давать pass");
        assert!(verdict(&[forged]) == Auth::Unknown);
    }

    #[test]
    fn reads_dmarc() {
        assert!(verdict(&["mx.yandex.net; spf=pass; dkim=pass; dmarc=pass header.from=github.com"]) == Auth::Pass);
        assert!(verdict(&["mx.yandex.net; dmarc=fail header.from=paypal.com"]) == Auth::Fail);
        assert!(verdict(&["mx.yandex.net; spf=pass; dkim=pass"]) == Auth::Unknown, "без DMARC не утверждаем pass");
        assert!(verdict(&["mx.yandex.net; dmarc=passive"]) == Auth::Unknown);
    }
}
