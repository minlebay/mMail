//! Окно написания письма: открытие, начальные данные (ответ/пересылка с цитатой), вложения.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

use crate::imap::Imap;
use crate::store::{date_label, Store};
use crate::{accounts, sanitize};

/// Лимит вложений на письмо — как у Gmail.
pub const MAX_ATTACHMENTS_BYTES: u64 = 25 * 1024 * 1024;

#[derive(Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefill {
    /// Желаемый отправитель (ящик выбранной папки или ящик исходного письма).
    pub(crate) from: Option<String>,
    pub(crate) to: String,
    pub(crate) cc: String,
    pub(crate) bcc: String,
    pub(crate) subject: String,
    /// Готовый текст письма (простой текст) — для писем, открытых извне.
    pub(crate) body: String,
    /// Файлы для писем, открытых извне. Из UI их передать нельзя (`skip`) — только ядро знает пути.
    #[serde(skip)]
    pub(crate) attach: Vec<PathBuf>,
    /// Исходное письмо для ответа или пересылки.
    pub(crate) source: Option<u32>,
    /// "new" | "reply" | "forward"
    pub(crate) kind: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeInit {
    from: String,
    accounts: Vec<String>,
    to: String,
    cc: String,
    bcc: String,
    subject: String,
    body: String,
    attachments: Vec<AttachmentInfo>,
    kind: String,
    /// Строка над цитатой: «17.10.2026 13:28, Имя <адрес> пишет:» или шапка пересылки.
    quote_header: Option<String>,
    /// Очищенный HTML исходного письма, без внешних ресурсов.
    quote_html: Option<String>,
    in_reply_to: Option<String>,
    references: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct AttachmentInfo {
    id: u64,
    name: String,
    size: u64,
}

/// Начальные данные открытых окон и выбранные файлы. Путь к файлу знает только ядро:
/// UI получает id и имя, поэтому подсунуть в письмо произвольный файл с диска нельзя.
#[derive(Default)]
pub struct ComposeState {
    next_window: AtomicU32,
    next_file: AtomicU64,
    prefills: Mutex<HashMap<String, Prefill>>,
    files: Mutex<HashMap<u64, PathBuf>>,
}

impl ComposeState {
    /// Запомнить выбранный файл; в UI уходят только id, имя и размер.
    fn register(&self, path: &Path) -> Option<AttachmentInfo> {
        let meta = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
        let id = self.next_file.fetch_add(1, Ordering::Relaxed) + 1;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
        self.files.lock().unwrap().insert(id, path.to_path_buf());
        Some(AttachmentInfo { id, name, size: meta.len() })
    }

    pub fn file(&self, id: u64) -> Option<PathBuf> {
        self.files.lock().unwrap().get(&id).cloned()
    }
}

#[tauri::command]
pub fn open_compose(app: AppHandle, state: State<'_, ComposeState>, prefill: Prefill) -> Result<(), String> {
    let label = format!("compose-{}", state.next_window.fetch_add(1, Ordering::Relaxed));
    let title = if prefill.subject.is_empty() { crate::i18n::text(&app, "Новое письмо") } else { prefill.subject.clone() };
    state.prefills.lock().unwrap().insert(label.clone(), prefill);

    let builder = crate::glass_window(&app, &label, &title).inner_size(820.0, 680.0).min_inner_size(560.0, 420.0);
    // Все окна письма открываются размера последнего изменённого.
    let window = crate::window_state::restore(&app, "compose", builder, (560.0, 420.0))
        .on_new_window(|_, _| NewWindowResponse::Deny)
        .build()
        .map_err(|e| e.to_string())?;
    crate::window_state::track(&app, &window, "compose");
    crate::finish_window(&app, &window).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn compose_init(
    window: WebviewWindow,
    state: State<'_, ComposeState>,
    store: State<'_, Store>,
    imap: State<'_, Imap>,
) -> Result<ComposeInit, String> {
    let prefill = state.prefills.lock().unwrap().remove(window.label()).unwrap_or_default();
    let accounts = accounts::list().await.map_err(|e| e.to_string())?;
    let from = prefill
        .from
        .filter(|f| accounts.contains(f))
        .or_else(|| accounts.first().cloned())
        .unwrap_or_default();

    let mut init = ComposeInit {
        from,
        accounts,
        to: prefill.to,
        cc: prefill.cc,
        bcc: prefill.bcc,
        subject: prefill.subject,
        body: prefill.body,
        attachments: prefill.attach.iter().filter_map(|path| state.register(path)).collect(),
        kind: prefill.kind.clone(),
        quote_header: None,
        quote_html: None,
        in_reply_to: None,
        references: Vec::new(),
    };
    let Some(mut source) = prefill.source.and_then(|id| store.details(id)) else { return Ok(init) };

    // Исходное письмо могли ещё не открывать — загружаем тело (и Message-ID) с сервера.
    if source.html.is_none() {
        if let (Some(account), Some(uid)) = (source.account.clone(), source.uid) {
            let opened = imap.open(&account, source.folder, uid).await.map_err(|e| e.to_string())?;
            store.set_opened(source.id, opened);
            source = store.details(source.id).ok_or("письмо пропало")?;
        }
    }

    let when = date_label(source.timestamp);
    let sender = format!("{} <{}>", source.from_name, source.from_addr);
    init.quote_header = Some(if prefill.kind == "forward" {
        format!("---------- Пересылаемое сообщение ----------\nОт: {sender}\nДата: {when}\nТема: {}", source.subject)
    } else {
        format!("{when}, {sender} пишет:")
    });
    init.quote_html = source.html.as_deref().map(|html| sanitize::clean_html(html, false).html);
    if prefill.kind == "reply" {
        if let Some(id) = &source.message_id {
            init.in_reply_to = Some(id.clone());
            init.references = source.references.clone();
            init.references.push(id.clone());
        }
    }
    Ok(init)
}

/// HTML, набранный в режиме «Код»: прежде чем показать его в редакторе, чистим тем же
/// фильтром, что и входящие письма (без скриптов, обработчиков и внешних картинок).
#[tauri::command]
pub fn clean_compose_html(html: String) -> Result<String, String> {
    if html.len() > 1024 * 1024 {
        return Err("письмо слишком большое".into());
    }
    Ok(sanitize::clean_html(&html, false).html)
}

/// Системный диалог выбора файлов открывает ядро; в UI уходят только id, имя и размер.
#[tauri::command]
pub async fn pick_attachments(app: AppHandle, state: State<'_, ComposeState>) -> Result<Vec<AttachmentInfo>, String> {
    let Some(picked) = app.dialog().file().set_title(crate::i18n::text(&app, "Прикрепить файлы")).blocking_pick_files() else {
        return Ok(Vec::new());
    };
    let mut infos = Vec::new();
    for file in picked {
        let Ok(path) = file.into_path() else { continue };
        infos.extend(state.register(&path));
    }
    Ok(infos)
}

/// Открыть окно письма из фонового потока (уведомление, D-Bus): окна GTK и размытие KWin
/// создаются только в главном потоке.
pub fn open_from_background(app: &AppHandle, prefill: Prefill) {
    let handle = app.clone();
    let opened = app.run_on_main_thread(move || {
        if let Err(e) = open_compose(handle.clone(), tauri::Manager::state::<ComposeState>(&handle), prefill) {
            eprintln!("mMail: окно письма: {e}");
        }
    });
    if let Err(e) = opened {
        eprintln!("mMail: окно письма: {e}");
    }
}

/// «Ответить» из уведомления о письме: то же, что кнопка «Ответить» в просмотре.
pub fn open_reply(app: &AppHandle, id: u32) -> Result<(), String> {
    let store = tauri::Manager::state::<Store>(app);
    let m = store.details(id).ok_or("письмо пропало")?;
    let lower = m.subject.to_lowercase();
    let subject = if lower.starts_with("re:") || lower.starts_with("fw:") || lower.starts_with("fwd:") { m.subject.clone() } else { format!("Re: {}", m.subject) };
    let prefill = Prefill { from: m.account.clone(), to: m.from_addr.clone(), subject, source: Some(id), kind: "reply".into(), ..Default::default() };
    open_from_background(app, prefill);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_cannot_attach_files_by_path() {
        // Путь к файлу знает только ядро: из UI вложение не передать, даже если окно скомпрометировано.
        let prefill: Prefill = serde_json::from_str(r#"{"to":"a@b.example","attach":["/home/user/.ssh/id_ed25519"]}"#).unwrap();
        assert_eq!(prefill.to, "a@b.example");
        assert!(prefill.attach.is_empty());
        assert!(ComposeState::default().file(1).is_none(), "чужой id файла ничего не даёт");
    }
}
