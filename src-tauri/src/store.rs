//! Письма в памяти: общий формат для IMAP-синхронизации и демо-данных.
//! Пока без диска — шифрованное хранилище (SQLCipher) следующим шагом.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::RwLock;

use chrono::{Datelike, Local, TimeZone};
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
    pub folder: Folder,
    pub uid: Option<u32>,
    pub from_name: String,
    pub from_addr: String,
    pub subject: String,
    pub date: String,
    pub auth: Auth,
    pub html: Option<String>,
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
    pub fn page(&self, folder: &str, query: &str, offset: usize, limit: usize) -> Option<(usize, Vec<Summary>)> {
        let in_folder = folder_filter(folder)?;
        let entries = self.entries.read().unwrap();
        let mut total = 0;
        let mut items = Vec::with_capacity(limit);
        for e in entries.iter().filter(|e| in_folder(&e.msg) && (query.is_empty() || e.haystack.contains(query))) {
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

    pub fn unread(&self, folder: &str) -> usize {
        let Some(in_folder) = folder_filter(folder) else { return 0 };
        self.entries.read().unwrap().iter().filter(|e| in_folder(&e.msg) && e.msg.unread).count()
    }

    pub fn details(&self, id: u32) -> Option<Details> {
        let entries = self.entries.read().unwrap();
        let e = entries.iter().find(|e| e.id == id)?;
        Some(Details {
            id,
            folder: e.msg.folder,
            uid: e.msg.uid,
            from_name: e.msg.from_name.clone(),
            from_addr: e.msg.from_addr.clone(),
            subject: e.msg.subject.clone(),
            date: date_label(e.msg.timestamp),
            auth: e.msg.auth,
            html: e.msg.html.clone(),
        })
    }

    /// Запоминает загруженное с сервера тело письма и результат проверки отправителя.
    pub fn set_opened(&self, id: u32, html: String, auth: Auth) {
        if let Some(e) = self.entries.write().unwrap().iter_mut().find(|e| e.id == id) {
            e.msg.html = Some(html);
            e.msg.auth = auth;
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

const MONTHS: [&str; 12] = ["янв", "фев", "мар", "апр", "мая", "июн", "июл", "авг", "сен", "окт", "ноя", "дек"];

fn date_label(timestamp: i64) -> String {
    let Some(at) = Local.timestamp_opt(timestamp, 0).single() else { return String::new() };
    let today = Local::now().date_naive();
    let day = at.date_naive();
    if day == today {
        at.format("%H:%M").to_string()
    } else if today.pred_opt() == Some(day) {
        "вчера".into()
    } else if day.year() == today.year() {
        format!("{} {}", day.day(), MONTHS[day.month0() as usize])
    } else {
        day.format("%d.%m.%Y").to_string()
    }
}
