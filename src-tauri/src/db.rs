//! Кэш писем на диске: SQLCipher (AES-256, шифруется весь файл, включая журнал WAL).
//! Ключ — 32 случайных байта в KWallet; без него файл не прочитать. Сервер остаётся
//! источником правды: потерянный ключ означает лишь, что кэш соберётся заново.

use std::fs;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection};
use zeroize::Zeroizing;

use crate::imap::Result;
use crate::store::{Auth, Folder, Message, Opened};

const SCHEMA_VERSION: i32 = 1;

pub struct Db(Mutex<Connection>);

impl Db {
    /// Открывает (или создаёт) базу. Если файл зашифрован другим ключом — он бесполезен, создаём заново.
    pub fn open(dir: &Path, key: &[u8; 32]) -> Result<Self> {
        fs::create_dir_all(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        let path = dir.join("mail.db");
        let conn = match Self::connect(&path, key) {
            Ok(conn) => conn,
            Err(e) if path.exists() => {
                eprintln!("mMail: кэш не открывается этим ключом ({e}) — создаю заново");
                for suffix in ["", "-wal", "-shm"] {
                    let _ = fs::remove_file(format!("{}{suffix}", path.display()));
                }
                Self::connect(&path, key)?
            }
            Err(e) => return Err(e),
        };
        // Журналы -wal/-shm SQLite создаёт с правами самого файла базы; уже созданные — поправляем.
        for suffix in ["", "-wal", "-shm"] {
            let file = format!("{}{suffix}", path.display());
            if Path::new(&file).exists() {
                fs::set_permissions(&file, fs::Permissions::from_mode(0o600))?;
            }
        }
        Ok(Self(Mutex::new(conn)))
    }

    fn connect(path: &Path, key: &[u8; 32]) -> Result<Connection> {
        // Файл создаём сами с правами 0600, а не SQLite по umask (0644).
        fs::OpenOptions::new().append(true).create(true).mode(0o600).open(path)?;
        let conn = Connection::open(path)?;
        let hex = Zeroizing::new(key.iter().map(|b| format!("{b:02x}")).collect::<String>());
        // Сырой ключ (x'…'), без PBKDF2: он и так случайный, а не пароль.
        conn.execute_batch(&Zeroizing::new(format!("PRAGMA key = \"x'{}'\";", hex.as_str())))?;
        // Неверный ключ обнаруживается при первом чтении.
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA secure_delete = ON;")?;
        migrate(&conn)?;
        Ok(conn)
    }

    pub fn load(&self) -> Result<(Vec<Message>, Vec<(String, Folder, u32)>)> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT account, folder, uid, unread, starred, from_name, from_addr, subject, timestamp, auth,
                    has_attachments, html, message_id, refs, attachments FROM messages",
        )?;
        let messages = stmt
            .query_map([], |r| {
                Ok(Message {
                    account: Some(r.get(0)?),
                    folder: folder_from(r.get(1)?),
                    uid: Some(r.get(2)?),
                    unread: r.get(3)?,
                    starred: r.get(4)?,
                    from_name: r.get(5)?,
                    from_addr: r.get(6)?,
                    subject: r.get(7)?,
                    timestamp: r.get(8)?,
                    auth: auth_from(&r.get::<_, String>(9)?),
                    has_attachments: r.get(10)?,
                    html: r.get(11)?,
                    message_id: r.get(12)?,
                    references: serde_json::from_str(&r.get::<_, String>(13)?).unwrap_or_default(),
                    attachments: serde_json::from_str(&r.get::<_, String>(14)?).unwrap_or_default(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt = conn.prepare("SELECT account, folder, uidvalidity FROM folders")?;
        let validity = stmt
            .query_map([], |r| Ok((r.get(0)?, folder_from(r.get(1)?), r.get(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((messages, validity))
    }

    /// Новые письма; уже известные (тот же ящик, папка и UID) не трогаем — у них может быть загружено тело.
    pub fn insert(&self, messages: &[&Message]) -> Result<()> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO messages (account, folder, uid, unread, starred, from_name, from_addr, subject,
                    timestamp, auth, has_attachments, html, message_id, refs, attachments)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            )?;
            for m in messages {
                let (Some(account), Some(uid)) = (&m.account, m.uid) else { continue };
                stmt.execute(params![
                    account,
                    folder_to(m.folder),
                    uid,
                    m.unread,
                    m.starred,
                    m.from_name,
                    m.from_addr,
                    m.subject,
                    m.timestamp,
                    auth_to(m.auth),
                    m.has_attachments,
                    m.html,
                    m.message_id,
                    serde_json::to_string(&m.references)?,
                    serde_json::to_string(&m.attachments)?,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Флаги: `(uid, непрочитано, важное)`.
    pub fn set_flags(&self, account: &str, folder: Folder, flags: &[(u32, bool, bool)]) -> Result<()> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare("UPDATE messages SET unread = ?4, starred = ?5 WHERE account = ?1 AND folder = ?2 AND uid = ?3")?;
            for &(uid, unread, starred) in flags {
                stmt.execute(params![account, folder_to(folder), uid, unread, starred])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn set_opened(&self, account: &str, folder: Folder, uid: u32, opened: &Opened) -> Result<()> {
        self.0.lock().unwrap().execute(
            "UPDATE messages SET html = ?4, auth = ?5, message_id = ?6, refs = ?7, attachments = ?8, has_attachments = ?9
             WHERE account = ?1 AND folder = ?2 AND uid = ?3",
            params![
                account,
                folder_to(folder),
                uid,
                opened.html,
                auth_to(opened.auth),
                opened.message_id,
                serde_json::to_string(&opened.references)?,
                serde_json::to_string(&opened.attachments)?,
                !opened.attachments.is_empty(),
            ],
        )?;
        Ok(())
    }

    /// Стирает содержимое писем старше `cutoff` (или всех) и ужимает файл. Возвращает размер на диске.
    pub fn clear_bodies(&self, cutoff: i64) -> Result<u64> {
        let conn = self.0.lock().unwrap();
        conn.execute("UPDATE messages SET html = NULL WHERE html IS NOT NULL AND timestamp < ?1", [cutoff])?;
        // Без VACUUM SQLite лишь пометит страницы свободными, и файл не уменьшится.
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(self.size_of(&conn))
    }

    /// Сколько кэш занимает на диске (база и журнал).
    pub fn size(&self) -> u64 {
        self.size_of(&self.0.lock().unwrap())
    }

    fn size_of(&self, conn: &Connection) -> u64 {
        let Some(path) = conn.path() else { return 0 };
        ["", "-wal"].iter().filter_map(|suffix| fs::metadata(format!("{path}{suffix}")).ok()).map(|m| m.len()).sum()
    }

    /// Письма папки, которых больше нет на сервере.
    pub fn delete(&self, account: &str, folder: Folder, uids: &[u32]) -> Result<()> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare("DELETE FROM messages WHERE account = ?1 AND folder = ?2 AND uid = ?3")?;
            for uid in uids {
                stmt.execute(params![account, folder_to(folder), uid])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// UIDVALIDITY сменился — прежние UID папки ничего не значат: очищаем её и запоминаем новый.
    pub fn reset_folder(&self, account: &str, folder: Folder, validity: u32) -> Result<()> {
        let mut conn = self.0.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM messages WHERE account = ?1 AND folder = ?2", params![account, folder_to(folder)])?;
        tx.execute(
            "INSERT OR REPLACE INTO folders (account, folder, uidvalidity) VALUES (?1, ?2, ?3)",
            params![account, folder_to(folder), validity],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn remove_account(&self, account: &str) -> Result<()> {
        let conn = self.0.lock().unwrap();
        conn.execute("DELETE FROM messages WHERE account = ?1", [account])?;
        conn.execute("DELETE FROM folders WHERE account = ?1", [account])?;
        Ok(())
    }

    #[cfg(test)]
    fn validity(&self, account: &str, folder: Folder) -> Option<u32> {
        use rusqlite::OptionalExtension;
        let conn = self.0.lock().unwrap();
        conn.query_row(
            "SELECT uidvalidity FROM folders WHERE account = ?1 AND folder = ?2",
            params![account, folder_to(folder)],
            |r| r.get(0),
        )
        .optional()
        .unwrap()
    }
}

fn migrate(conn: &Connection) -> Result<()> {
    let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version >= SCHEMA_VERSION {
        return Ok(());
    }
    conn.execute_batch(
        "CREATE TABLE messages (
            account TEXT NOT NULL,
            folder INTEGER NOT NULL,
            uid INTEGER NOT NULL,
            unread INTEGER NOT NULL,
            starred INTEGER NOT NULL,
            from_name TEXT NOT NULL,
            from_addr TEXT NOT NULL,
            subject TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            auth TEXT NOT NULL,
            has_attachments INTEGER NOT NULL,
            html TEXT,
            message_id TEXT,
            refs TEXT NOT NULL,
            attachments TEXT NOT NULL,
            PRIMARY KEY (account, folder, uid)
        ) WITHOUT ROWID;
        CREATE TABLE folders (
            account TEXT NOT NULL,
            folder INTEGER NOT NULL,
            uidvalidity INTEGER NOT NULL,
            PRIMARY KEY (account, folder)
        ) WITHOUT ROWID;
        PRAGMA user_version = 1;",
    )?;
    Ok(())
}

fn folder_to(folder: Folder) -> i64 {
    match folder {
        Folder::Inbox => 0,
        Folder::Sent => 1,
        Folder::Drafts => 2,
        Folder::Trash => 3,
    }
}

fn folder_from(n: i64) -> Folder {
    match n {
        1 => Folder::Sent,
        2 => Folder::Drafts,
        3 => Folder::Trash,
        _ => Folder::Inbox,
    }
}

fn auth_to(auth: Auth) -> &'static str {
    match auth {
        Auth::Pass => "pass",
        Auth::Fail => "fail",
        Auth::Unknown => "unknown",
    }
}

fn auth_from(s: &str) -> Auth {
    match s {
        "pass" => Auth::Pass,
        "fail" => Auth::Fail,
        _ => Auth::Unknown,
    }
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
            from_name: "Секретный отправитель".into(),
            from_addr: "a@b.example".into(),
            subject: "Совершенно секретная тема".into(),
            timestamp: 1_700_000_000,
            auth: Auth::Unknown,
            html: None,
            message_id: None,
            references: Vec::new(),
            has_attachments: false,
            attachments: Vec::new(),
        }
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mmail-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn file_is_encrypted_and_round_trips() {
        let dir = temp_dir("roundtrip");
        let key = [7u8; 32];
        let db = Db::open(&dir, &key).unwrap();
        db.insert(&[&message(1), &message(2)]).unwrap();
        db.set_flags("me@ya.ru", Folder::Inbox, &[(2, false, true)]).unwrap();
        db.reset_folder("me@ya.ru", Folder::Sent, 42).unwrap();
        drop(db);

        let bytes = fs::read(dir.join("mail.db")).unwrap();
        let wal = fs::read(dir.join("mail.db-wal")).unwrap_or_default();
        for haystack in [&bytes, &wal] {
            assert!(!haystack.windows(6).any(|w| w == b"SQLite"), "нет открытого заголовка SQLite");
            let needle = "секретная".as_bytes();
            assert!(!haystack.windows(needle.len()).any(|w| w == needle), "тема письма не видна на диске");
        }
        for file in ["mail.db", "mail.db-wal"] {
            let path = dir.join(file);
            if path.exists() {
                assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "{file}");
            }
        }

        let db = Db::open(&dir, &key).unwrap();
        let (messages, _) = db.load().unwrap();
        assert_eq!(messages.len(), 2);
        let second = messages.iter().find(|m| m.uid == Some(2)).unwrap();
        assert!(!second.unread && second.starred);
        assert_eq!(db.validity("me@ya.ru", Folder::Sent), Some(42));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn clearing_keeps_headers_and_drops_old_bodies() {
        let dir = temp_dir("clear");
        let db = Db::open(&dir, &[3u8; 32]).unwrap();
        let mut old = message(1);
        old.html = Some("старое".into());
        let mut new = message(2);
        new.html = Some("новое".into());
        new.timestamp += 1000;
        db.insert(&[&old, &new]).unwrap();
        db.clear_bodies(old.timestamp + 1).unwrap();
        let (messages, _) = db.load().unwrap();
        assert_eq!(messages.len(), 2, "заголовки остаются");
        let html = |uid| messages.iter().find(|m| m.uid == Some(uid)).unwrap().html.clone();
        assert_eq!(html(1), None);
        assert_eq!(html(2).as_deref(), Some("новое"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_key_starts_a_fresh_cache() {
        let dir = temp_dir("wrongkey");
        Db::open(&dir, &[1u8; 32]).unwrap().insert(&[&message(1)]).unwrap();
        let db = Db::open(&dir, &[2u8; 32]).unwrap();
        assert!(db.load().unwrap().0.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
