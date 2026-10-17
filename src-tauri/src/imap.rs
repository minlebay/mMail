//! IMAP: TLS-подключение, фоновая синхронизация каждого ящика и загрузка писем по требованию.

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_imap::types::{Fetch, Flag, NameAttribute};
use futures::TryStreamExt;
use mail_parser::MessageParser;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{self, ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;
use zeroize::Zeroizing;

use crate::accounts::{self, Account};
use crate::config::ConfigStore;
use crate::fake;
use crate::store::{Auth, Folder, Message, Opened, Store};

pub type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
type Session = async_imap::Session<TlsStream<TcpStream>>;

/// Писем за один FETCH: каждая порция сразу появляется в списке.
const BATCH: u32 = 200;
const TIMEOUT: Duration = Duration::from_secs(20);
/// Как часто сверять флаги (прочитано/важное), изменённые в других клиентах.
/// Новые письма проверяются чаще — интервал в настройках.
const FLAGS_EVERY: Duration = Duration::from_secs(60);
/// У скольких последних писем папки сверять флаги.
const FLAGS_WINDOW: u32 = 500;
const HEADER_QUERY: &str = "(UID FLAGS INTERNALDATE BODY.PEEK[HEADER.FIELDS (FROM SUBJECT DATE)])";

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

async fn fetch_batch(session: &mut Session, account: &str, cursor: &mut Cursor) -> Result<Vec<Message>> {
    if cursor.next == 0 {
        return Ok(Vec::new());
    }
    session.examine(&cursor.name).await?;
    let first = cursor.next.saturating_sub(BATCH - 1).max(1);
    let started = std::time::Instant::now();
    // Только нужные поля заголовка — так же синхронизирует Thunderbird.
    let fetches: Vec<Fetch> = session
        .fetch(format!("{first}:{}", cursor.next), HEADER_QUERY)
        .await?
        .try_collect()
        .await?;
    eprintln!("mMail: {} {first}:{} за {:?}", cursor.name, cursor.next, started.elapsed());
    cursor.next = first - 1;
    Ok(fetches.iter().filter_map(|f| to_message(f, account, cursor.folder)).collect())
}

fn to_message(fetch: &Fetch, account: &str, folder: Folder) -> Option<Message> {
    let parsed = MessageParser::default().parse_headers(fetch.header()?)?;
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
        account: Some(account.to_string()),
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
        message_id: None,
        references: Vec::new(),
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

/// Подключённый ящик: папки на сервере и отдельная сессия для открытия писем.
struct Connection {
    account: Account,
    folders: HashMap<Folder, String>,
    reader: Mutex<Option<Session>>,
    /// Ящик удалён или добавлен заново — фоновая синхронизация должна остановиться.
    cancelled: AtomicBool,
}

#[derive(Default)]
pub struct Imap {
    connections: std::sync::Mutex<HashMap<String, Arc<Connection>>>,
    /// Статус по каждому ящику; в шапку уходит сводка.
    statuses: std::sync::Mutex<BTreeMap<String, SyncStatus>>,
    demo: AtomicBool,
}

fn set_status(app: &AppHandle, email: &str, state: &'static str, message: impl Into<String>) {
    let message = message.into();
    eprintln!("mMail: [{state}] {email}: {message}");
    let imap = app.state::<Imap>();
    imap.statuses.lock().unwrap().insert(email.to_string(), SyncStatus { state, message });
    let _ = app.emit("sync-status", imap.summary());
}

impl Imap {
    fn summary(&self) -> SyncStatus {
        if self.demo.load(Ordering::Relaxed) {
            return SyncStatus { state: "demo", message: "Демо-данные: добавьте почтовый ящик в настройках".into() };
        }
        let statuses = self.statuses.lock().unwrap();
        if statuses.is_empty() {
            return SyncStatus { state: "ok", message: "Нет почтовых ящиков — добавьте в настройках".into() };
        }
        let pick = |state: &str| -> Vec<String> {
            statuses.iter().filter(|(_, s)| s.state == state).map(|(email, s)| format!("{email}: {}", s.message)).collect()
        };
        let (errors, busy) = (pick("error"), pick("busy"));
        match (errors.is_empty(), busy.is_empty()) {
            (false, _) => SyncStatus { state: "error", message: errors.join(" · ") },
            (true, false) => SyncStatus { state: "busy", message: busy.join(" · ") },
            (true, true) => SyncStatus { state: "ok", message: String::new() },
        }
    }

    /// Ящики, о которых ядро уже знает (в том числе ещё подключающиеся).
    pub fn known_accounts(&self) -> Vec<String> {
        self.statuses.lock().unwrap().keys().filter(|k| k.contains('@')).cloned().collect()
    }

    fn connection(&self, email: &str) -> Option<Arc<Connection>> {
        self.connections.lock().unwrap().get(email).cloned()
    }

    /// Регистрирует подключение; прежнее для того же ящика останавливается.
    fn register(&self, conn: Arc<Connection>) {
        if let Some(old) = self.connections.lock().unwrap().insert(conn.account.email.clone(), conn) {
            old.cancelled.store(true, Ordering::Relaxed);
        }
    }

    fn unregister(&self, email: &str) {
        if let Some(old) = self.connections.lock().unwrap().remove(email) {
            old.cancelled.store(true, Ordering::Relaxed);
        }
        self.statuses.lock().unwrap().remove(email);
    }
}

/// Для UI, который подписался на события позже, чем ядро начало синхронизацию.
#[tauri::command]
pub fn sync_status(imap: State<'_, Imap>) -> SyncStatus {
    imap.summary()
}

/// Запуск: синхронизируем все ящики из KWallet, а если их нет — показываем демо-данные.
pub async fn start(app: AppHandle) {
    let emails = match accounts::list().await {
        Ok(emails) => emails,
        Err(e) => {
            set_status(&app, "KWallet", "error", format!("нет доступа: {e}"));
            return;
        }
    };
    if emails.is_empty() {
        app.state::<Imap>().demo.store(true, Ordering::Relaxed);
        app.state::<Store>().replace_all(fake::generate(10_000));
        let _ = app.emit("mailbox-changed", ());
        let _ = app.emit("sync-status", app.state::<Imap>().summary());
        return;
    }
    for email in emails {
        match Account::new(&email) {
            Ok(account) => {
                tauri::async_runtime::spawn(sync_account(app.clone(), account));
            }
            Err(e) => set_status(&app, &email, "error", e.to_string()),
        }
    }
}

async fn sync_account(app: AppHandle, account: Account) {
    let email = account.email.clone();
    match run_sync(&app, account).await {
        Ok(()) => set_status(&app, &email, "ok", ""),
        Err(e) => set_status(&app, &email, "error", format!("ошибка синхронизации: {e}")),
    }
}

async fn run_sync(app: &AppHandle, account: Account) -> Result<()> {
    let email = account.email.clone();
    set_status(app, &email, "busy", "вход…");
    let password = accounts::password(&email).await?;
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
    let conn = Arc::new(Connection { account, folders, reader: Mutex::new(Some(reader)), cancelled: AtomicBool::new(false) });
    app.state::<Imap>().register(conn.clone());

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
        set_status(app, &email, "busy", format!("{label} {done} / {}", cursor.total));
    };
    // Сначала свежие письма всех папок, потом в фоне — всё остальное, входящие первыми.
    for cursor in &mut cursors {
        let messages = fetch_batch(&mut session, &email, cursor).await?;
        step(cursor, messages);
    }
    let poll = || Duration::from_secs(app.state::<ConfigStore>().get().poll_seconds());
    let mut last_poll = std::time::Instant::now();
    let mut last_flags = std::time::Instant::now();
    for i in 0..cursors.len() {
        while cursors[i].next > 0 {
            if conn.cancelled.load(Ordering::Relaxed) {
                let _ = session.logout().await;
                return Ok(());
            }
            let messages = fetch_batch(&mut session, &email, &mut cursors[i]).await?;
            step(&cursors[i], messages);
            // Пока догружается история, свежие письма и флаги тоже подтягиваем.
            if last_poll.elapsed() >= poll() {
                let with_flags = last_flags.elapsed() >= FLAGS_EVERY;
                refresh(app, &mut session, &email, &cursors, with_flags).await?;
                last_poll = std::time::Instant::now();
                if with_flags {
                    last_flags = last_poll;
                }
            }
        }
    }
    set_status(app, &email, "ok", "");

    // Дальше — периодическая проверка; при обрыве соединения переподключаемся.
    loop {
        while last_poll.elapsed() < poll() {
            if conn.cancelled.load(Ordering::Relaxed) {
                let _ = session.logout().await;
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let with_flags = last_flags.elapsed() >= FLAGS_EVERY;
        last_poll = std::time::Instant::now();
        if with_flags {
            last_flags = last_poll;
        }
        if let Err(e) = refresh(app, &mut session, &email, &cursors, with_flags).await {
            eprintln!("mMail: обновление {email} не удалось ({e}), переподключаюсь");
            let password = accounts::password(&email).await?;
            session = connect(&conn.account, &password).await?;
        }
    }
}

/// Новые письма (UID больше известных) и, если `with_flags`, актуальные флаги последних писем.
async fn refresh(app: &AppHandle, session: &mut Session, email: &str, cursors: &[Cursor], with_flags: bool) -> Result<()> {
    let store = app.state::<Store>();
    let mut changed = false;
    for cursor in cursors {
        let mailbox = session.examine(&cursor.name).await?;
        if mailbox.exists == 0 {
            continue;
        }
        let max = store.max_uid(email, cursor.folder);
        // Пустую папку, которую ещё грузит основной цикл, не трогаем — иначе получим дубли.
        if max > 0 || cursor.next == 0 {
            let fetches: Vec<Fetch> = session.uid_fetch(format!("{}:*", max + 1), HEADER_QUERY).await?.try_collect().await?;
            // «N:*» всегда возвращает хотя бы последнее письмо, даже если его UID меньше N.
            let fresh: Vec<Message> = fetches
                .iter()
                .filter(|f| f.uid.is_some_and(|uid| uid > max))
                .filter_map(|f| to_message(f, email, cursor.folder))
                .collect();
            if !fresh.is_empty() {
                store.append(fresh);
                changed = true;
            }
        }
        if !with_flags {
            continue;
        }
        let first = mailbox.exists.saturating_sub(FLAGS_WINDOW - 1).max(1);
        let fetches: Vec<Fetch> = session.fetch(format!("{first}:*"), "(UID FLAGS)").await?.try_collect().await?;
        let flags: Vec<_> = fetches
            .iter()
            .filter_map(|f| {
                let flags: Vec<_> = f.flags().collect();
                Some((f.uid?, !flags.contains(&Flag::Seen), flags.contains(&Flag::Flagged)))
            })
            .collect();
        changed |= store.update_flags(email, cursor.folder, &flags);
    }
    if changed {
        let _ = app.emit("mailbox-changed", ());
    }
    Ok(())
}

/// Проверка ящика перед сохранением: вход на сервер и сразу выход.
async fn verify(account: &Account, password: &str) -> Result<()> {
    let mut session = connect(account, password).await?;
    let _ = session.logout().await;
    Ok(())
}

#[tauri::command]
pub async fn list_accounts() -> std::result::Result<Vec<String>, String> {
    accounts::list().await.map_err(|e| e.to_string())
}

/// Добавляет ящик: проверяет вход, кладёт пароль в KWallet и запускает синхронизацию.
#[tauri::command]
pub async fn add_account(app: AppHandle, email: String, password: String) -> std::result::Result<String, String> {
    let password = Zeroizing::new(password);
    let account = Account::new(&email).map_err(|e| e.to_string())?;
    verify(&account, &password).await.map_err(|e| format!("не удалось войти: {e}"))?;
    accounts::save(&account, &password).await.map_err(|e| format!("не удалось сохранить в KWallet: {e}"))?;
    drop(password);

    let imap = app.state::<Imap>();
    if imap.demo.swap(false, Ordering::Relaxed) {
        app.state::<Store>().replace_all(Vec::new());
        let _ = app.emit("mailbox-changed", ());
    }
    let email = account.email.clone();
    app.state::<Store>().remove_account(&email);
    tauri::async_runtime::spawn(sync_account(app.clone(), account));
    Ok(email)
}

/// Удаляет ящик: останавливает синхронизацию, убирает письма и пароль из KWallet.
#[tauri::command]
pub async fn remove_account(app: AppHandle, email: String) -> std::result::Result<(), String> {
    let email = email.trim().to_lowercase();
    accounts::remove(&email).await.map_err(|e| e.to_string())?;
    let imap = app.state::<Imap>();
    imap.unregister(&email);
    app.state::<Store>().remove_account(&email);
    let _ = app.emit("mailbox-changed", ());
    let _ = app.emit("sync-status", imap.summary());
    Ok(())
}

impl Imap {
    /// Исходный HTML письма и проверка отправителя (BODY.PEEK — письмо не помечается прочитанным).
    pub async fn open(&self, email: &str, folder: Folder, uid: u32) -> Result<Opened> {
        let conn = self.connection(email).ok_or("ящик ещё подключается")?;
        let name = conn.folders.get(&folder).ok_or("папка не найдена на сервере")?;
        let mut reader = conn.reader.lock().await;

        // Одна попытка переподключения: соединение могло оборваться, пока приложение простаивало.
        for attempt in 0..2 {
            if reader.is_none() {
                let password = accounts::password(email).await?;
                *reader = Some(connect(&conn.account, &password).await?);
            }
            match fetch_raw(reader.as_mut().unwrap(), name, uid).await {
                Ok(raw) => {
                    let parsed = MessageParser::default().parse(&raw).ok_or("не удалось разобрать письмо")?;
                    let auth_results = parsed
                        .headers()
                        .iter()
                        .filter(|h| h.name().eq_ignore_ascii_case("Authentication-Results"))
                        .map(|h| String::from_utf8_lossy(&raw[h.offset_start as usize..h.offset_end as usize]).into_owned());
                    let auth = auth_verdict(auth_results, conn.account.provider.trusted_authserv);
                    let html = parsed.body_html(0).map(|h| h.into_owned()).unwrap_or_default();
                    let references = match parsed.references() {
                        mail_parser::HeaderValue::Text(id) => vec![id.to_string()],
                        mail_parser::HeaderValue::TextList(ids) => ids.iter().map(|id| id.to_string()).collect(),
                        _ => Vec::new(),
                    };
                    let message_id = parsed.message_id().map(str::to_string);
                    return Ok(Opened { html, auth, message_id, references });
                }
                Err(e) if attempt == 0 => {
                    eprintln!("mMail: IMAP-сессия {email} сброшена ({e}), переподключаюсь");
                    *reader = None;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }
}

impl Imap {
    /// Кладёт готовое письмо в папку ящика (копия отправленного).
    pub async fn append(&self, email: &str, folder: Folder, raw: &[u8], flags: &str) -> Result<()> {
        let conn = self.connection(email).ok_or("ящик ещё подключается")?;
        let name = conn.folders.get(&folder).ok_or("папка не найдена на сервере")?;
        let mut reader = conn.reader.lock().await;
        if reader.is_none() {
            let password = accounts::password(email).await?;
            *reader = Some(connect(&conn.account, &password).await?);
        }
        let result = reader.as_mut().unwrap().append(name, Some(flags), None, raw).await;
        if result.is_err() {
            *reader = None;
        }
        Ok(result?)
    }

    /// Помечает письмо прочитанным на сервере.
    pub async fn mark_seen(&self, email: &str, folder: Folder, uid: u32) -> Result<()> {
        let conn = self.connection(email).ok_or("ящик ещё подключается")?;
        let name = conn.folders.get(&folder).ok_or("папка не найдена на сервере")?;
        let mut reader = conn.reader.lock().await;
        if reader.is_none() {
            let password = accounts::password(email).await?;
            *reader = Some(connect(&conn.account, &password).await?);
        }
        let session = reader.as_mut().unwrap();
        let result: Result<()> = async {
            session.select(name).await?;
            let _: Vec<Fetch> = session.uid_store(uid.to_string(), "+FLAGS.SILENT (\\Seen)").await?.try_collect().await?;
            Ok(())
        }
        .await;
        if result.is_err() {
            *reader = None;
        }
        result
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
        auth_verdict(headers.iter().map(|h| h.to_string()), crate::accounts::YANDEX.trusted_authserv)
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
