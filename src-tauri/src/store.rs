//! Письма в памяти: общий формат для IMAP-синхронизации и демо-данных.
//! Пока без диска — шифрованное хранилище (SQLCipher) следующим шагом.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::RwLock;

use chrono::{Local, TimeZone};
use serde::Serialize;

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
}

/// Письмо, загруженное с сервера целиком.
pub struct Opened {
    pub html: String,
    pub auth: Auth,
    pub message_id: Option<String>,
    pub references: Vec<String>,
}

#[derive(Default)]
pub struct Store {
    next_id: AtomicU32,
    entries: RwLock<Vec<Entry>>,
}

impl Store {
    /// Заменяет всё содержимое (демо-режим).
    pub fn replace_all(&self, messages: Vec<Message>) {
        self.replace(|_| true, messages);
    }

    /// Убирает письма удалённого (или заново добавленного) ящика.
    pub fn remove_account(&self, email: &str) {
        self.entries.write().unwrap().retain(|e| e.msg.account.as_deref() != Some(email));
    }

    /// Добавляет порцию писем, пришедшую при синхронизации.
    pub fn append(&self, messages: Vec<Message>) {
        self.replace(|_| false, messages);
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
        })
    }

    pub fn set_unread(&self, id: u32, unread: bool) {
        if let Some(e) = self.entries.write().unwrap().iter_mut().find(|e| e.id == id) {
            e.msg.unread = unread;
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
        let mut changed = false;
        for e in self.entries.write().unwrap().iter_mut() {
            if e.msg.folder != folder || e.msg.account.as_deref() != Some(account) {
                continue;
            }
            if let Some(&(unread, starred)) = e.msg.uid.and_then(|uid| by_uid.get(&uid)) {
                if (e.msg.unread, e.msg.starred) != (unread, starred) {
                    e.msg.unread = unread;
                    e.msg.starred = starred;
                    changed = true;
                }
            }
        }
        changed
    }

    /// Запоминает загруженное с сервера письмо: тело, проверку отправителя, идентификаторы цепочки.
    pub fn set_opened(&self, id: u32, opened: Opened) {
        if let Some(e) = self.entries.write().unwrap().iter_mut().find(|e| e.id == id) {
            e.msg.html = Some(opened.html);
            e.msg.auth = opened.auth;
            e.msg.message_id = opened.message_id;
            e.msg.references = opened.references;
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
