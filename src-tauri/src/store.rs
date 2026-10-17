//! Письма в памяти: общий формат для IMAP-синхронизации и демо-данных.
//! Письма ящиков дублируются в шифрованный кэш на диске ([`Db`]); демо-письма — нет.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};

use chrono::{Local, TimeZone};
use serde::Serialize;

use crate::attachments::AttachmentMeta;
use crate::db::Db;

/// Папка ящика: стандартные — по ролям SPECIAL-USE (RFC 6154), остальные — по имени на сервере.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Folder {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Junk,
    Archive,
    /// Имя на сервере (как в LIST, в modified UTF-7). Имена интернируются, поэтому тип остаётся Copy.
    Other(&'static str),
}

impl Folder {
    pub fn other(name: &str) -> Self {
        static NAMES: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
        let mut names = NAMES.lock().unwrap();
        let names = names.get_or_insert_with(HashSet::new);
        let name = match names.get(name) {
            Some(&known) => known,
            None => {
                // Папок у ящика немного, а живут они до конца работы программы.
                let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
                names.insert(leaked);
                leaked
            }
        };
        Folder::Other(name)
    }

    /// Ключ для UI и кэша: `inbox`, `sent`, … или `f:<имя на сервере>`.
    pub fn key(self) -> String {
        match self {
            Folder::Inbox => "inbox".into(),
            Folder::Sent => "sent".into(),
            Folder::Drafts => "drafts".into(),
            Folder::Trash => "trash".into(),
            Folder::Junk => "junk".into(),
            Folder::Archive => "archive".into(),
            Folder::Other(name) => format!("f:{name}"),
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "inbox" => Folder::Inbox,
            "sent" => Folder::Sent,
            "drafts" => Folder::Drafts,
            "trash" => Folder::Trash,
            "junk" => Folder::Junk,
            "archive" => Folder::Archive,
            _ => Folder::other(key.strip_prefix("f:").filter(|name| !name.is_empty())?),
        })
    }
}

/// Результат проверки отправителя (DMARC), насколько ему можно верить.
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Auth {
    Pass,
    Fail,
    Unknown,
}

pub struct Message {
    /// Адрес ящика; `None` у демо-писем.
    pub account: Option<String>,
    pub folder: Folder,
    /// UID на сервере; `None` у демо-писем.
    pub uid: Option<u32>,
    pub starred: bool,
    pub unread: bool,
    pub from_name: String,
    pub from_addr: String,
    pub subject: String,
    /// Unix-время, секунды.
    pub timestamp: i64,
    pub auth: Auth,
    /// Исходный (ещё не очищенный) HTML; у IMAP-писем загружается при открытии.
    pub html: Option<String>,
    /// Message-ID и References — для цепочки при ответе; известны после открытия.
    pub message_id: Option<String>,
    pub references: Vec<String>,
    /// По Content-Type при синхронизации; после открытия — точно.
    pub has_attachments: bool,
    /// Известны после открытия письма.
    pub attachments: Vec<AttachmentMeta>,
}

struct Entry {
    id: u32,
    msg: Message,
    haystack: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    id: u32,
    in_trash: bool,
    account: Option<String>,
    from_addr: String,
    from_name: String,
    subject: String,
    date: String,
    unread: bool,
    starred: bool,
    has_attachments: bool,
}

/// Всё, что нужно, чтобы открыть письмо.
#[derive(Clone)]
pub struct Details {
    pub id: u32,
    pub account: Option<String>,
    pub folder: Folder,
    pub uid: Option<u32>,
    pub from_name: String,
    pub from_addr: String,
    pub subject: String,
    pub date: String,
    pub auth: Auth,
    pub html: Option<String>,
    pub unread: bool,
    pub timestamp: i64,
    pub message_id: Option<String>,
    pub references: Vec<String>,
    pub attachments: Vec<AttachmentMeta>,
}

/// Письмо, загруженное с сервера целиком.
pub struct Opened {
    pub html: String,
    pub auth: Auth,
    pub message_id: Option<String>,
    pub references: Vec<String>,
    pub attachments: Vec<AttachmentMeta>,
}

#[derive(Default)]
pub struct Store {
    next_id: AtomicU32,
    entries: RwLock<Vec<Entry>>,
    db: OnceLock<Db>,
    /// UIDVALIDITY папок ящиков: если на сервере он другой, UID в кэше устарели.
    validity: Mutex<HashMap<(String, Folder), u32>>,
}

impl Store {
    /// Подключает открытый кэш и показывает всё, что в нём есть (до связи с сервером).
    pub fn attach_db(&self, db: Db) -> crate::imap::Result<()> {
        if self.db.get().is_some() {
            return Ok(());
        }
        let (messages, validity) = db.load()?;
        *self.validity.lock().unwrap() = validity.into_iter().map(|(a, f, v)| ((a, f), v)).collect();
        self.replace(|m| m.account.is_some(), messages);
        let _ = self.db.set(db);
        Ok(())
    }

    pub fn has_db(&self) -> bool {
        self.db.get().is_some()
    }

    /// Запись в кэш на диске. Ошибка не мешает работе: письма всё равно есть на сервере.
    fn persist(&self, write: impl FnOnce(&Db) -> crate::imap::Result<()>) {
        if let Some(db) = self.db.get() {
            if let Err(e) = write(db) {
                eprintln!("mMail: запись в кэш не удалась: {e}");
            }
        }
    }

    /// Заменяет всё содержимое (демо-режим).
    pub fn replace_all(&self, messages: Vec<Message>) {
        self.replace(|_| true, messages);
    }

    /// Убирает письма удалённого (или заново добавленного) ящика.
    pub fn remove_account(&self, email: &str) {
        self.entries.write().unwrap().retain(|e| e.msg.account.as_deref() != Some(email));
        self.validity.lock().unwrap().retain(|(account, _), _| account != email);
        self.persist(|db| db.remove_account(email));
    }

    /// Добавляет порцию писем, пришедшую при синхронизации; уже известные (тот же UID) пропускает.
    pub fn append(&self, messages: Vec<Message>) {
        let known: HashSet<(String, Folder, u32)> = {
            let entries = self.entries.read().unwrap();
            entries.iter().filter_map(|e| Some((e.msg.account.clone()?, e.msg.folder, e.msg.uid?))).collect()
        };
        let messages: Vec<Message> = messages
            .into_iter()
            .filter(|m| match (&m.account, m.uid) {
                (Some(account), Some(uid)) => !known.contains(&(account.clone(), m.folder, uid)),
                _ => true,
            })
            .collect();
        self.persist(|db| db.insert(&messages.iter().collect::<Vec<_>>()));
        self.replace(|_| false, messages);
    }

    /// Непрочитанные письма папки (`folder` — ключ из UI): `(id, ящик, папка, UID)`.
    pub fn unread_in(&self, scope: Scope, folder: &str) -> Option<Vec<(u32, Option<String>, Folder, Option<u32>)>> {
        let in_folder = folder_filter(folder)?;
        let entries = self.entries.read().unwrap();
        Some(
            entries
                .iter()
                .filter(|e| e.msg.unread && scope.contains(&e.msg) && in_folder(&e.msg))
                .map(|e| (e.id, e.msg.account.clone(), e.msg.folder, e.msg.uid))
                .collect(),
        )
    }

    /// Папки ящика, письма которых есть в хранилище.
    pub fn folders_of(&self, account: &str) -> Vec<Folder> {
        let entries = self.entries.read().unwrap();
        let set: HashSet<Folder> = entries.iter().filter(|e| e.msg.account.as_deref() == Some(account)).map(|e| e.msg.folder).collect();
        set.into_iter().collect()
    }

    /// Есть ли письма в папке с этой ролью хоть у одного ящика.
    pub fn has_folder(&self, folder: Folder) -> bool {
        self.entries.read().unwrap().iter().any(|e| e.msg.folder == folder)
    }

    /// UID писем папки, которые уже есть в хранилище.
    pub fn uids(&self, account: &str, folder: Folder) -> HashSet<u32> {
        let entries = self.entries.read().unwrap();
        entries.iter().filter(|e| e.msg.folder == folder && e.msg.account.as_deref() == Some(account)).filter_map(|e| e.msg.uid).collect()
    }

    /// Сверяет UIDVALIDITY папки; если он сменился (или папка новая) — забывает её письма.
    pub fn check_validity(&self, account: &str, folder: Folder, validity: u32) {
        let key = (account.to_string(), folder);
        if self.validity.lock().unwrap().get(&key) == Some(&validity) {
            return;
        }
        self.entries.write().unwrap().retain(|e| !(e.msg.folder == folder && e.msg.account.as_deref() == Some(account)));
        self.validity.lock().unwrap().insert(key, validity);
        self.persist(|db| db.reset_folder(account, folder, validity));
    }

    /// Apply only after the server confirmed MOVE. Demo messages move locally.
    pub fn trashed(&self, id: u32) {
        let mut entries = self.entries.write().unwrap();
        let Some(index) = entries.iter().position(|e| e.id == id) else { return };
        if entries[index].msg.account.is_none() {
            entries[index].msg.folder = Folder::Trash;
            return;
        }
        let entry = entries.remove(index);
        if let (Some(account), Some(uid)) = (entry.msg.account.as_deref(), entry.msg.uid) {
            self.persist(|db| db.delete(account, entry.msg.folder, &[uid]));
        }
    }

    /// Убирает письма папки, которых больше нет на сервере. Возвращает, было ли что убирать.
    pub fn retain_on_server(&self, account: &str, folder: Folder, on_server: &HashSet<u32>) -> bool {
        let mut gone = Vec::new();
        self.entries.write().unwrap().retain(|e| {
            let stale = e.msg.folder == folder
                && e.msg.account.as_deref() == Some(account)
                && e.msg.uid.is_some_and(|uid| !on_server.contains(&uid));
            if stale {
                gone.extend(e.msg.uid);
            }
            !stale
        });
        self.persist(|db| db.delete(account, folder, &gone));
        !gone.is_empty()
    }

    /// Удаляет письма, подходящие под `drop`, добавляет новые; держит порядок от новых к старым.
    fn replace(&self, drop: impl Fn(&Message) -> bool, messages: Vec<Message>) {
        let mut entries = self.entries.write().unwrap();
        entries.retain(|e| !drop(&e.msg));
        entries.extend(messages.into_iter().map(|msg| Entry {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            haystack: format!("{} {} {}", msg.from_name, msg.from_addr, msg.subject).to_lowercase(),
            msg,
        }));
        entries.sort_by(|a, b| b.msg.timestamp.cmp(&a.msg.timestamp));
    }

    /// Страница писем папки (`query` уже в нижнем регистре) и общее число совпадений.
    pub fn page(&self, scope: Scope, folder: &str, query: &str, offset: usize, limit: usize) -> Option<(usize, Vec<Summary>)> {
        let in_folder = folder_filter(folder)?;
        let entries = self.entries.read().unwrap();
        let mut total = 0;
        let mut items = Vec::with_capacity(limit);
        let matches = |e: &&Entry| scope.contains(&e.msg) && in_folder(&e.msg) && (query.is_empty() || e.haystack.contains(query));
        for e in entries.iter().filter(matches) {
            if total >= offset && items.len() < limit {
                items.push(Summary {
                    id: e.id,
                    in_trash: e.msg.folder == Folder::Trash,
                    account: e.msg.account.clone(),
                    from_addr: e.msg.from_addr.clone(),
                    from_name: e.msg.from_name.clone(),
                    subject: e.msg.subject.clone(),
                    date: date_label(e.msg.timestamp),
                    unread: e.msg.unread,
                    starred: e.msg.starred,
                    has_attachments: e.msg.has_attachments,
                });
            }
            total += 1;
        }
        Some((total, items))
    }

    pub fn unread(&self, scope: Scope, folder: &str) -> usize {
        let Some(in_folder) = folder_filter(folder) else { return 0 };
        self.entries.read().unwrap().iter().filter(|e| scope.contains(&e.msg) && in_folder(&e.msg) && e.msg.unread).count()
    }

    /// Ящики, письма которых есть в хранилище; `None` — демо-письма.
    pub fn accounts(&self) -> Vec<Option<String>> {
        let mut accounts: Vec<_> = self.entries.read().unwrap().iter().map(|e| e.msg.account.clone()).collect();
        accounts.sort();
        accounts.dedup();
        accounts
    }

    pub fn details(&self, id: u32) -> Option<Details> {
        let entries = self.entries.read().unwrap();
        let e = entries.iter().find(|e| e.id == id)?;
        Some(Details {
            id,
            account: e.msg.account.clone(),
            folder: e.msg.folder,
            uid: e.msg.uid,
            from_name: e.msg.from_name.clone(),
            from_addr: e.msg.from_addr.clone(),
            subject: e.msg.subject.clone(),
            date: date_label(e.msg.timestamp),
            auth: e.msg.auth,
            html: e.msg.html.clone(),
            unread: e.msg.unread,
            timestamp: e.msg.timestamp,
            message_id: e.msg.message_id.clone(),
            references: e.msg.references.clone(),
            attachments: e.msg.attachments.clone(),
        })
    }

    pub fn set_starred(&self, id: u32, starred: bool) {
        let mut entries = self.entries.write().unwrap();
        let Some(e) = entries.iter_mut().find(|e| e.id == id) else { return };
        e.msg.starred = starred;
        if let (Some(account), Some(uid)) = (&e.msg.account, e.msg.uid) {
            self.persist(|db| db.set_flags(account, e.msg.folder, &[(uid, e.msg.unread, starred)]));
        }
    }

    pub fn set_unread(&self, id: u32, unread: bool) {
        let mut entries = self.entries.write().unwrap();
        let Some(e) = entries.iter_mut().find(|e| e.id == id) else { return };
        e.msg.unread = unread;
        if let (Some(account), Some(uid)) = (&e.msg.account, e.msg.uid) {
            self.persist(|db| db.set_flags(account, e.msg.folder, &[(uid, unread, e.msg.starred)]));
        }
    }

    /// Наибольший известный UID папки ящика (0 — писем ещё нет).
    pub fn max_uid(&self, account: &str, folder: Folder) -> u32 {
        let entries = self.entries.read().unwrap();
        entries
            .iter()
            .filter(|e| e.msg.folder == folder && e.msg.account.as_deref() == Some(account))
            .filter_map(|e| e.msg.uid)
            .max()
            .unwrap_or(0)
    }

    /// Флаги с сервера: `(uid, непрочитано, важное)`. Возвращает, изменилось ли что-нибудь.
    pub fn update_flags(&self, account: &str, folder: Folder, flags: &[(u32, bool, bool)]) -> bool {
        let by_uid: std::collections::HashMap<u32, (bool, bool)> = flags.iter().map(|&(uid, u, s)| (uid, (u, s))).collect();
        let mut changed = Vec::new();
        for e in self.entries.write().unwrap().iter_mut() {
            if e.msg.folder != folder || e.msg.account.as_deref() != Some(account) {
                continue;
            }
            if let Some(&(unread, starred)) = e.msg.uid.and_then(|uid| by_uid.get(&uid)) {
                if (e.msg.unread, e.msg.starred) != (unread, starred) {
                    e.msg.unread = unread;
                    e.msg.starred = starred;
                    changed.push((e.msg.uid.unwrap_or_default(), unread, starred));
                }
            }
        }
        if !changed.is_empty() {
            self.persist(|db| db.set_flags(account, folder, &changed));
        }
        !changed.is_empty()
    }

    /// UID писем папки без загруженного тела (не старше `since`), от новых к старым.
    pub fn unopened(&self, account: &str, folder: Folder, since: Option<i64>) -> Vec<u32> {
        let entries = self.entries.read().unwrap();
        entries
            .iter()
            .filter(|e| e.msg.html.is_none() && e.msg.folder == folder && e.msg.account.as_deref() == Some(account))
            .filter(|e| since.is_none_or(|since| e.msg.timestamp >= since))
            .filter_map(|e| e.msg.uid)
            .collect()
    }

    /// Забывает содержимое писем ящиков старше `cutoff` (заголовки остаются; при открытии тело скачается снова).
    /// Возвращает размер кэша на диске.
    pub fn clear_bodies(&self, cutoff: i64) -> crate::imap::Result<u64> {
        for e in self.entries.write().unwrap().iter_mut() {
            if e.msg.account.is_some() && e.msg.timestamp < cutoff {
                e.msg.html = None;
            }
        }
        match self.db.get() {
            Some(db) => db.clear_bodies(cutoff),
            None => Ok(0),
        }
    }

    pub fn disk_size(&self) -> u64 {
        self.db.get().map_or(0, Db::size)
    }

    /// Как [`Store::set_opened`], но по UID; возвращает, поменялась ли отметка о вложениях.
    pub fn set_opened_uid(&self, account: &str, folder: Folder, uid: u32, opened: Opened) -> bool {
        let found = {
            let entries = self.entries.read().unwrap();
            entries
                .iter()
                .find(|e| e.msg.uid == Some(uid) && e.msg.folder == folder && e.msg.account.as_deref() == Some(account))
                .map(|e| (e.id, e.msg.has_attachments))
        };
        let Some((id, had)) = found else { return false };
        let has = !opened.attachments.is_empty();
        self.set_opened(id, opened);
        had != has
    }

    /// Запоминает загруженное с сервера письмо: тело, проверку отправителя, идентификаторы цепочки.
    pub fn set_opened(&self, id: u32, opened: Opened) {
        if let Some(e) = self.entries.write().unwrap().iter_mut().find(|e| e.id == id) {
            if let (Some(account), Some(uid)) = (&e.msg.account, e.msg.uid) {
                self.persist(|db| db.set_opened(account, e.msg.folder, uid, &opened));
            }
            e.msg.html = Some(opened.html);
            e.msg.auth = opened.auth;
            e.msg.message_id = opened.message_id;
            e.msg.references = opened.references;
            e.msg.has_attachments = !opened.attachments.is_empty();
            e.msg.attachments = opened.attachments;
        }
    }
}

/// Какие ящики показывать: все сразу, только демо-письма или один ящик.
#[derive(Clone, Copy)]
pub enum Scope<'a> {
    All,
    Demo,
    Account(&'a str),
}

impl<'a> Scope<'a> {
    /// `None` — все ящики, `""` — демо, иначе адрес ящика.
    pub fn parse(account: Option<&'a str>) -> Self {
        match account {
            None => Scope::All,
            Some("") => Scope::Demo,
            Some(email) => Scope::Account(email),
        }
    }

    fn contains(&self, m: &Message) -> bool {
        match self {
            Scope::All => true,
            Scope::Demo => m.account.is_none(),
            Scope::Account(email) => m.account.as_deref() == Some(email),
        }
    }
}

fn folder_filter(folder: &str) -> Option<Box<dyn Fn(&Message) -> bool>> {
    if folder == "starred" {
        return Some(Box::new(|m| m.starred && !matches!(m.folder, Folder::Trash | Folder::Junk)));
    }
    let folder = Folder::from_key(folder)?;
    Some(Box::new(move |m| m.folder == folder))
}

/// Всегда дата и время в местном часовом поясе: «17.10.2026 13:28».
pub fn date_label(timestamp: i64) -> String {
    Local.timestamp_opt(timestamp, 0).single().map(|at| at.format("%d.%m.%Y %H:%M").to_string()).unwrap_or_default()
}
