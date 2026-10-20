use serde::Serialize;
use tauri::{Manager, State};

use crate::config::ConfigStore;
use crate::imap::Imap;
use crate::folders;
use crate::store::{Auth, Folder, Scope, Store, Summary};
use crate::{links, remote, sanitize};

/// Папки, которые показываем всегда, даже пока ящик не подключился (и в «Все ящики»).
const STANDARD: [Folder; 4] = [Folder::Inbox, Folder::Sent, Folder::Drafts, Folder::Trash];
/// «Важные» — не папка на сервере, а все письма с флагом.
const STARRED: (&str, &str) = ("starred", "Важные");

const MAX_PAGE: usize = 500;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderInfo {
    /// `inbox`, `starred`, … или `f:<имя на сервере>`.
    id: String,
    /// Для стандартных — ключ каталога переводов, для своих — имя с сервера.
    name: String,
    /// Своя папка: имя не переводится.
    custom: bool,
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
    in_trash: bool,
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
pub fn list_folders(store: State<'_, Store>, imap: State<'_, Imap>, config: State<'_, ConfigStore>) -> Vec<FolderGroup> {
    let info = |scope: Scope, folder: Folder, display: Option<String>| {
        let id = folder.key();
        let unread = store.unread(scope, &id);
        match folders::special_name(folder) {
            Some(name) => FolderInfo { id, name: name.into(), custom: false, unread },
            None => FolderInfo { name: display.unwrap_or_else(|| folders::decode_mutf7(&id[2..])), id, custom: true, unread },
        }
    };
    let starred = |scope: Scope| FolderInfo {
        id: STARRED.0.into(),
        name: STARRED.1.into(),
        custom: false,
        unread: store.unread(scope, STARRED.0),
    };
    // Папки ящика: с сервера, а пока он не подключился — стандартные и те, что есть в кэше.
    let folders = |scope: Scope, email: Option<&str>| -> Vec<FolderInfo> {
        let mut list: Vec<(Folder, Option<String>)> = match email.and_then(|e| imap.folders(e)) {
            Some(server) => server.into_iter().map(|f| (f.folder, Some(f.display))).collect(),
            None => {
                let mut known: Vec<Folder> = STANDARD.to_vec();
                if let Some(email) = email {
                    let extra: Vec<Folder> = store.folders_of(email).into_iter().filter(|f| !known.contains(f)).collect();
                    known.extend(extra);
                }
                known.sort_by_key(|f| (folders::rank(*f), f.key().to_lowercase()));
                known.into_iter().map(|f| (f, None)).collect()
            }
        };
        if email.is_none() {
            // «Все ящики»: только общие для всех ящиков роли.
            list.retain(|(f, _)| !matches!(f, Folder::Other(_)));
            for f in [Folder::Junk, Folder::Archive] {
                if store.has_folder(f) && !list.iter().any(|(g, _)| *g == f) {
                    list.push((f, None));
                }
            }
            list.sort_by_key(|(f, _)| folders::rank(*f));
        }
        let mut out: Vec<FolderInfo> = list.into_iter().map(|(f, display)| info(scope, f, display)).collect();
        out.insert(1.min(out.len()), starred(scope));
        out
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
    if accounts.len() > 1 && config.get().show_all_accounts.unwrap_or(true) {
        groups.push(FolderGroup { account: None, title: "Все ящики".into(), folders: folders(Scope::All, None) });
    }
    for account in accounts {
        let (key, title) = match account {
            Some(email) => (email.clone(), email),
            None => (String::new(), "Демо-ящик".into()),
        };
        let email = (!key.is_empty()).then_some(key.as_str());
        let folders = folders(Scope::parse(Some(&key)), email);
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
    unread_only: Option<bool>,
) -> Result<Page, String> {
    let query = query.trim().to_lowercase();
    let (total, items) = store.page(Scope::parse(account.as_deref()), &folder, &query, offset, limit.min(MAX_PAGE), unread_only.unwrap_or(false)).ok_or("unknown folder")?;
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
    let RemotePolicy { can_trust, trusted, remote_shown } =
        remote_policy(&m.from_addr, m.auth, &config.get().trusted_senders, show_remote);

    let mut clean = sanitize::clean_message(&html, remote_shown);
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
        in_trash: m.folder == crate::store::Folder::Trash,
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

#[derive(Debug, PartialEq)]
struct RemotePolicy {
    /// Можно ли добавить отправителя в доверенные.
    can_trust: bool,
    /// Отправитель в списке доверенных.
    trusted: bool,
    /// Загружать ли внешнее содержимое.
    remote_shown: bool,
}

/// Внешнее содержимое — по кнопке; само, без кнопки — только доверенному отправителю с
/// подтверждённой подлинностью: адрес From подделать легко, а загрузка картинок выдаёт
/// отправителю IP и время прочтения. Не доверяем провалившим проверку и доменам с омоглифами.
fn remote_policy(from_addr: &str, auth: Auth, trusted_senders: &std::collections::BTreeSet<String>, show_remote: bool) -> RemotePolicy {
    let sender = from_addr.to_lowercase();
    let lookalike = sender.rsplit_once('@').is_some_and(|(_, domain)| !domain.is_ascii());
    let can_trust = auth != Auth::Fail && !lookalike && !sender.is_empty();
    let trusted = can_trust && trusted_senders.contains(&sender);
    RemotePolicy { can_trust, trusted, remote_shown: show_remote || (trusted && auth == Auth::Pass) }
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
    wide_layout: bool,
    blur_enabled: bool,
    language: String,
    theme: String,
    scheme: String,
    /// Схема системы сейчас — для режима `system` (меняется на лету, см. scheme.rs).
    system_dark: bool,
    opacity: Option<f32>,
    middle_click_minimize: bool,
    zoom: f64,
    poll_seconds: u64,
    tray_mode: &'static str,
    tray_icon: String,
    close_to_tray: bool,
    show_all_accounts: bool,
    notify_mail: bool,
    notify_events: bool,
    window_buttons: bool,
    pin_buttons: bool,
    /// Вид календаря при запуске: month | week | day.
    calendar_view: String,
    /// Названия событий в месяце — целиком, в несколько строк.
    calendar_full_titles: bool,
}

pub const ZOOM_MIN: f64 = 0.5;
pub const ZOOM_MAX: f64 = 2.0;

#[tauri::command]
pub fn get_appearance(config: State<'_, ConfigStore>) -> Appearance {
    let c = config.get();
    let poll_seconds = c.poll_seconds();
    let tray_mode = c.tray_mode().as_str();
    Appearance {
        wide_layout: c.wide_layout.unwrap_or(false),
        blur_enabled: c.blur_enabled.unwrap_or(true),
        language: c.language().into(),
        theme: c.theme,
        scheme: c.scheme.clone().unwrap_or_else(|| "system".into()),
        system_dark: crate::scheme::system_dark(),
        opacity: c.opacity,
        middle_click_minimize: c.middle_click_minimize.unwrap_or(true),
        zoom: c.zoom.unwrap_or(1.0),
        poll_seconds,
        tray_mode,
        tray_icon: c.tray_icon.clone().unwrap_or_else(|| "color".into()),
        close_to_tray: c.close_to_tray.unwrap_or(true),
        show_all_accounts: c.show_all_accounts.unwrap_or(true),
        notify_mail: c.notify_mail.unwrap_or(true),
        notify_events: c.notify_events.unwrap_or(true),
        window_buttons: c.window_buttons.unwrap_or(true),
        pin_buttons: c.pin_buttons.unwrap_or(true),
        calendar_view: c.calendar_view.clone().unwrap_or_else(|| "week".into()),
        calendar_full_titles: c.calendar_full_titles.unwrap_or(false),
    }
}

/// Уведомления KDE: о новых письмах и напоминания календаря.
#[tauri::command]
pub fn set_notifications(config: State<'_, ConfigStore>, mail: bool, events: bool) -> Result<(), String> {
    config.update(|c| {
        c.notify_mail = Some(mail);
        c.notify_events = Some(events);
    })
}

/// Какие кнопки показывать в шапке окон; все окна перерисовывают шапку по appearance-changed.
#[tauri::command]
pub fn set_title_buttons(app: tauri::AppHandle, config: State<'_, ConfigStore>, window_buttons: bool, pin_buttons: bool) -> Result<(), String> {
    config.update(|c| {
        c.window_buttons = Some(window_buttons);
        c.pin_buttons = Some(pin_buttons);
    })?;
    let _ = tauri::Emitter::emit(&app, "appearance-changed", get_appearance(config));
    Ok(())
}

/// Вид календаря по умолчанию; главное окно переключается по appearance-changed.
#[tauri::command]
pub fn set_calendar_view(app: tauri::AppHandle, config: State<'_, ConfigStore>, view: String) -> Result<(), String> {
    if !["month", "week", "day"].contains(&view.as_str()) {
        return Err("неизвестный вид календаря".into());
    }
    config.update(|c| c.calendar_view = Some(view))?;
    let _ = tauri::Emitter::emit(&app, "appearance-changed", get_appearance(config));
    Ok(())
}

/// Названия событий в месяце целиком; главное окно перерисовывает сетку по appearance-changed.
#[tauri::command]
pub fn set_calendar_full_titles(app: tauri::AppHandle, config: State<'_, ConfigStore>, full: bool) -> Result<(), String> {
    config.update(|c| c.calendar_full_titles = Some(full))?;
    let _ = tauri::Emitter::emit(&app, "appearance-changed", get_appearance(config));
    Ok(())
}

/// Раздел «Все ящики» в дереве папок; главное окно перестраивает дерево по mailbox-changed.
#[tauri::command]
pub fn set_show_all_accounts(app: tauri::AppHandle, config: State<'_, ConfigStore>, show: bool) -> Result<(), String> {
    config.update(|c| c.show_all_accounts = Some(show))?;
    let _ = tauri::Emitter::emit(&app, "mailbox-changed", ());
    Ok(())
}

#[tauri::command]
pub fn set_appearance(
    app: tauri::AppHandle,
    config: State<'_, ConfigStore>,
    theme: String,
    scheme: String,
    language: String,
    opacity: f32,
    blur_enabled: bool,
    wide_layout: bool,
    middle_click_minimize: bool,
    zoom: f64,
    poll_seconds: u64,
    tray_mode: String,
    tray_icon: String,
    close_to_tray: bool,
) -> Result<(), String> {
    if !["ru", "en", "sr"].contains(&language.as_str()) {
        return Err("unsupported interface language".into());
    }
    if !["dark", "light", "system"].contains(&scheme.as_str()) {
        return Err("неизвестная схема".into());
    }
    if !["color", "auto", "light", "dark"].contains(&tray_icon.as_str()) {
        return Err("неизвестный стиль значка".into());
    }
    if !crate::config::POLL_CHOICES.contains(&poll_seconds) {
        return Err("недопустимый интервал проверки".into());
    }
    let tray_mode = crate::config::TrayMode::parse(&tray_mode).ok_or("неизвестный режим значка")?;
    if theme.len() > 32 || !theme.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("некорректная тема".into());
    }
    config.update(|c| {
        c.language = language;
        c.theme = theme;
        c.scheme = Some(scheme);
        c.opacity = Some(opacity.clamp(0.15, 0.85));
        c.blur_enabled = Some(blur_enabled);
        c.wide_layout = Some(wide_layout);
        c.middle_click_minimize = Some(middle_click_minimize);
        c.zoom = Some(zoom.clamp(ZOOM_MIN, ZOOM_MAX));
        c.poll_seconds = Some(poll_seconds);
        c.tray_mode = Some(tray_mode.as_str().into());
        c.close_to_tray = Some(close_to_tray);
        c.tray_icon = Some(tray_icon);
    })?;
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        crate::blur::set_enabled(blur_enabled);
        if let Some(window) = handle.get_webview_window("settings") {
            let _ = window.set_title(&crate::i18n::text(&handle, "Настройки mMail"));
        }
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

    let Some(target) = app.dialog().file().set_title(crate::i18n::text(&app, "Сохранить вложение")).set_file_name(&name).blocking_save_file() else {
        return Ok(false);
    };
    let path = target.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&path, part.contents()).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageInfo {
    /// Размер шифрованного кэша на диске.
    bytes: u64,
    /// `None` — хранить содержимое всех писем.
    body_days: Option<u32>,
}

#[tauri::command]
pub fn storage_info(store: State<'_, Store>, config: State<'_, ConfigStore>) -> StorageInfo {
    StorageInfo { bytes: store.disk_size(), body_days: config.get().body_days }
}

/// Срок хранения содержимого писем; более старое стирается сразу (заголовки остаются).
#[tauri::command]
pub async fn set_body_days(app: tauri::AppHandle, days: Option<u32>) -> Result<StorageInfo, String> {
    if days.is_some_and(|d| !crate::config::BODY_DAYS_CHOICES.contains(&d)) {
        return Err("недопустимый срок хранения".into());
    }
    app.state::<ConfigStore>().update(|c| c.body_days = days)?;
    clear_cache(app).await
}

/// Применяет срок хранения сейчас и ужимает файл кэша.
#[tauri::command]
pub async fn clear_cache(app: tauri::AppHandle) -> Result<StorageInfo, String> {
    let config = app.state::<ConfigStore>().get();
    let (cutoff, body_days) = (config.body_cutoff(), config.body_days);
    let handle = app.clone();
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let store = handle.state::<Store>();
        match cutoff {
            Some(cutoff) => store.clear_bodies(cutoff).map_err(|e| e.to_string()),
            None => Ok(store.disk_size()),
        }
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(StorageInfo { bytes, body_days })
}

/// Окно настроек: одно на приложение, независимое от главного; повторный вызов — показать его.
#[tauri::command]
pub fn open_settings(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.unminimize();
        return window.set_focus().map_err(|e| e.to_string());
    }
    // Размер свободный и запоминается; что не влезло — прокручивается внутри окна.
    let builder = crate::glass_window(&app, "settings", &crate::i18n::text(&app, "Настройки mMail"))
        .inner_size(680.0, 640.0)
        .min_inner_size(480.0, 320.0)
        .maximizable(false);
    let window = crate::window_state::restore(&app, "settings", builder, (480.0, 320.0))
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .build()
        .map_err(|e| e.to_string())?;
    crate::window_state::track(&app, &window, "settings");
    crate::finish_window(&app, &window).map_err(|e| e.to_string())
}

/// Write-only clipboard access for explicit menu actions in the main window.
#[tauri::command]
pub async fn write_clipboard(app: tauri::AppHandle, text: String) -> Result<(), String> {
    if text.len() > 1024 * 1024 { return Err("clipboard text is too large".into()); }
    let (send, receive) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
        clipboard.set_text(&text);
        let _ = send.send(());
    }).map_err(|e| e.to_string())?;
    receive.await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn mark_message_read(app: tauri::AppHandle, store: State<'_, Store>, imap: State<'_, Imap>, id: u32) -> Result<(), String> {
    let message = store.details(id).ok_or("письмо пропало")?;
    if let (Some(account), Some(uid)) = (message.account.as_deref(), message.uid) {
        imap.mark_seen(account, message.folder, uid).await.map_err(|e| e.to_string())?;
    }
    store.set_unread(id, false);
    let _ = tauri::Emitter::emit(&app, "mailbox-changed", ());
    Ok(())
}

#[tauri::command]
pub async fn trash_message(app: tauri::AppHandle, store: State<'_, Store>, imap: State<'_, Imap>, id: u32) -> Result<(), String> {
    let message = store.details(id).ok_or("письмо пропало")?;
    if message.folder == crate::store::Folder::Trash { return Err("Письмо уже в корзине".into()); }
    if let (Some(account), Some(uid)) = (message.account.as_deref(), message.uid) {
        imap.move_to_trash(account, message.folder, uid).await.map_err(|e| e.to_string())?;
    }
    store.trashed(id);
    let _ = tauri::Emitter::emit(&app, "message-trashed", id);
    let _ = tauri::Emitter::emit(&app, "mailbox-changed", ());
    Ok(())
}

/// «Отметить все прочитанными» для папки (в том числе «Важные» и «Все ящики»):
/// на сервер — по папкам, локально — только то, что сервер подтвердил.
#[tauri::command]
pub async fn mark_folder_read(
    app: tauri::AppHandle,
    store: State<'_, Store>,
    imap: State<'_, Imap>,
    account: Option<String>,
    folder: String,
    query: Option<String>,
) -> Result<usize, String> {
    let query = query.unwrap_or_default().trim().to_lowercase();
    let targets = store.unread_in(Scope::parse(account.as_deref()), &folder, &query).ok_or("unknown folder")?;
    let mut groups: std::collections::HashMap<(Option<String>, Folder), Vec<(u32, Option<u32>)>> = Default::default();
    for (id, account, folder, uid) in targets {
        groups.entry((account, folder)).or_default().push((id, uid));
    }
    let mut marked = 0;
    let mut error = None;
    for ((account, folder), items) in groups {
        if let Some(account) = &account {
            let uids: Vec<u32> = items.iter().filter_map(|(_, uid)| *uid).collect();
            if let Err(e) = imap.mark_seen_many(account, folder, &uids).await {
                error = Some(e.to_string());
                continue;
            }
        }
        for (id, _) in &items {
            store.set_unread(*id, false);
        }
        marked += items.len();
    }
    let _ = tauri::Emitter::emit(&app, "mailbox-changed", ());
    match error {
        Some(e) => Err(e),
        _ => Ok(marked),
    }
}

/// Прочитано/непрочитано и «важное» (флаг `\\Flagged`, письмо остаётся в своей папке).
/// Локально меняем только после подтверждения сервера.
#[tauri::command]
pub async fn set_message_flags(
    app: tauri::AppHandle,
    store: State<'_, Store>,
    imap: State<'_, Imap>,
    id: u32,
    unread: Option<bool>,
    starred: Option<bool>,
) -> Result<(), String> {
    let message = store.details(id).ok_or("письмо пропало")?;
    if let (Some(account), Some(uid)) = (message.account.as_deref(), message.uid) {
        if let Some(unread) = unread {
            imap.set_flag(account, message.folder, &[uid], "\\Seen", !unread).await.map_err(|e| e.to_string())?;
        }
        if let Some(starred) = starred {
            imap.set_flag(account, message.folder, &[uid], "\\Flagged", starred).await.map_err(|e| e.to_string())?;
        }
    }
    if let Some(unread) = unread {
        store.set_unread(id, unread);
    }
    if let Some(starred) = starred {
        store.set_starred(id, starred);
    }
    let _ = tauri::Emitter::emit(&app, "mailbox-changed", ());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_content_loads_by_itself_only_for_verified_trusted_senders() {
        let trusted: std::collections::BTreeSet<String> = ["news@github.com".to_string(), "x@пример.рф".to_string()].into();
        let policy = |from: &str, auth: Auth, show: bool| remote_policy(from, auth, &trusted, show);

        assert!(policy("News@GitHub.com", Auth::Pass, false).remote_shown, "доверенный и подлинный — сразу");
        let spoofed = policy("news@github.com", Auth::Unknown, false);
        assert!(spoofed.trusted && !spoofed.remote_shown, "без dmarc=pass адрес мог быть подделан — только по кнопке");
        let failed = policy("news@github.com", Auth::Fail, false);
        assert!(!failed.can_trust && !failed.trusted && !failed.remote_shown, "провал проверки отменяет доверие");
        assert!(!policy("x@пример.рф", Auth::Pass, false).can_trust, "домен с омоглифами не доверяем");
        assert!(!policy("other@github.com", Auth::Pass, false).remote_shown, "чужой адрес — по кнопке");
        assert!(policy("other@github.com", Auth::Unknown, true).remote_shown, "кнопка «Показать» работает всегда");
    }
}
