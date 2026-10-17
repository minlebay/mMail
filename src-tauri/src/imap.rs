//! IMAP: TLS-подключение, фоновая синхронизация каждого ящика и загрузка писем по требованию.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_imap::types::{Fetch, Flag};
use futures::TryStreamExt;
use mail_parser::{MessageParser, MimeHeaders};
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
use crate::attachments;
use crate::config::ConfigStore;
use crate::db::Db;
use crate::fake;
use crate::folders::{self, ServerFolder};
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
/// Тела писем до этого размера сохраняются в кэш заранее, в фоне; крупные — при открытии.
const PREFETCH_MAX_BYTES: u32 = 1024 * 1024;
/// Писем за один фоновый FETCH тел.
const PREFETCH_BATCH: usize = 20;
const HEADER_QUERY: &str = "(UID FLAGS INTERNALDATE BODY.PEEK[HEADER.FIELDS (FROM SUBJECT DATE CONTENT-TYPE)])";

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

/// Все папки ящика, которые стоит синхронизировать (см. [`folders::classify`]), в порядке дерева.
async fn discover_folders(session: &mut Session) -> Result<Vec<ServerFolder>> {
    let names: Vec<_> = session.list(Some(""), Some("*")).await?.try_collect().await?;
    let mut found: Vec<ServerFolder> = Vec::new();
    for name in &names {
        let Some(folder) = folders::classify(name) else { continue };
        // Две папки с одной ролью (бывает у Gmail) — берём первую.
        if found.iter().any(|f| f.folder == folder) {
            continue;
        }
        let display = folders::display_name(name.name(), name.delimiter());
        found.push(ServerFolder { folder, server_name: name.name().to_string(), display });
    }
    if !found.iter().any(|f| f.folder == Folder::Inbox) {
        found.push(ServerFolder { folder: Folder::Inbox, server_name: "INBOX".into(), display: "INBOX".into() });
    }
    found.sort_by(|a, b| (folders::rank(a.folder), a.display.to_lowercase()).cmp(&(folders::rank(b.folder), b.display.to_lowercase())));
    Ok(found)
}

/// Синхронизация папки: письма, которые есть на сервере, но ещё нет в кэше.
struct Cursor {
    folder: Folder,
    name: String,
    /// Имя для статуса синхронизации.
    display: String,
    /// Писем в папке на сервере.
    total: usize,
    /// UID ещё не загруженных писем, от новых к старым; пусто — папка загружена целиком.
    missing: Vec<u32>,
}

async fn fetch_batch(session: &mut Session, account: &str, cursor: &mut Cursor) -> Result<Vec<Message>> {
    if cursor.missing.is_empty() {
        return Ok(Vec::new());
    }
    session.examine(&cursor.name).await?;
    let batch: Vec<u32> = cursor.missing.drain(..cursor.missing.len().min(BATCH as usize)).collect();
    let set = uid_set(&batch);
    let started = std::time::Instant::now();
    // Только нужные поля заголовка — так же синхронизирует Thunderbird.
    let fetches: Vec<Fetch> = session.uid_fetch(&set, HEADER_QUERY).await?.try_collect().await?;
    eprintln!("mMail: {} {} писем за {:?}", cursor.name, batch.len(), started.elapsed());
    Ok(fetches.iter().filter_map(|f| to_message(f, account, cursor.folder)).collect())
}

/// Набор UID для команды IMAP: подряд идущие сворачиваются в диапазоны («1:3,7,9:10»).
fn uid_set(uids: &[u32]) -> String {
    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < sorted.len() {
        let start = sorted[i];
        while i + 1 < sorted.len() && sorted[i + 1] == sorted[i] + 1 {
            i += 1;
        }
        parts.push(if sorted[i] == start { start.to_string() } else { format!("{start}:{}", sorted[i]) });
        i += 1;
    }
    parts.join(",")
}

/// Открывает шифрованный кэш (ключ — из KWallet) и сразу показывает сохранённые письма.
async fn open_cache(app: &AppHandle) -> Result<()> {
    if app.state::<Store>().has_db() {
        return Ok(());
    }
    let key = accounts::db_key().await?;
    let dir = app.path().app_data_dir()?;
    let db = tauri::async_runtime::spawn_blocking(move || Db::open(&dir, &key)).await??;
    app.state::<Store>().attach_db(db)?;
    let _ = app.emit("mailbox-changed", ());
    // Содержимое писем старше срока хранения (его могли открыть в прошлый раз) — убираем.
    if let Some(cutoff) = app.state::<ConfigStore>().get().body_cutoff() {
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(e) = handle.state::<Store>().clear_bodies(cutoff) {
                eprintln!("mMail: очистка кэша не удалась: {e}");
            }
        });
    }
    Ok(())
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
        // multipart/mixed почти всегда означает вложения; точно узнаем при открытии.
        has_attachments: parsed.content_type().is_some_and(|ct| {
            ct.ctype().eq_ignore_ascii_case("multipart") && ct.subtype().is_some_and(|s| s.eq_ignore_ascii_case("mixed"))
        }),
        attachments: Vec::new(),
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
    /// Папки в порядке дерева, с именами для людей.
    list: Vec<ServerFolder>,
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
        let locked = pick("locked");
        if !locked.is_empty() {
            return SyncStatus { state: "locked", message: locked.join(" · ") };
        }
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

    /// Папки подключённого ящика для дерева; `None`, пока ящик не подключился.
    pub fn folders(&self, email: &str) -> Option<Vec<ServerFolder>> {
        self.connection(email).map(|c| c.list.clone())
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

const STORE_STATUS: &str = "Хранилище";

/// Повторная попытка открыть хранилище (например, после отказа в окне KWallet).
#[tauri::command]
pub async fn unlock_store(app: AppHandle) {
    set_status(&app, STORE_STATUS, "busy", "открываю…");
    app.state::<Imap>().statuses.lock().unwrap().remove(STORE_STATUS);
    start(app).await;
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
    // Без кэша не синхронизируем: если KWallet закрыт, паролей ящиков всё равно не получить.
    if let Err(e) = open_cache(&app).await {
        set_status(&app, STORE_STATUS, "locked", format!("закрыто: {e}"));
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

    let store = app.state::<Store>();
    let list = discover_folders(&mut session).await?;
    let folders: HashMap<Folder, String> = list.iter().map(|f| (f.folder, f.server_name.clone())).collect();
    let mut cursors = Vec::new();
    for ServerFolder { folder, server_name: name, display } in &list {
        let (folder, name) = (*folder, name);
        let mailbox = match session.examine(name).await {
            Ok(mailbox) => mailbox,
            // Папка могла исчезнуть между LIST и EXAMINE или быть недоступной — остальные не ждут.
            Err(e) => {
                eprintln!("mMail: папка {display} недоступна: {e}");
                continue;
            }
        };
        store.check_validity(&email, folder, mailbox.uid_validity.unwrap_or_default());
        // Список UID папки — дёшево даже для десятков тысяч писем; сверяем с кэшем.
        let on_server: HashSet<u32> = if mailbox.exists == 0 { HashSet::new() } else { session.uid_search("ALL").await? };
        if store.retain_on_server(&email, folder, &on_server) {
            let _ = app.emit("mailbox-changed", ());
        }
        let known = store.uids(&email, folder);
        let mut missing: Vec<u32> = on_server.iter().copied().filter(|uid| !known.contains(uid)).collect();
        missing.sort_unstable_by(|a, b| b.cmp(a));
        cursors.push(Cursor { folder, name: name.clone(), display: display.clone(), total: on_server.len(), missing });
    }
    let conn = Arc::new(Connection { account, folders, list, reader: Mutex::new(Some(reader)), cancelled: AtomicBool::new(false) });
    app.state::<Imap>().register(conn.clone());

    let step = |cursor: &Cursor, messages: Vec<Message>| {
        if !messages.is_empty() {
            store.append(messages);
            let _ = app.emit("mailbox-changed", ());
        }
        let done = cursor.total - cursor.missing.len();
        let label = match folders::special_name(cursor.folder) {
            Some(name) => crate::i18n::text(app, name),
            None => cursor.display.clone(),
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
        while !cursors[i].missing.is_empty() {
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
    // Между проверками, пока есть что, сохраняем тела писем для чтения без сети.
    let mut prefetching = store.has_db();
    let mut skipped = HashSet::new();
    loop {
        while last_poll.elapsed() < poll() {
            if conn.cancelled.load(Ordering::Relaxed) {
                let _ = session.logout().await;
                return Ok(());
            }
            if !prefetching {
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            }
            match prefetch_bodies(app, &mut session, &conn.account, &cursors, &mut skipped).await {
                Ok(0) => {
                    prefetching = false;
                    set_status(app, &email, "ok", "");
                }
                Ok(left) => set_status(app, &email, "busy", format!("сохраняю письма: осталось {left}")),
                Err(e) => {
                    // Сессия могла оборваться — переподключится обновление ниже.
                    eprintln!("mMail: загрузка тел {email} не удалась: {e}");
                    break;
                }
            }
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

/// Следующая порция тел писем, которых ещё нет в кэше (новые первыми). Возвращает, сколько осталось.
async fn prefetch_bodies(
    app: &AppHandle,
    session: &mut Session,
    account: &Account,
    cursors: &[Cursor],
    skipped: &mut HashSet<(Folder, u32)>,
) -> Result<usize> {
    let store = app.state::<Store>();
    let since = app.state::<ConfigStore>().get().body_cutoff();
    let pending = |c: &Cursor, skipped: &HashSet<(Folder, u32)>| -> Vec<u32> {
        store.unopened(&account.email, c.folder, since).into_iter().filter(|uid| !skipped.contains(&(c.folder, *uid))).collect()
    };
    let Some((cursor, uids)) = cursors.iter().map(|c| (c, pending(c, skipped))).find(|(_, uids)| !uids.is_empty()) else {
        return Ok(0);
    };
    let batch = &uids[..uids.len().min(PREFETCH_BATCH)];
    // Порцию отмечаем заранее: письмо, на котором сервер или разбор ломается, не будет повторяться по кругу.
    skipped.extend(batch.iter().map(|uid| (cursor.folder, *uid)));

    session.examine(&cursor.name).await?;
    let sizes: Vec<Fetch> = session.uid_fetch(uid_set(batch), "(UID RFC822.SIZE)").await?.try_collect().await?;
    let small: Vec<u32> = sizes.iter().filter(|f| f.size.is_some_and(|s| s <= PREFETCH_MAX_BYTES)).filter_map(|f| f.uid).collect();
    if !small.is_empty() {
        let fetches: Vec<Fetch> = session.uid_fetch(uid_set(&small), "(UID BODY.PEEK[])").await?.try_collect().await?;
        let mut changed = false;
        for f in &fetches {
            let (Some(uid), Some(raw)) = (f.uid, f.body()) else { continue };
            match parse_opened(raw, account.provider) {
                Ok(opened) => changed |= store.set_opened_uid(&account.email, cursor.folder, uid, opened),
                Err(e) => eprintln!("mMail: письмо {uid} не разобрано: {e}"),
            }
        }
        if changed {
            let _ = app.emit("mailbox-changed", ());
        }
    }
    Ok(cursors.iter().map(|c| pending(c, skipped).len()).sum())
}

/// Новые письма (UID больше известных) и, если `with_flags`, актуальные флаги последних писем.
async fn refresh(app: &AppHandle, session: &mut Session, email: &str, cursors: &[Cursor], with_flags: bool) -> Result<()> {
    let store = app.state::<Store>();
    let mut changed = false;
    for cursor in cursors {
        // Свои папки (не стандартные) проверяем реже — вместе с флагами, раз в минуту:
        // их может быть много, а EXAMINE каждой — лишний запрос на каждом опросе.
        if matches!(cursor.folder, Folder::Other(_)) && !with_flags {
            continue;
        }
        let mailbox = session.examine(&cursor.name).await?;
        if mailbox.exists == 0 {
            continue;
        }
        let max = store.max_uid(email, cursor.folder);
        // Пустую папку, которую ещё грузит основной цикл, не трогаем — иначе получим дубли.
        if max > 0 || cursor.missing.is_empty() {
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
    // Первый ящик (или хранилище не открылось при запуске): без кэша почта работает, но не сохраняется.
    if let Err(e) = open_cache(&app).await {
        set_status(&app, STORE_STATUS, "error", format!("не открылось, письма не сохраняются: {e}"));
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
    /// Письмо целиком с сервера (BODY.PEEK — не помечается прочитанным); одна попытка переподключения.
    pub async fn raw(&self, email: &str, folder: Folder, uid: u32) -> Result<Vec<u8>> {
        let conn = self.connection(email).ok_or("ящик ещё подключается")?;
        let name = conn.folders.get(&folder).ok_or("папка не найдена на сервере")?;
        let mut reader = conn.reader.lock().await;
        for attempt in 0..2 {
            if reader.is_none() {
                let password = accounts::password(email).await?;
                *reader = Some(connect(&conn.account, &password).await?);
            }
            match fetch_raw(reader.as_mut().unwrap(), name, uid).await {
                Ok(raw) => return Ok(raw),
                Err(e) if attempt == 0 => {
                    eprintln!("mMail: IMAP-сессия {email} сброшена ({e}), переподключаюсь");
                    *reader = None;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }

    /// Тело письма, проверка отправителя, идентификаторы цепочки и список вложений.
    pub async fn open(&self, email: &str, folder: Folder, uid: u32) -> Result<Opened> {
        let raw = self.raw(email, folder, uid).await?;
        let provider = self.connection(email).ok_or("ящик отключён")?.account.provider;
        parse_opened(&raw, provider)
    }
}

/// Тело письма, проверка отправителя, идентификаторы цепочки и список вложений (без их содержимого).
fn parse_opened(raw: &[u8], provider: &accounts::Provider) -> Result<Opened> {
    let parsed = MessageParser::default().parse(raw).ok_or("не удалось разобрать письмо")?;
    let auth_results = parsed
        .headers()
        .iter()
        .filter(|h| h.name().eq_ignore_ascii_case("Authentication-Results"))
        .map(|h| String::from_utf8_lossy(&raw[h.offset_start as usize..h.offset_end as usize]).into_owned());
    let auth = auth_verdict(auth_results, provider.trusted_authserv);
    let html = parsed.body_html(0).map(|h| attachments::inline_cid_images(&h, &parsed)).unwrap_or_default();
    let references = match parsed.references() {
        mail_parser::HeaderValue::Text(id) => vec![id.to_string()],
        mail_parser::HeaderValue::TextList(ids) => ids.iter().map(|id| id.to_string()).collect(),
        _ => Vec::new(),
    };
    Ok(Opened {
        html,
        auth,
        message_id: parsed.message_id().map(str::to_string),
        references,
        attachments: attachments::describe(&parsed),
    })
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

    /// Atomic move of one UID; never expunge unrelated messages.
    pub async fn move_to_trash(&self, email: &str, folder: Folder, uid: u32) -> Result<()> {
        if folder == Folder::Trash { return Ok(()); }
        let conn = self.connection(email).ok_or("ящик ещё подключается")?;
        let source = conn.folders.get(&folder).ok_or("папка не найдена на сервере")?;
        let target = conn.folders.get(&Folder::Trash).ok_or("Корзина не найдена на сервере")?;
        if source == target { return Err("Письмо уже в корзине".into()); }
        let mut reader = conn.reader.lock().await;
        if reader.is_none() {
            let password = accounts::password(email).await?;
            *reader = Some(connect(&conn.account, &password).await?);
        }
        let session = reader.as_mut().unwrap();
        let result: Result<()> = async {
            if !session.capabilities().await?.has_str("MOVE") {
                return Err("Сервер не поддерживает безопасный перенос письма".into());
            }
            session.select(source).await?;
            session.uid_mv(uid.to_string(), target).await?;
            Ok(())
        }.await;
        if result.is_err() { *reader = None; }
        result
    }

    /// Помечает письмо прочитанным на сервере.
    pub async fn mark_seen(&self, email: &str, folder: Folder, uid: u32) -> Result<()> {
        self.mark_seen_many(email, folder, &[uid]).await
    }

    /// Помечает прочитанными письма одной папки: одна команда STORE на порцию UID.
    pub async fn mark_seen_many(&self, email: &str, folder: Folder, uids: &[u32]) -> Result<()> {
        self.set_flag(email, folder, uids, "\\Seen", true).await
    }

    /// Ставит (`add`) или снимает системный флаг (`\Seen`, `\Flagged`) у писем одной папки.
    pub async fn set_flag(&self, email: &str, folder: Folder, uids: &[u32], flag: &str, add: bool) -> Result<()> {
        if !matches!(flag, "\\Seen" | "\\Flagged") {
            return Err("недопустимый флаг".into());
        }
        if uids.is_empty() {
            return Ok(());
        }
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
            for chunk in uids.chunks(500) {
                let query = format!("{}FLAGS.SILENT ({flag})", if add { '+' } else { '-' });
                let _: Vec<Fetch> = session.uid_store(uid_set(chunk), query).await?.try_collect().await?;
            }
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
    fn uid_sets_collapse_runs() {
        assert_eq!(uid_set(&[10, 9, 3, 2, 1, 7]), "1:3,7,9:10");
        assert_eq!(uid_set(&[5]), "5");
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
