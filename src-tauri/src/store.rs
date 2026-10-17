//! Письма в памяти: общий формат для IMAP-синхронизации и демо-данных.
//! Письма ящиков дублируются в шифрованный кэш на диске ([`Db`]); демо-письма — нет.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};

use chrono::{Local, TimeZone};
use serde::Serialize;

use crate::attachments::AttachmentMeta;
use crate::db::Db;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Folder {
    Inbox,
    Sent,
    Drafts,
    Trash,
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

fn folder_filter(folder: &str) -> Option<fn(&Message) -> bool> {
    Some(match folder {
        "inbox" => |m| m.folder == Folder::Inbox,
        "starred" => |m| m.starred && m.folder != Folder::Trash,
        "sent" => |m| m.folder == Folder::Sent,
        "drafts" => |m| m.folder == Folder::Drafts,
        "trash" => |m| m.folder == Folder::Trash,
        _ => return None,
    })
}

/// Всегда дата и время в местном часовом поясе: «17.10.2026 13:28».
pub fn date_label(timestamp: i64) -> String {
    Local.timestamp_opt(timestamp, 0).single().map(|at| at.format("%d.%m.%Y %H:%M").to_string()).unwrap_or_default()
}
