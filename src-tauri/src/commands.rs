use serde::Serialize;
use tauri::State;

use crate::imap::Imap;
use crate::store::{Auth, Store, Summary};
use crate::{links, sanitize};

const FOLDERS: [(&str, &str); 5] = [
    ("inbox", "Входящие"),
    ("starred", "Важные"),
    ("sent", "Отправленные"),
    ("drafts", "Черновики"),
    ("trash", "Корзина"),
];

const MAX_PAGE: usize = 500;

#[derive(Serialize)]
pub struct FolderInfo {
    id: &'static str,
    name: &'static str,
    unread: usize,
}

#[derive(Serialize)]
pub struct Page {
    total: usize,
    items: Vec<Summary>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageView {
    id: u32,
    from_name: String,
    from_addr: String,
    subject: String,
    date: String,
    auth: Auth,
    warnings: Vec<String>,
    blocked_remote: usize,
    srcdoc: String,
}

#[tauri::command]
pub fn list_folders(store: State<'_, Store>) -> Vec<FolderInfo> {
    FOLDERS.iter().map(|&(id, name)| FolderInfo { id, name, unread: store.unread(id) }).collect()
}

#[tauri::command]
pub fn list_messages(
    store: State<'_, Store>,
    folder: String,
    query: String,
    offset: usize,
    limit: usize,
) -> Result<Page, String> {
    let query = query.trim().to_lowercase();
    let (total, items) = store.page(&folder, &query, offset, limit.min(MAX_PAGE)).ok_or("unknown folder")?;
    Ok(Page { total, items })
}

#[tauri::command]
pub async fn get_message(store: State<'_, Store>, imap: State<'_, Imap>, id: u32) -> Result<MessageView, String> {
    let mut m = store.details(id).ok_or("no such message")?;
    let html = match (m.html.take(), m.uid) {
        (Some(html), _) => html,
        (None, Some(uid)) => {
            let (html, auth) = imap.open(m.folder, uid).await.map_err(|e| e.to_string())?;
            store.set_opened(id, html.clone(), auth);
            m.auth = auth;
            html
        }
        (None, None) => String::new(),
    };
    let clean = sanitize::clean_html(&html);

    let mut warnings = Vec::new();
    if m.auth == Auth::Fail {
        warnings.push("Проверка отправителя (SPF/DKIM/DMARC) не пройдена — письмо может быть подделкой".into());
    }
    if let Some((_, domain)) = m.from_addr.rsplit_once('@') {
        if !domain.is_ascii() {
            warnings.push(format!(
                "Домен «{domain}» содержит не-латинские символы — возможна подмена адреса"
            ));
        }
    }
    for link in links::find_deceptive(&clean.html) {
        warnings.push(format!("Ссылка «{}» на самом деле ведёт на {}", link.text, link.real_host));
    }

    Ok(MessageView {
        id: m.id,
        from_name: m.from_name,
        from_addr: m.from_addr,
        subject: m.subject,
        date: m.date,
        auth: m.auth,
        warnings,
        blocked_remote: clean.blocked_remote,
        srcdoc: sanitize::wrap(&clean.html),
    })
}
