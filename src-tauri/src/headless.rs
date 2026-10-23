//! Чтение почты из командной строки — без графики и без записи в кэш.
//!
//!   mmail accounts [--json]
//!   mmail folders [--account a@b.c] [--json]
//!   mmail list [--account a@b.c] [--folder inbox] [--unread] [--limit N] [--offset N] [--json]
//!   mmail search [--account a@b.c] [--folder inbox] [--limit N] [--json] запрос…
//!   mmail read ID [--html] [--json]
//!
//! Команды обслуживаются до запуска Tauri: окно не открывается, второй экземпляр не
//! поднимается. Кэш писем открывается отдельным соединением только на чтение: файл не
//! создаётся, не мигрирует и не пересоздаётся (в отличие от `db::Db::open`), ключ берётся
//! из KWallet готовый — новый не создаётся. Свежесть писем — та, что успел сохранить
//! работающий mMail.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use zeroize::Zeroizing;

use crate::accounts;
use crate::store::{date_label, Folder, Mailbox, Recipients};

/// Идентификатор приложения Tauri: от него Tauri строит каталог данных
/// (`app.path().app_data_dir()`), а в headless-режиме AppHandle недоступен.
const APP_ID: &str = "dev.minlebay.mmail";

const USAGE: &str = "\
Команды чтения (графика не запускается, кэш писем только читается):

  mmail accounts [--json]
  mmail folders [--account ЯЩИК] [--json]
  mmail list [--account ЯЩИК] [--folder ПАПКА] [--unread] [--limit N] [--offset N] [--json]
  mmail search [--account ЯЩИК] [--folder ПАПКА] [--limit N] [--json] ЗАПРОС…
  mmail read ID [--html] [--json]

ПАПКА — inbox, sent, drafts, trash, junk, archive или имя папки на сервере.
ID — как в list/search: ящик:папка:uid (копируется целиком). Тело письма есть в кэше
только у писем, которые уже открывали в mMail; у остальных в ответе будет body: null.
";

/// Разобранные аргументы команд чтения.
struct Opts {
    json: bool,
    html: bool,
    account: Option<String>,
    folder: Option<String>,
    unread: bool,
    limit: usize,
    offset: usize,
    words: Vec<String>,
}

impl Opts {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut o = Opts { json: false, html: false, account: None, folder: None, unread: false, limit: 20, offset: 0, words: Vec::new() };
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            let (flag, inline) = match arg.split_once('=') {
                Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
                _ => (arg.as_str(), None),
            };
            let mut value = |name: &str| -> Result<String, String> {
                inline.clone().or_else(|| it.next().cloned()).ok_or_else(|| format!("{name}: нужно значение"))
            };
            match flag {
                "--json" => o.json = true,
                "--html" => o.html = true,
                "--unread" => o.unread = true,
                "--account" => o.account = Some(value("--account")?),
                "--folder" => o.folder = Some(folder_key(&value("--folder")?)),
                "--limit" => {
                    o.limit = value("--limit")?.parse().map_err(|_| "--limit: нужно число")?;
                    o.limit = o.limit.clamp(1, 500);
                }
                "--offset" => o.offset = value("--offset")?.parse().map_err(|_| "--offset: нужно число")?,
                "--help" | "-h" => return Err("help".into()),
                _ if flag.starts_with("--") => return Err(format!("неизвестный параметр: {flag}")),
                _ => o.words.push(arg.clone()),
            }
        }
        Ok(o)
    }
}

/// `Some(код)` — команду обслужили здесь; `None` — обычный запуск mMail (окно, `-compose`).
pub fn run(argv: &[String]) -> Option<i32> {
    let args = argv.get(1..).unwrap_or_default();
    let (cmd, rest) = args.split_first()?;
    if matches!(cmd.as_str(), "--help" | "-h" | "help") {
        print!("{USAGE}");
        return Some(0);
    }
    let command = match cmd.as_str() {
        "accounts" | "folders" | "list" | "search" | "read" => cmd.as_str(),
        _ => return None,
    };
    let opts = match Opts::parse(rest) {
        Ok(o) => o,
        Err(e) if e == "help" => {
            print!("{USAGE}");
            return Some(0);
        }
        Err(e) => return Some(fail(&e)),
    };
    let result = match command {
        "accounts" => cmd_accounts(&opts),
        "folders" => cmd_folders(&opts),
        "list" => cmd_list(&opts),
        "search" => cmd_search(&opts),
        "read" => cmd_read(&opts),
        _ => unreachable!(),
    };
    Some(match result {
        Ok(()) => 0,
        Err(e) => fail(&e),
    })
}

fn fail(message: &str) -> i32 {
    eprintln!("mmail: {message}");
    2
}

fn cmd_accounts(o: &Opts) -> Result<(), String> {
    let accounts = runtime()?.block_on(accounts::list()).map_err(|e| e.to_string())?;
    if o.json {
        return print_json(&accounts);
    }
    for account in accounts {
        println!("{account}");
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FolderRow {
    account: String,
    /// Ключ в кэше: `inbox`, `sent`, … или `f:<имя на сервере>`.
    folder: String,
    name: String,
    messages: i64,
    unread: i64,
}

fn cmd_folders(o: &Opts) -> Result<(), String> {
    let conn = open_cache()?;
    let mut stmt = conn
        .prepare("SELECT account, folder, count(*), sum(unread) FROM messages GROUP BY account, folder")
        .map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query_map([], |r| {
            Ok(FolderRow {
                account: r.get(0)?,
                folder: r.get(1)?,
                name: String::new(),
                messages: r.get(2)?,
                unread: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    for row in &mut rows {
        row.name = folder_name(&row.folder);
    }
    rows.retain(|r| o.account.as_ref().is_none_or(|a| &r.account == a));
    rows.sort_by(|a, b| {
        let rank = |r: &FolderRow| Folder::from_key(&r.folder).map(crate::folders::rank).unwrap_or(u8::MAX);
        a.account.cmp(&b.account).then(rank(a).cmp(&rank(b))).then(a.name.cmp(&b.name))
    });
    if o.json {
        return print_json(&rows);
    }
    for row in rows {
        println!("{}\t{}\t{}\t{}\t{}", row.account, row.folder, row.name, row.messages, row.unread);
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    /// Ключ письма `ящик:папка:uid` — годится для `mmail read`.
    id: String,
    account: String,
    folder: String,
    uid: i64,
    unread: bool,
    starred: bool,
    from_name: String,
    from_addr: String,
    to: Vec<Mailbox>,
    subject: String,
    date: String,
    timestamp: i64,
    has_attachments: bool,
    /// Сервер не отдал заголовок — в списке заглушка.
    unavailable: bool,
}

const LIST_SQL: &str = "SELECT account, folder, uid, unread, starred, from_name, from_addr, subject,
        timestamp, has_attachments, recipients, unavailable
     FROM messages
     WHERE (?1 IS NULL OR account = ?1)
       AND (?2 IS NULL OR folder = ?2)
       AND (?3 = 0 OR unread = 1)
     ORDER BY timestamp DESC, account, folder, uid";

fn cmd_list(o: &Opts) -> Result<(), String> {
    let conn = open_cache()?;
    let rows = select_rows(&conn, o, "")?;
    print_rows(o, &rows)
}

fn cmd_search(o: &Opts) -> Result<(), String> {
    let needle = o.words.join(" ").trim().to_lowercase();
    if needle.is_empty() {
        return Err("search: нужен запрос".into());
    }
    let conn = open_cache()?;
    let rows = select_rows(&conn, o, &needle)?;
    print_rows(o, &rows)
}

fn print_rows(o: &Opts, rows: &[Row]) -> Result<(), String> {
    if o.json {
        return print_json(rows);
    }
    for row in rows {
        let from = if row.from_name.is_empty() {
            row.from_addr.clone()
        } else {
            format!("{} <{}>", row.from_name, row.from_addr)
        };
        let mark = if row.unread { '•' } else { ' ' };
        println!("{mark}{}\t{}\t{}\t{}\t{}", row.id, row.date, row.folder, from, row.subject);
    }
    Ok(())
}

/// Подстрока по отправителю, получателям и теме — как поиск в окне mMail (тело в кэше
/// есть не у всех писем, поэтому тела поиск не трогает). Фильтр по запросу — в Rust:
/// `lower()` в SQLite не знает кириллицу.
fn select_rows(conn: &Connection, o: &Opts, needle: &str) -> Result<Vec<Row>, String> {
    let mut stmt = conn.prepare(LIST_SQL).map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query(rusqlite::params![o.account, o.folder, i64::from(o.unread)])
        .map_err(|e| e.to_string())?;
    let mut found = Vec::new();
    let mut skipped = 0usize;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let recipients: Option<String> = row.get(10).map_err(|e| e.to_string())?;
        let account: String = row.get(0).map_err(|e| e.to_string())?;
        let folder: String = row.get(1).map_err(|e| e.to_string())?;
        let uid: i64 = row.get(2).map_err(|e| e.to_string())?;
        let timestamp: i64 = row.get(8).map_err(|e| e.to_string())?;
        let message: Row = Row {
            id: make_id(&account, &folder, uid),
            account,
            folder,
            uid,
            unread: row.get(3).map_err(|e| e.to_string())?,
            starred: row.get(4).map_err(|e| e.to_string())?,
            from_name: row.get(5).map_err(|e| e.to_string())?,
            from_addr: row.get(6).map_err(|e| e.to_string())?,
            to: recipients.as_deref().and_then(|j| serde_json::from_str::<Recipients>(j).ok()).map(|r| r.to).unwrap_or_default(),
            subject: row.get(7).map_err(|e| e.to_string())?,
            timestamp,
            date: date_label(timestamp),
            has_attachments: row.get(9).map_err(|e| e.to_string())?,
            unavailable: row.get(11).map_err(|e| e.to_string())?,
        };
        if !needle.is_empty() && !haystack(&message).contains(needle) {
            continue;
        }
        if skipped < o.offset {
            skipped += 1;
            continue;
        }
        found.push(message);
        if found.len() >= o.limit {
            break;
        }
    }
    Ok(found)
}

fn haystack(row: &Row) -> String {
    let to = row.to.iter().map(|a| format!("{} {}", a.name, a.addr)).collect::<Vec<_>>().join(" ");
    format!("{} {} {to} {}", row.from_name, row.from_addr, row.subject).to_lowercase()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Details {
    id: String,
    account: String,
    folder: String,
    uid: i64,
    unread: bool,
    starred: bool,
    from_name: String,
    from_addr: String,
    to: Vec<Mailbox>,
    cc: Vec<Mailbox>,
    subject: String,
    date: String,
    timestamp: i64,
    has_attachments: bool,
    attachments: serde_json::Value,
    /// Текст письма (HTML без разметки); `null` — тела в кэше нет.
    body: Option<String>,
    /// Исходный HTML из кэша — только с `--html` (не очищен, как и в кэше).
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<String>,
}

fn cmd_read(o: &Opts) -> Result<(), String> {
    let raw = o.words.first().ok_or("read: нужен ID письма (из list или search)")?;
    let (account, folder, uid) = parse_id(raw).ok_or("read: ID как в list/search — ящик:папка:uid")?;
    let conn = open_cache()?;
    let row = conn
        .query_row(
            "SELECT account, folder, uid, unread, starred, from_name, from_addr, subject, timestamp,
                    has_attachments, recipients, unavailable, html, attachments
             FROM messages WHERE account = ?1 AND folder = ?2 AND uid = ?3",
            rusqlite::params![account, folder, uid],
            |r| {
                let recipients: Option<String> = r.get(10)?;
                let recipients = recipients.as_deref().and_then(|j| serde_json::from_str::<Recipients>(j).ok()).unwrap_or_default();
                let html: Option<String> = r.get(12)?;
                let account: String = r.get(0)?;
                let folder: String = r.get(1)?;
                let uid: i64 = r.get(2)?;
                let timestamp: i64 = r.get(8)?;
                Ok(Details {
                    id: make_id(&account, &folder, uid),
                    account,
                    folder,
                    uid,
                    unread: r.get(3)?,
                    starred: r.get(4)?,
                    from_name: r.get(5)?,
                    from_addr: r.get(6)?,
                    to: recipients.to,
                    cc: recipients.cc,
                    subject: r.get(7)?,
                    timestamp,
                    date: date_label(timestamp),
                    has_attachments: r.get(9)?,
                    attachments: r
                        .get::<_, Option<String>>(13)?
                        .as_deref()
                        .and_then(|j| serde_json::from_str(j).ok())
                        .unwrap_or_else(|| serde_json::json!([])),
                    body: html.as_deref().map(html_to_text),
                    html: if o.html { html } else { None },
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("письмо {raw} не найдено в кэше"))?;
    if o.json {
        return print_json(&row);
    }
    println!("ID: {}", row.id);
    println!("Ящик: {} / {}", row.account, folder_name(&row.folder));
    println!("От: {} <{}>", row.from_name, row.from_addr);
    if !row.to.is_empty() {
        println!("Кому: {}", row.to.iter().map(|a| format!("{} <{}>", a.name, a.addr)).collect::<Vec<_>>().join(", "));
    }
    if !row.cc.is_empty() {
        println!("Копия: {}", row.cc.iter().map(|a| format!("{} <{}>", a.name, a.addr)).collect::<Vec<_>>().join(", "));
    }
    println!("Дата: {}", row.date);
    println!("Тема: {}", row.subject);
    if let Some(items) = row.attachments.as_array() {
        if !items.is_empty() {
            let names = items
                .iter()
                .filter_map(|a| a.get("name").and_then(|n| n.as_str()).map(|n| n.to_string()))
                .collect::<Vec<_>>()
                .join(", ");
            println!("Вложения: {names}");
        }
    }
    println!();
    match &row.body {
        Some(body) if !body.is_empty() => println!("{body}"),
        _ => println!("(тела нет в кэше — открой письмо в mMail)"),
    }
    Ok(())
}

fn print_json<T: Serialize + ?Sized>(value: &T) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    println!("{text}");
    Ok(())
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())
}

fn data_dir() -> PathBuf {
    match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir).join(APP_ID),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share").join(APP_ID),
    }
}

/// Кэш писем, открытый только на чтение. Ключ — готовый из KWallet; файл не создаётся,
/// схема не мигрирует, при неверном ключе кэш не пересоздаётся (этим занимается только GUI).
fn open_cache() -> Result<Connection, String> {
    let path = data_dir().join("mail.db");
    if !path.is_file() {
        return Err(format!("кэш писем не найден: {} — запусти mMail", path.display()));
    }
    let key = runtime()?
        .block_on(accounts::existing_db_key())
        .map_err(|e| e.to_string())?
        .ok_or("в KWallet нет ключа кэша — запусти mMail хотя бы раз")?;
    open_read_only(&path, &key)
}

/// Отдельное соединение с кэшем: сначала строго READ_ONLY, без CREATE и без миграций.
fn open_read_only(path: &Path, key: &[u8; 32]) -> Result<Connection, String> {
    match connect(path, key, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(conn) => Ok(conn),
        Err(read_only) => {
            // WAL: соединению только на чтение нужен готовый -shm. Если приложение закрыто
            // и журнал свёрнут, открываем тот же файл без CREATE и запрещаем запись на
            // уровне SQLite — данные не меняются, файл не пересоздаётся.
            connect(path, key, OpenFlags::SQLITE_OPEN_READ_WRITE)
                .and_then(|conn| {
                    conn.execute_batch("PRAGMA query_only = ON;")?;
                    Ok(conn)
                })
                .map_err(|read_write| format!("кэш не открывается на чтение ({read_only}); и без записи ({read_write})"))
        }
    }
}

fn connect(path: &Path, key: &[u8; 32], flags: OpenFlags) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(path, flags)?;
    conn.busy_timeout(Duration::from_secs(3))?;
    let hex = Zeroizing::new(key.iter().map(|b| format!("{b:02x}")).collect::<String>());
    // Сырой ключ (x'…'), без PBKDF2 — как в db.rs.
    conn.execute_batch(&Zeroizing::new(format!("PRAGMA key = \"x'{}'\";", hex.as_str())))?;
    // Неверный ключ обнаруживается при первом чтении.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))?;
    Ok(conn)
}

/// `all`/пусто — без фильтра; стандартные ключи как есть; иначе папка на сервере (`f:<имя>`).
fn folder_key(value: &str) -> String {
    let value = value.trim();
    match value {
        "" | "all" => String::new(),
        "inbox" | "sent" | "drafts" | "trash" | "junk" | "archive" => value.to_string(),
        other if other.starts_with("f:") => other.to_string(),
        other => format!("f:{other}"),
    }
}

/// В ID экранируем только `:` и `%`: `a@b.c:inbox:123` читается глазами и однозначно
/// разбирается, а ключ папки вида `f:Работа` не ломает разделитель. Кириллицу не трогаем —
/// ID должен оставаться читаемым.
fn make_id(account: &str, folder: &str, uid: i64) -> String {
    format!("{}:{}:{}", escape(account), escape(folder), uid)
}

fn escape(part: &str) -> String {
    part.replace('%', "%25").replace(':', "%3A")
}

fn unescape(part: &str) -> Option<String> {
    let bytes = part.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| char::from(b).to_digit(16);
            if let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((high * 16 + low) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

fn parse_id(id: &str) -> Option<(String, String, i64)> {
    let mut parts = id.split(':');
    let account = unescape(parts.next()?)?;
    let folder = unescape(parts.next()?)?;
    let uid = parts.next()?.parse().ok()?;
    if parts.next().is_some() || account.is_empty() || folder.is_empty() {
        return None;
    }
    Some((account, folder, uid))
}

fn folder_name(key: &str) -> String {
    match Folder::from_key(key) {
        Some(Folder::Other(server)) => crate::folders::decode_mutf7(server),
        Some(folder) => crate::folders::special_name(folder).unwrap_or(key).to_string(),
        None => key.to_string(),
    }
}

/// HTML письма в читаемый текст: без скриптов и стилей, блочные теги — с новой строки,
/// сущности раскрыты. Полноценный HTML-парсер здесь не нужен: это подсказка для чтения,
/// а точный вид — в окне mMail.
fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut skip: Option<&'static str> = None;
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        if skip.is_none() {
            push_text(&mut out, &rest[..lt]);
        }
        rest = &rest[lt..];
        let Some(gt) = rest.find('>') else {
            push_text(&mut out, rest);
            rest = "";
            break;
        };
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if let Some(skipped) = skip {
            if tag.starts_with('/') && name == skipped {
                skip = None;
            }
            continue;
        }
        if !tag.starts_with('/') {
            match name.as_str() {
                "script" => skip = Some("script"),
                "style" => skip = Some("style"),
                "head" => skip = Some("head"),
                "title" => skip = Some("title"),
                _ => {}
            }
            if skip.is_some() {
                continue;
            }
        }
        if matches!(
            name.as_str(),
            "br" | "p" | "div" | "tr" | "td" | "th" | "li" | "ul" | "ol" | "table" | "blockquote" | "hr"
                | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "section" | "article" | "header" | "footer" | "pre"
        ) {
            push_newline(&mut out);
        }
    }
    if skip.is_none() {
        push_text(&mut out, rest);
    }
    out.trim_end().to_string()
}

fn push_text(out: &mut String, text: &str) {
    for ch in decode_entities(text).chars() {
        if ch.is_whitespace() {
            if !out.is_empty() && !out.ends_with(' ') && !out.ends_with('\n') {
                out.push(' ');
            }
        } else {
            out.push(ch);
        }
    }
}

fn push_newline(out: &mut String) {
    while out.ends_with(' ') {
        out.pop();
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
}

fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        let Some(semi) = tail.find(';').filter(|&i| i <= 12) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let decoded = match &tail[1..semi] {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "laquo" => Some('«'),
            "raquo" => Some('»'),
            "hellip" => Some('…'),
            "copy" => Some('©'),
            "reg" => Some('®'),
            "times" => Some('×'),
            "middot" => Some('·'),
            numeric => numeric
                .strip_prefix("#x")
                .or_else(|| numeric.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| numeric.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(ch) => {
                out.push(ch);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(args: &[&str]) -> Result<Opts, String> {
        Opts::parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mmail-headless-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn sample_message() -> crate::store::Message {
        crate::store::Message {
            account: Some("me@ya.ru".into()),
            folder: Folder::Inbox,
            uid: Some(7),
            starred: false,
            unread: true,
            from_name: "Иван".into(),
            from_addr: "ivan@example.com".into(),
            recipients: None,
            subject: "Счёт".into(),
            timestamp: 1_700_000_000,
            auth: crate::store::Auth::Unknown,
            html: Some("<p>Привет</p>".into()),
            message_id: None,
            references: Vec::new(),
            has_attachments: false,
            attachments: Vec::new(),
            unavailable: false,
        }
    }

    /// Кэш, созданный приложением, читается отдельным соединением, и это соединение
    /// не может ничего записать.
    #[test]
    fn read_only_connection_reads_and_cannot_write() {
        let dir = temp_dir("read-only");
        let key = [7u8; 32];
        {
            let db = crate::db::Db::open(&dir, &key).unwrap();
            db.insert(&[&sample_message()]).unwrap();
        }
        let path = dir.join("mail.db");
        {
            let conn = open_read_only(&path, &key).unwrap();
            let subject: String = conn.query_row("SELECT subject FROM messages", [], |r| r.get(0)).unwrap();
            assert_eq!(subject, "Счёт");
            assert!(conn.execute("DELETE FROM messages", []).is_err(), "read-only соединение записало в кэш");
        }
        // Кэш на месте и по-прежнему открывается настоящим ключом.
        assert!(path.is_file());
        let conn = open_read_only(&path, &key).unwrap();
        let count: i64 = conn.query_row("SELECT count(*) FROM messages", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Неверный ключ — ошибка, а не «создам кэш заново», как сделал бы GUI.
    #[test]
    fn wrong_key_does_not_touch_the_cache() {
        let dir = temp_dir("wrong-key");
        let key = [7u8; 32];
        {
            let db = crate::db::Db::open(&dir, &key).unwrap();
            db.insert(&[&sample_message()]).unwrap();
        }
        let path = dir.join("mail.db");
        assert!(open_read_only(&path, &[9u8; 32]).is_err());
        assert!(path.is_file(), "кэш исчез при неверном ключе");
        let conn = open_read_only(&path, &key).unwrap();
        let subject: String = conn.query_row("SELECT subject FROM messages", [], |r| r.get(0)).unwrap();
        assert_eq!(subject, "Счёт");
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_arguments() {
        let o = opts(&["--json", "--limit", "5", "--unread", "42"]).unwrap();
        assert!(o.json && o.unread);
        assert_eq!(o.limit, 5);
        assert_eq!(o.words, vec!["42"]);
        assert_eq!(opts(&["--account", "a@b.c"]).unwrap().account.unwrap(), "a@b.c");
        assert_eq!(opts(&["--limit=7"]).unwrap().limit, 7);
        assert!(opts(&["--nope"]).is_err());
        assert_eq!(opts(&[]).unwrap().limit, 20);
    }

    #[test]
    fn folders_are_keys_or_server_names() {
        assert_eq!(folder_key("inbox"), "inbox");
        assert_eq!(folder_key("all"), "");
        assert_eq!(folder_key("Работа/2026"), "f:Работа/2026");
        assert_eq!(folder_key("f:Работа"), "f:Работа");
        assert_eq!(folder_name("f:Работа"), "Работа");
        assert_eq!(folder_name("f:&BCIENQRBBEI-"), "Тест");
        assert_eq!(folder_name("sent"), "Отправленные");
    }

    #[test]
    fn html_becomes_text() {
        assert_eq!(html_to_text("<p>Привет, <b>мир</b>!</p><p>Второй</p>"), "Привет, мир!\nВторой");
        assert_eq!(html_to_text("<style>p{color:red}</style><script>x()</script>ок"), "ок");
        assert_eq!(html_to_text("a&nbsp;b &amp; c &#8212; всё"), "a b & c — всё");
        assert_eq!(html_to_text("<div>строчка<br>другая</div>"), "строчка\nдругая");
    }

    #[test]
    fn ids_roundtrip_through_columns_and_slashes() {
        let id = make_id("a@b.c", "f:Работа/2026", 42);
        assert_eq!(id, "a@b.c:f%3AРабота/2026:42");
        assert_eq!(parse_id(&id), Some(("a@b.c".to_string(), "f:Работа/2026".to_string(), 42)));
        assert_eq!(parse_id("a@b.c:inbox:7"), Some(("a@b.c".to_string(), "inbox".to_string(), 7)));
        assert_eq!(parse_id("a@b.c:inbox:7:лишнее"), None);
        assert_eq!(parse_id("мусор"), None);
    }

    #[test]
    fn search_haystack_is_lowercase() {
        let row = Row {
            id: make_id("a@b.c", "inbox", 3),
            account: "a@b.c".into(),
            folder: "inbox".into(),
            uid: 3,
            unread: true,
            starred: false,
            from_name: "Иван".into(),
            from_addr: "ivan@example.com".into(),
            to: vec![Mailbox { name: "Я".into(), addr: "me@b.c".into() }],
            subject: "Счёт за Октябрь".into(),
            date: String::new(),
            timestamp: 0,
            has_attachments: false,
            unavailable: false,
        };
        assert!(haystack(&row).contains("счёт"));
        assert!(haystack(&row).contains("ivan@example.com"));
    }
}
