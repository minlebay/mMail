//! Папки ящика на сервере: какие синхронизировать, как называть в дереве.

use async_imap::types::{Name, NameAttribute};
use base64::Engine;

use crate::store::Folder;

/// Папка на сервере: роль или имя, имя для команд IMAP и имя для людей.
#[derive(Clone)]
pub struct ServerFolder {
    pub folder: Folder,
    pub server_name: String,
    pub display: String,
}

/// Роль папки по атрибутам LIST; `None` — папку не показываем и не синхронизируем.
pub fn classify(name: &Name) -> Option<Folder> {
    let attributes = name.attributes();
    let has = |wanted: &NameAttribute| attributes.iter().any(|a| a == wanted);
    // \All (Gmail «Вся почта»), \Flagged и \Important — повторяют письма других папок:
    // синхронизировать их — качать всё дважды. «Важные» у нас и так есть по флагу.
    if has(&NameAttribute::NoSelect) || has(&NameAttribute::All) || has(&NameAttribute::Flagged) {
        return None;
    }
    if attributes.iter().any(|a| matches!(a, NameAttribute::Extension(e) if e.eq_ignore_ascii_case("\\NonExistent") || e.eq_ignore_ascii_case("\\Important"))) {
        return None;
    }
    if name.name().eq_ignore_ascii_case("INBOX") {
        return Some(Folder::Inbox);
    }
    Some(attributes.iter().find_map(|a| match a {
        NameAttribute::Sent => Some(Folder::Sent),
        NameAttribute::Drafts => Some(Folder::Drafts),
        NameAttribute::Trash => Some(Folder::Trash),
        NameAttribute::Junk => Some(Folder::Junk),
        NameAttribute::Archive => Some(Folder::Archive),
        _ => None,
    }).unwrap_or_else(|| Folder::other(name.name())))
}

/// Имя для дерева: декодированное, с вложенностью через « / ».
pub fn display_name(server_name: &str, delimiter: Option<&str>) -> String {
    let decoded = decode_mutf7(server_name);
    match delimiter.filter(|d| !d.is_empty()) {
        Some(d) => decoded.split(d).filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" / "),
        None => decoded,
    }
}

/// Название стандартной папки (ключ каталога переводов); `None` у своих папок.
pub fn special_name(folder: Folder) -> Option<&'static str> {
    Some(match folder {
        Folder::Inbox => "Входящие",
        Folder::Sent => "Отправленные",
        Folder::Drafts => "Черновики",
        Folder::Junk => "Спам",
        Folder::Trash => "Корзина",
        Folder::Archive => "Архив",
        Folder::Other(_) => return None,
    })
}

/// Порядок в дереве и при синхронизации: стандартные папки первыми, остальные по имени.
pub fn rank(folder: Folder) -> u8 {
    match folder {
        Folder::Inbox => 0,
        Folder::Sent => 1,
        Folder::Drafts => 2,
        Folder::Junk => 3,
        Folder::Trash => 4,
        Folder::Archive => 5,
        Folder::Other(_) => 6,
    }
}

/// Имена папок IMAP — «modified UTF-7» (RFC 3501, 5.1.3): `&` открывает base64 от UTF-16BE,
/// с `,` вместо `/`; `&-` — сам амперсанд. Битые последовательности оставляем как есть.
pub fn decode_mutf7(name: &str) -> String {
    let mut out = String::new();
    let mut rest = name;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('-') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let encoded = &after[..end];
        if encoded.is_empty() {
            out.push('&');
        } else {
            let decoded = base64::engine::general_purpose::STANDARD_NO_PAD
                .decode(encoded.replace(',', "/"))
                .ok()
                .filter(|bytes| bytes.len() % 2 == 0)
                .and_then(|bytes| {
                    let units: Vec<u16> = bytes.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                    String::from_utf16(&units).ok()
                });
            match decoded {
                Some(text) => out.push_str(&text),
                None => out.push_str(&rest[start..start + 1 + end + 1]),
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_modified_utf7() {
        assert_eq!(decode_mutf7("INBOX"), "INBOX");
        assert_eq!(decode_mutf7("&BB4EQgQ,BEAEMAQyBDsENQQ9BD0ESwQ1-"), "Отправленные");
        assert_eq!(decode_mutf7("Tom &- Jerry"), "Tom & Jerry");
        assert_eq!(decode_mutf7("&BCAEMAQxBD4EQgQw-/2026"), "Работа/2026");
        assert_eq!(decode_mutf7("broken&@@@-"), "broken&@@@-");
    }

    #[test]
    fn nested_names_use_the_server_delimiter() {
        assert_eq!(display_name("Work|Projects", Some("|")), "Work / Projects");
        assert_eq!(display_name("[Gmail]/Spam", Some("/")), "[Gmail] / Spam");
    }
}
