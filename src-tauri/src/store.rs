//! Письма в памяти: общий формат для IMAP-синхронизации и демо-данных.
//! Письма ящиков дублируются в шифрованный кэш на диске ([`Db`]); демо-письма — нет.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};

use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};

use crate::attachments::AttachmentMeta;
use crate::db::Db;
use crate::text::display_safe;

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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Mailbox {
    pub name: String,
    pub addr: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Recipients {
    pub to: Vec<Mailbox>,
    pub cc: Vec<Mailbox>,
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
    /// Кому и копия. `None` — письмо попало в кэш до того, как их стали запрашивать:
    /// в отправленных дозапрашиваем сразу, остальные — при открытии.
    pub recipients: Option<Recipients>,
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
    /// Сервер отказал в заголовке (Яндекс: `NO [UNAVAILABLE]`): в списке — заглушка
    /// с датой и флагами, заголовок запрашиваем позже ещё раз.
    pub unavailable: bool,
}

struct Entry {
    id: u32,
    msg: Message,
    haystack: String,
    /// Тело лежит в кэше на диске, а не в `msg.html`: в памяти держим только заголовки.
    body_on_disk: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    id: u32,
    in_trash: bool,
    account: Option<String>,
    from_addr: String,
    from_name: String,
    to: Vec<Mailbox>,
    subject: String,
    date: String,
    unread: bool,
    starred: bool,
    has_attachments: bool,
    /// Заглушка: сервер не отдал заголовок.
    unavailable: bool,
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
    pub recipients: Option<Recipients>,
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
    pub recipients: Recipients,
    pub auth: Auth,
    pub message_id: Option<String>,
    pub references: Vec<String>,
    pub attachments: Vec<AttachmentMeta>,
}

/// Предел исходника, который кладём в кэш ради сохранения вложений. Больше — редкость,
/// а диск занимать надолго не хочется: такое письмо при сохранении вложения скачается снова.
const RAW_CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;

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
        self.replace_entries(|m| m.account.is_some(), messages);
        let _ = self.db.set(db);
        crate::memory::trim();
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

    /// Непрочитанные письма папки (`query` уже в нижнем регистре): `(id, ящик, папка, UID)`.
    pub fn unread_in(&self, scope: Scope, folder: &str, query: &str, filter: AddressFilter) -> Option<Vec<(u32, Option<String>, Folder, Option<u32>)>> {
        let in_folder = folder_filter(folder)?;
        let entries = self.entries.read().unwrap();
        Some(
            entries
                .iter()
                .filter(|e| e.msg.unread && scope.contains(&e.msg) && in_folder(&e.msg) && filter.matches(&e.msg) && (query.is_empty() || e.haystack.contains(query)))
                .map(|e| (e.id, e.msg.account.clone(), e.msg.folder, e.msg.uid))
                .collect(),
        )
    }

    /// UID заглушек папки (сервер не отдал заголовок), от новых к старым.
    pub fn unavailable_uids(&self, account: &str, folder: Folder) -> Vec<u32> {
        let entries = self.entries.read().unwrap();
        entries
            .iter()
            .filter(|e| e.msg.unavailable && e.msg.folder == folder && e.msg.account.as_deref() == Some(account))
            .filter_map(|e| e.msg.uid)
            .collect()
    }

    /// Отправленные и черновики из кэша до v4: получатели неизвестны.
    pub fn missing_recipients(&self, account: &str, folder: Folder) -> Vec<u32> {
        if !matches!(folder, Folder::Sent | Folder::Drafts) {
            return Vec::new();
        }
        let entries = self.entries.read().unwrap();
        entries
            .iter()
            .filter(|e| e.msg.recipients.is_none() && !e.msg.unavailable && e.msg.folder == folder && e.msg.account.as_deref() == Some(account))
            .filter_map(|e| e.msg.uid)
            .collect()
    }

    /// Сервер отдал заголовки писем, которые были заглушками (или без получателей).
    pub fn replace_headers(&self, messages: Vec<Message>) {
        let mut entries = self.entries.write().unwrap();
        for m in messages {
            let Some(e) = entries.iter_mut().find(|e| e.msg.uid == m.uid && e.msg.folder == m.folder && e.msg.account == m.account) else {
                continue;
            };
            self.persist(|db| db.replace_headers(&m));
            e.haystack = haystack(&m);
            e.msg.from_name = m.from_name;
            e.msg.from_addr = m.from_addr;
            e.msg.recipients = m.recipients;
            e.msg.subject = m.subject;
            e.msg.timestamp = m.timestamp;
            e.msg.has_attachments = m.has_attachments;
            e.msg.unread = m.unread;
            e.msg.starred = m.starred;
            e.msg.unavailable = false;
        }
        entries.sort_by(|a, b| b.msg.timestamp.cmp(&a.msg.timestamp));
    }

    /// Папку удалили на сервере: забываем её письма и UIDVALIDITY.
    pub fn remove_folder(&self, account: &str, folder: Folder) {
        self.entries.write().unwrap().retain(|e| !(e.msg.folder == folder && e.msg.account.as_deref() == Some(account)));
        self.validity.lock().unwrap().remove(&(account.to_string(), folder));
        self.persist(|db| db.remove_folder(account, folder));
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
        self.moved(id, Folder::Trash);
    }

    /// Письмо перенесено в `target`: у настоящих ящиков оно уходит из кэша до синхронизации
    /// целевой папки (там у него будет новый UID), демо переносится на месте.
    pub fn moved(&self, id: u32, target: Folder) {
        let mut entries = self.entries.write().unwrap();
        let Some(index) = entries.iter().position(|e| e.id == id) else { return };
        if entries[index].msg.account.is_none() {
            entries[index].msg.folder = target;
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
        self.replace_entries(drop, messages.into_iter().map(|m| (m, false)).collect());
    }

    /// Как [`Store::replace`]; второе в паре — тело уже лежит в кэше на диске.
    fn replace_entries(&self, drop: impl Fn(&Message) -> bool, messages: Vec<(Message, bool)>) {
        let mut entries = self.entries.write().unwrap();
        entries.retain(|e| !drop(&e.msg));
        entries.extend(messages.into_iter().map(|(msg, body_on_disk)| Entry {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            haystack: haystack(&msg),
            msg,
            body_on_disk,
        }));
        entries.sort_by(|a, b| b.msg.timestamp.cmp(&a.msg.timestamp));
    }

    /// Страница писем папки (`query` уже в нижнем регистре) и общее число совпадений.
    pub fn page(&self, scope: Scope, folder: &str, query: &str, filter: AddressFilter, offset: usize, limit: usize, unread_only: bool) -> Option<(usize, Vec<Summary>)> {
        let in_folder = folder_filter(folder)?;
        let entries = self.entries.read().unwrap();
        let mut total = 0;
        let mut items = Vec::with_capacity(limit);
        let matches = |e: &&Entry| scope.contains(&e.msg) && in_folder(&e.msg) && filter.matches(&e.msg) && (!unread_only || e.msg.unread) && (query.is_empty() || e.haystack.contains(query));
        for e in entries.iter().filter(matches) {
            if total >= offset && items.len() < limit {
                items.push(Summary {
                    id: e.id,
                    in_trash: e.msg.folder == Folder::Trash,
                    account: e.msg.account.clone(),
                    from_addr: display_safe(&e.msg.from_addr).into_owned(),
                    from_name: display_safe(&e.msg.from_name).into_owned(),
                    to: e
                        .msg
                        .recipients
                        .as_ref()
                        .map(|r| {
                            r.to.iter()
                                .map(|a| Mailbox {
                                    name: display_safe(&a.name).into_owned(),
                                    addr: display_safe(&a.addr).into_owned(),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    subject: display_safe(&e.msg.subject).into_owned(),
                    date: date_label(e.msg.timestamp),
                    unread: e.msg.unread,
                    starred: e.msg.starred,
                    has_attachments: e.msg.has_attachments,
                    unavailable: e.msg.unavailable,
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

    /// id письма в хранилище по ящику, папке и UID.
    pub fn id_of(&self, account: &str, folder: Folder, uid: u32) -> Option<u32> {
        let entries = self.entries.read().unwrap();
        entries.iter().find(|e| e.msg.uid == Some(uid) && e.msg.folder == folder && e.msg.account.as_deref() == Some(account)).map(|e| e.id)
    }

    /// Ящик, папка и UID письма — без чтения тела из кэша (оно может быть в мегабайтах).
    pub fn location(&self, id: u32) -> Option<(Option<String>, Folder, Option<u32>)> {
        let entries = self.entries.read().unwrap();
        let e = entries.iter().find(|e| e.id == id)?;
        Some((e.msg.account.clone(), e.msg.folder, e.msg.uid))
    }

    pub fn details(&self, id: u32) -> Option<Details> {
        let mut details = {
            let entries = self.entries.read().unwrap();
            let e = entries.iter().find(|e| e.id == id)?;
            (self.details_of(e), e.body_on_disk)
        };
        // Тело читаем с диска уже без блокировки списка.
        if let (true, Some(db), Some(account), Some(uid)) = (details.1, self.db.get(), details.0.account.as_deref(), details.0.uid) {
            match db.body(account, details.0.folder, uid) {
                Ok(html) => details.0.html = html,
                Err(e) => eprintln!("mMail: чтение письма из кэша: {e}"),
            }
        }
        Some(details.0)
    }

    fn details_of(&self, e: &Entry) -> Details {
        let id = e.id;
        Details {
            id,
            account: e.msg.account.clone(),
            folder: e.msg.folder,
            uid: e.msg.uid,
            from_name: e.msg.from_name.clone(),
            from_addr: e.msg.from_addr.clone(),
            recipients: e.msg.recipients.clone(),
            subject: e.msg.subject.clone(),
            date: date_label(e.msg.timestamp),
            auth: e.msg.auth,
            html: e.msg.html.clone(),
            unread: e.msg.unread,
            timestamp: e.msg.timestamp,
            message_id: e.msg.message_id.clone(),
            references: e.msg.references.clone(),
            attachments: e.msg.attachments.clone(),
        }
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
            .filter(|e| e.msg.html.is_none() && !e.body_on_disk && !e.msg.unavailable && e.msg.folder == folder && e.msg.account.as_deref() == Some(account))
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
                e.body_on_disk = false;
            }
        }
        let size = match self.db.get() {
            Some(db) => db.clear_bodies(cutoff),
            None => Ok(0),
        };
        crate::memory::trim();
        size
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
            let mut saved = false;
            if let (Some(account), Some(uid), Some(db)) = (&e.msg.account, e.msg.uid, self.db.get()) {
                match db.set_opened(account, e.msg.folder, uid, &opened) {
                    Ok(()) => saved = true,
                    Err(err) => eprintln!("mMail: запись в кэш не удалась: {err}"),
                }
            }
            // Записали на диск — в памяти тело не держим; без кэша (хранилище закрыто) — держим.
            e.body_on_disk = saved;
            e.msg.html = if saved { None } else { Some(opened.html) };
            e.msg.auth = opened.auth;
            e.msg.message_id = opened.message_id;
            e.msg.references = opened.references;
            e.msg.has_attachments = !opened.attachments.is_empty();
            e.msg.attachments = opened.attachments;
            e.msg.recipients = Some(opened.recipients);
            e.haystack = haystack(&e.msg);
        }
    }

    /// Исходник письма из кэша: из него берутся вложения, не скачивая письмо с сервера.
    pub fn raw(&self, account: &str, folder: Folder, uid: u32) -> Option<Vec<u8>> {
        let db = self.db.get()?;
        match db.raw(account, folder, uid) {
            Ok(raw) => raw,
            Err(e) => {
                eprintln!("mMail: чтение исходника письма из кэша: {e}");
                None
            }
        }
    }

    /// Кладёт исходник письма с вложениями в кэш — целиком, как его отдал сервер:
    /// вложения и заголовки в нём всегда согласованы, а разбор при сохранении недорог.
    /// Очень большие письма не кэшируем, иначе кэш растёт быстрее, чем его чистят.
    pub fn cache_raw(&self, account: &str, folder: Folder, uid: u32, raw: &[u8]) {
        if raw.len() > RAW_CACHE_MAX_BYTES {
            return;
        }
        self.persist(|db| db.set_raw(account, folder, uid, raw));
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

/// Точный фильтр по адресу: «от» — для входящих, «кому» — для отправленных.
#[derive(Clone, Copy, Default)]
pub struct AddressFilter<'a> {
    pub from: Option<&'a str>,
    pub to: Option<&'a str>,
}

impl AddressFilter<'_> {
    fn matches(&self, m: &Message) -> bool {
        self.from.is_none_or(|addr| m.from_addr.eq_ignore_ascii_case(addr))
            && self.to.is_none_or(|addr| {
                m.recipients.as_ref().is_some_and(|r| r.to.iter().chain(r.cc.iter()).any(|a| a.addr.eq_ignore_ascii_case(addr)))
            })
    }
}

fn folder_filter(folder: &str) -> Option<Box<dyn Fn(&Message) -> bool>> {
    if folder == "starred" {
        return Some(Box::new(|m| m.starred && !matches!(m.folder, Folder::Trash | Folder::Junk)));
    }
    let folder = Folder::from_key(folder)?;
    Some(Box::new(move |m| m.folder == folder))
}

/// Строка для поиска: отправитель, получатели, тема.
fn haystack(m: &Message) -> String {
    let to = m.recipients.iter().flat_map(|r| &r.to).map(|a| format!("{} {}", a.name, a.addr)).collect::<Vec<_>>().join(" ");
    format!("{} {} {to} {}", m.from_name, m.from_addr, m.subject).to_lowercase()
}

/// Всегда дата и время в местном часовом поясе: «17.10.2026 13:28».
pub fn date_label(timestamp: i64) -> String {
    Local.timestamp_opt(timestamp, 0).single().map(|at| at.format("%d.%m.%Y %H:%M").to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(uid: u32) -> Message {
        Message {
            account: Some("me@ya.ru".into()),
            folder: Folder::Inbox,
            uid: Some(uid),
            starred: false,
            unread: true,
            from_name: "a".into(),
            from_addr: "a@b.example".into(),
            subject: "s".into(),
            timestamp: 1_700_000_000 + i64::from(uid),
            auth: Auth::Unknown,
            html: None,
            message_id: None,
            references: Vec::new(),
            has_attachments: false,
            attachments: Vec::new(),
            unavailable: false,
            recipients: None,
        }
    }

    #[test]
    fn unread_pages_filter_before_pagination_and_combine_with_scope_and_search() {
        let store = Store::default();
        let mut messages: Vec<_> = (1..=205).map(message).collect();
        for m in &mut messages {
            m.unread = m.uid.unwrap() % 2 == 1;
        }
        let mut other_account = message(206);
        other_account.account = Some("other@example.com".into());
        let mut other_folder = message(207);
        other_folder.folder = Folder::other("Work");
        let mut search_match = message(208);
        search_match.subject = "Unique subject".into();
        search_match.starred = true;
        messages.extend([other_account, other_folder, search_match]);
        store.append(messages);
        let scope = || Scope::parse(Some("me@ya.ru"));
        let (total, first) = store.page(scope(), "inbox", "", AddressFilter::default(), 0, 100, true).unwrap();
        let (same_total, second) = store.page(scope(), "inbox", "", AddressFilter::default(), 100, 100, true).unwrap();
        assert_eq!((total, same_total, first.len(), second.len()), (104, 104, 100, 4));
        assert!(first.iter().chain(&second).all(|m| m.unread));
        assert!(first.iter().all(|a| second.iter().all(|b| a.id != b.id)));
        assert_eq!(store.page(scope(), "inbox", "", AddressFilter::default(), 0, 100, false).unwrap().0, 206);
        assert_eq!(store.page(Scope::All, "inbox", "", AddressFilter::default(), 0, 100, true).unwrap().0, 105);
        assert_eq!(store.page(scope(), "f:Work", "", AddressFilter::default(), 0, 100, true).unwrap().0, 1);
        let (total, found) = store.page(scope(), "inbox", "unique", AddressFilter::default(), 0, 100, true).unwrap();
        assert_eq!(total, 1);
        assert_eq!(found[0].subject, "Unique subject");
        assert_eq!(store.page(scope(), "starred", "unique", AddressFilter::default(), 0, 100, true).unwrap().0, 1);
        assert_eq!(store.page(scope(), "inbox", "missing", AddressFilter::default(), 0, 100, true).unwrap().0, 0);
        assert!(store.page(scope(), "inbox", "", AddressFilter::default(), 104, 100, true).unwrap().1.is_empty());

        // Массовое действие охватывает все страницы и ровно те же фильтры, что список.
        for (account, folder, query) in [
            (Some("me@ya.ru"), "inbox", ""),
            (None, "inbox", ""),
            (Some("me@ya.ru"), "f:Work", ""),
            (Some("me@ya.ru"), "starred", "unique"),
            (Some("me@ya.ru"), "inbox", "missing"),
        ] {
            let targets = store.unread_in(Scope::parse(account), folder, query, AddressFilter::default()).unwrap();
            let (_, visible) = store.page(Scope::parse(account), folder, query, AddressFilter::default(), 0, 1000, true).unwrap();
            assert_eq!(targets.iter().map(|m| m.0).collect::<Vec<_>>(), visible.iter().map(|m| m.id).collect::<Vec<_>>());
        }
        let targets = store.unread_in(scope(), "inbox", "unique", AddressFilter::default()).unwrap();
        assert_eq!(targets.len(), 1);
        for (id, _, _, _) in targets {
            store.set_unread(id, false);
        }
        assert!(store.unread_in(scope(), "inbox", "unique", AddressFilter::default()).unwrap().is_empty());
        assert_eq!(store.unread_in(scope(), "inbox", "", AddressFilter::default()).unwrap().len(), 103);
        assert_eq!(store.unread_in(scope(), "f:Work", "", AddressFilter::default()).unwrap().len(), 1);
    }

    #[test]
    fn address_filter_matches_sender_and_recipient_exactly() {
        let store = Store::default();
        let mut from_alice = message(1);
        from_alice.from_addr = "Alice@Example.com".into();
        let mut from_bob = message(2);
        from_bob.from_addr = "bob@example.com".into();
        let mut to_jane = message(3);
        to_jane.recipients = Some(Recipients {
            to: vec![Mailbox { name: "Jane".into(), addr: "jane@example.com".into() }],
            cc: Vec::new(),
        });
        store.append(vec![from_alice, from_bob, to_jane]);
        let by_from = AddressFilter { from: Some("alice@example.com"), to: None };
        assert_eq!(store.page(Scope::All, "inbox", "", by_from, 0, 100, false).unwrap().0, 1);
        // «Отметить все прочитанными» действует по тем же фильтрам, что и список.
        assert_eq!(store.unread_in(Scope::All, "inbox", "", by_from).unwrap().len(), 1);
        let by_to = AddressFilter { from: None, to: Some("JANE@example.com") };
        assert_eq!(store.page(Scope::All, "inbox", "", by_to, 0, 100, false).unwrap().0, 1);
        // Подстрока не подходит: фильтр по адресу точный.
        let domain = AddressFilter { from: Some("example.com"), to: None };
        assert_eq!(store.page(Scope::All, "inbox", "", domain, 0, 100, false).unwrap().0, 0);
    }

    #[test]
    fn unread_pages_follow_flag_changes_without_removing_messages() {
        let store = Store::default();
        store.append(vec![message(1)]);
        let id = store.id_of("me@ya.ru", Folder::Inbox, 1).unwrap();
        assert_eq!(store.page(Scope::All, "inbox", "", AddressFilter::default(), 0, 100, true).unwrap().0, 1);
        store.set_unread(id, false);
        let (total, items) = store.page(Scope::All, "inbox", "", AddressFilter::default(), 0, 100, true).unwrap();
        assert_eq!(total, 0);
        assert!(items.is_empty());
        assert_eq!(store.page(Scope::All, "inbox", "", AddressFilter::default(), 0, 100, false).unwrap().0, 1);
        store.set_unread(id, true);
        assert_eq!(store.page(Scope::All, "inbox", "", AddressFilter::default(), 0, 100, true).unwrap().1[0].id, id);
    }

    #[test]
    fn synced_batches_survive_restart() {
        let dir = std::env::temp_dir().join(format!("mmail-test-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let key = [9u8; 32];
        let store = Store::default();
        store.attach_db(Db::open(&dir, &key).unwrap()).unwrap();
        store.check_validity("me@ya.ru", Folder::Inbox, 7);
        store.append((1..=3).map(message).collect());
        store.append((3..=5).map(message).collect());
        drop(store);

        let again = Store::default();
        again.attach_db(Db::open(&dir, &key).unwrap()).unwrap();
        again.check_validity("me@ya.ru", Folder::Inbox, 7);
        assert_eq!(again.uids("me@ya.ru", Folder::Inbox).len(), 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn opened_bodies_live_on_disk_not_in_memory() {
        let dir = std::env::temp_dir().join(format!("mmail-test-bodies-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let key = [5u8; 32];
        let store = Store::default();
        store.attach_db(Db::open(&dir, &key).unwrap()).unwrap();
        store.check_validity("me@ya.ru", Folder::Inbox, 1);
        store.append(vec![message(1), message(2)]);
        let opened = || Opened { html: "<p>тело</p>".into(), recipients: Recipients::default(), auth: Auth::Unknown, message_id: None, references: Vec::new(), attachments: Vec::new() };
        assert!(store.set_opened_uid("me@ya.ru", Folder::Inbox, 1, opened()) == false);
        {
            let entries = store.entries.read().unwrap();
            assert!(entries.iter().all(|e| e.msg.html.is_none()), "в памяти тел нет");
        }
        let id = store.id_of("me@ya.ru", Folder::Inbox, 1).unwrap();
        assert_eq!(store.details(id).unwrap().html.as_deref(), Some("<p>тело</p>"));
        assert_eq!(store.unopened("me@ya.ru", Folder::Inbox, None), vec![2]);
        drop(store);

        let again = Store::default();
        again.attach_db(Db::open(&dir, &key).unwrap()).unwrap();
        let id = again.id_of("me@ya.ru", Folder::Inbox, 1).unwrap();
        assert_eq!(again.details(id).unwrap().html.as_deref(), Some("<p>тело</p>"));
        assert_eq!(again.unopened("me@ya.ru", Folder::Inbox, None), vec![2]);
        again.clear_bodies(i64::MAX).unwrap();
        assert_eq!(again.details(id).unwrap().html, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn message_source_for_attachments_survives_restart_until_body_expires() {
        let dir = std::env::temp_dir().join(format!("mmail-test-raw-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let key = [6u8; 32];
        let raw = b"From: a@b\r\nContent-Type: text/plain\r\n\r\nbody";
        let store = Store::default();
        store.attach_db(Db::open(&dir, &key).unwrap()).unwrap();
        store.append(vec![message(1)]);
        assert!(store.raw("me@ya.ru", Folder::Inbox, 1).is_none());
        store.cache_raw("me@ya.ru", Folder::Inbox, 1, raw);
        drop(store);

        let again = Store::default();
        again.attach_db(Db::open(&dir, &key).unwrap()).unwrap();
        assert_eq!(again.raw("me@ya.ru", Folder::Inbox, 1).as_deref(), Some(&raw[..]));
        // Исходник живёт ровно столько же, сколько тело письма.
        again.clear_bodies(i64::MAX).unwrap();
        assert!(again.raw("me@ya.ru", Folder::Inbox, 1).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod memory_probe {
    use super::*;

    fn rss_mb() -> u64 {
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        status.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap() / 1024
    }

    /// cargo test --release memory_probe -- --ignored --nocapture
    #[test]
    #[ignore]
    fn load_keeps_bodies_out_of_memory() {
        let dir = std::env::temp_dir().join(format!("mmail-probe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let key = [7u8; 32];
        let body = "x".repeat(50_000);
        {
            let db = Db::open(&dir, &key).unwrap();
            let messages: Vec<Message> = (1..=6000u32)
                .map(|uid| Message {
                    account: Some("me@ya.ru".into()),
                    folder: Folder::Inbox,
                    uid: Some(uid),
                    starred: false,
                    unread: false,
                    from_name: "Имя Отправителя".into(),
                    from_addr: "sender@example.com".into(),
                    subject: "Тема письма средней длины".into(),
                    timestamp: 1_700_000_000 + i64::from(uid),
                    auth: Auth::Unknown,
                    html: Some(body.clone()),
                    message_id: Some("<id@example.com>".into()),
                    references: Vec::new(),
                    has_attachments: false,
                    attachments: Vec::new(),
                    unavailable: false,
                    recipients: None,
                })
                .collect();
            db.insert(&messages.iter().collect::<Vec<_>>()).unwrap();
        }
        crate::memory::trim();
        let before = rss_mb();
        let store = Store::default();
        let db = Db::open(&dir, &key).unwrap();
        println!("после open: {} МБ", rss_mb() - before);
        let (loaded, _) = db.load().unwrap();
        println!("после load: {} МБ ({} писем)", rss_mb() - before, loaded.len());
        drop(loaded);
        store.attach_db(db).unwrap();
        println!("после attach_db: {} МБ", rss_mb() - before);
        let peak = || {
            let status = std::fs::read_to_string("/proc/self/status").unwrap();
            status.lines().find(|l| l.starts_with("VmHWM:")).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap() / 1024
        };
        // «5» в clear_refs сбрасывает пиковое RSS процесса.
        let reset_peak = || std::fs::write("/proc/self/clear_refs", "5").unwrap();
        reset_peak();
        store.clear_bodies(0).unwrap();
        println!("очистка, стирать нечего: пик {} МБ над исходным", peak() - before);
        reset_peak();
        store.clear_bodies(1_700_003_000).unwrap();
        println!("очистка половины (первое сжатие, полный VACUUM): пик {} МБ над исходным", peak() - before);
        reset_peak();
        store.clear_bodies(1_700_005_000).unwrap();
        println!("следующая очистка (incremental_vacuum): пик {} МБ над исходным", peak() - before);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
