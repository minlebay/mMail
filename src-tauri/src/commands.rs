use serde::Serialize;
use tauri::State;

use crate::config::ConfigStore;
use crate::imap::Imap;
use crate::store::{Auth, Scope, Store, Summary};
use crate::{links, remote, sanitize};

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

/// Узел дерева папок: ящик (или «Все ящики», `account: None`) и его папки.
#[derive(Serialize)]
pub struct FolderGroup {
    account: Option<String>,
    title: String,
    folders: Vec<FolderInfo>,
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
    /// Ящик письма — для «Ответить» с того же адреса; `None` у демо.
    account: Option<String>,
    from_name: String,
    from_addr: String,
    subject: String,
    date: String,
    auth: Auth,
    warnings: Vec<String>,
    blocked_remote: usize,
    /// Внешние картинки загружены и встроены.
    remote_shown: bool,
    /// Отправитель в списке доверенных.
    trusted: bool,
    /// Можно ли доверять: не для провалившихся проверку и не для доменов с омоглифами.
    can_trust: bool,
    attachments: Vec<crate::attachments::AttachmentMeta>,
    srcdoc: String,
}

#[tauri::command]
pub fn list_folders(store: State<'_, Store>, imap: State<'_, Imap>) -> Vec<FolderGroup> {
    let folders = |scope: Scope| -> Vec<FolderInfo> {
        FOLDERS.iter().map(|&(id, name)| FolderInfo { id, name, unread: store.unread(scope, id) }).collect()
    };
    // Ящики из хранилища плюс те, что ещё подключаются и писем пока не прислали.
    let mut accounts = store.accounts();
    for email in imap.known_accounts() {
        if !accounts.contains(&Some(email.clone())) {
            accounts.push(Some(email));
        }
    }
    accounts.sort();

    let mut groups = Vec::new();
    if accounts.len() > 1 {
        groups.push(FolderGroup { account: None, title: "Все ящики".into(), folders: folders(Scope::All) });
    }
    for account in accounts {
        let (key, title) = match account {
            Some(email) => (email.clone(), email),
            None => (String::new(), "Демо-ящик".into()),
        };
        let folders = folders(Scope::parse(Some(&key)));
        groups.push(FolderGroup { account: Some(key), title, folders });
    }
    groups
}

#[tauri::command]
pub fn list_messages(
    store: State<'_, Store>,
    account: Option<String>,
    folder: String,
    query: String,
    offset: usize,
    limit: usize,
) -> Result<Page, String> {
    let query = query.trim().to_lowercase();
    let (total, items) = store.page(Scope::parse(account.as_deref()), &folder, &query, offset, limit.min(MAX_PAGE)).ok_or("unknown folder")?;
    Ok(Page { total, items })
}

#[tauri::command]
pub async fn get_message(
    app: tauri::AppHandle,
    store: State<'_, Store>,
    imap: State<'_, Imap>,
    config: State<'_, ConfigStore>,
    id: u32,
    show_remote: bool,
) -> Result<MessageView, String> {
    let mut m = store.details(id).ok_or("no such message")?;
    let html = match (m.html.take(), m.account.as_deref(), m.uid) {
        (Some(html), _, _) => html,
        (None, Some(account), Some(uid)) => {
            let opened = imap.open(account, m.folder, uid).await.map_err(|e| e.to_string())?;
            let html = opened.html.clone();
            m.auth = opened.auth;
            m.attachments = opened.attachments.clone();
            store.set_opened(id, opened);
            html
        }
        _ => String::new(),
    };
    if m.unread {
        // Открыл — значит прочитал, как в любом почтовом клиенте; демо-письма только локально.
        if let (Some(account), Some(uid)) = (m.account.as_deref(), m.uid) {
            if let Err(e) = imap.mark_seen(account, m.folder, uid).await {
                eprintln!("mMail: не удалось пометить прочитанным: {e}");
            }
        }
        store.set_unread(id, false);
        let _ = tauri::Emitter::emit(&app, "mailbox-changed", ());
    }
    let sender = m.from_addr.to_lowercase();
    let lookalike = sender.rsplit_once('@').is_some_and(|(_, domain)| !domain.is_ascii());
    let can_trust = m.auth != Auth::Fail && !lookalike && !sender.is_empty();
    let trusted = can_trust && config.get().trusted_senders.contains(&sender);
    let remote_shown = show_remote || trusted;

    let mut clean = sanitize::clean_html(&html, remote_shown);
    if remote_shown && clean.blocked_remote > 0 {
        clean.html = remote::inline(clean.html).await;
    }

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
    warnings.extend(deceptive_warnings(links::find_deceptive(&clean.html)));

    Ok(MessageView {
        id: m.id,
        account: m.account,
        from_name: m.from_name,
        from_addr: m.from_addr,
        subject: m.subject,
        date: m.date,
        auth: m.auth,
        warnings,
        blocked_remote: clean.blocked_remote,
        remote_shown,
        trusted,
        can_trust,
        attachments: m.attachments,
        srcdoc: sanitize::wrap(&clean.html),
    })
}

/// «Всегда показывать внешнее содержимое от этого отправителя» и обратно.
#[tauri::command]
pub fn set_sender_trusted(config: State<'_, ConfigStore>, address: String, trusted: bool) -> Result<(), String> {
    let address = address.trim().to_lowercase();
    if !address.contains('@') {
        return Err("некорректный адрес".into());
    }
    config.update(|c| {
        if trusted {
            c.trusted_senders.insert(address);
        } else {
            c.trusted_senders.remove(&address);
        }
    })
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Appearance {
    theme: String,
    opacity: Option<f32>,
    middle_click_minimize: bool,
    zoom: f64,
    poll_seconds: u64,
    tray_mode: &'static str,
    close_to_tray: bool,
}

pub const ZOOM_MIN: f64 = 0.5;
pub const ZOOM_MAX: f64 = 2.0;

#[tauri::command]
pub fn get_appearance(config: State<'_, ConfigStore>) -> Appearance {
    let c = config.get();
    let poll_seconds = c.poll_seconds();
    let tray_mode = c.tray_mode().as_str();
    Appearance {
        theme: c.theme,
        opacity: c.opacity,
        middle_click_minimize: c.middle_click_minimize.unwrap_or(true),
        zoom: c.zoom.unwrap_or(1.0),
        poll_seconds,
        tray_mode,
        close_to_tray: c.close_to_tray.unwrap_or(true),
    }
}

#[tauri::command]
pub fn set_appearance(
    app: tauri::AppHandle,
    config: State<'_, ConfigStore>,
    theme: String,
    opacity: f32,
    middle_click_minimize: bool,
    zoom: f64,
    poll_seconds: u64,
    tray_mode: String,
    close_to_tray: bool,
) -> Result<(), String> {
    if !crate::config::POLL_CHOICES.contains(&poll_seconds) {
        return Err("недопустимый интервал проверки".into());
    }
    let tray_mode = crate::config::TrayMode::parse(&tray_mode).ok_or("неизвестный режим значка")?;
    if theme.len() > 32 || !theme.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("некорректная тема".into());
    }
    config.update(|c| {
        c.theme = theme;
        c.opacity = Some(opacity.clamp(0.15, 0.85));
        c.middle_click_minimize = Some(middle_click_minimize);
        c.zoom = Some(zoom.clamp(ZOOM_MIN, ZOOM_MAX));
        c.poll_seconds = Some(poll_seconds);
        c.tray_mode = Some(tray_mode.as_str().into());
        c.close_to_tray = Some(close_to_tray);
    })?;
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Err(e) = crate::tray::apply(&handle) {
            eprintln!("mMail: трей: {e}");
        }
    });
    // Окна писем подхватывают тему сразу, без переоткрытия.
    let _ = tauri::Emitter::emit(&app, "appearance-changed", get_appearance(config));
    Ok(())
}

/// Текущий масштаб окна (быстрый ползунок в шапке и Ctrl +/−/0).
#[tauri::command]
pub fn set_zoom(window: tauri::WebviewWindow, scale: f64) -> Result<(), String> {
    if !scale.is_finite() {
        return Err("некорректный масштаб".into());
    }
    window.set_zoom(scale.clamp(ZOOM_MIN, ZOOM_MAX)).map_err(|e| e.to_string())
}

/// Одно предупреждение на каждый реальный адрес, а не на каждую ссылку.
fn deceptive_warnings(found: Vec<links::Deceptive>) -> Vec<String> {
    let mut by_host: Vec<(String, Vec<String>)> = Vec::new();
    for link in found {
        let quoted = format!("«{}»", link.text);
        match by_host.iter_mut().find(|(host, _)| *host == link.real_host) {
            Some((_, texts)) if !texts.contains(&quoted) => texts.push(quoted),
            Some(_) => {}
            None => by_host.push((link.real_host, vec![quoted])),
        }
    }
    by_host
        .into_iter()
        .map(|(host, texts)| match texts.len() {
            1 => format!("Ссылка {} на самом деле ведёт на {host}", texts[0]),
            n if n <= 3 => format!("Ссылки {} на самом деле ведут на {host}", texts.join(", ")),
            n => format!("Ссылки {} и ещё {} на самом деле ведут на {host}", texts[..3].join(", "), n - 3),
        })
        .collect()
}

/// Сохраняет вложение: письмо заново берётся с сервера, путь выбирают в системном диалоге.
/// Возвращает false, если диалог закрыли без выбора.
#[tauri::command]
pub async fn save_attachment(app: tauri::AppHandle, store: State<'_, Store>, imap: State<'_, Imap>, id: u32, index: usize) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;

    let m = store.details(id).ok_or("no such message")?;
    let (Some(account), Some(uid)) = (m.account.as_deref(), m.uid) else { return Err("у демо-писем нет вложений".into()) };
    let raw = imap.raw(account, m.folder, uid).await.map_err(|e| e.to_string())?;
    let parsed = mail_parser::MessageParser::default().parse(&raw).ok_or("не удалось разобрать письмо")?;
    let parts = crate::attachments::visible(&parsed);
    let part = parts.get(index).ok_or("вложение не найдено")?;
    let name = crate::attachments::safe_name(mail_parser::MimeHeaders::attachment_name(*part).unwrap_or("вложение"));

    let Some(target) = app.dialog().file().set_title("Сохранить вложение").set_file_name(&name).blocking_save_file() else {
        return Ok(false);
    };
    let path = target.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&path, part.contents()).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}
