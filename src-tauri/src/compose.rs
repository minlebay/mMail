//! Окно написания письма: открытие, начальные данные (ответ/пересылка с цитатой), вложения.

use std::collections::HashMap;
use std::path::PathBuf;
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
    from: Option<String>,
    to: String,
    subject: String,
    /// Исходное письмо для ответа или пересылки.
    source: Option<u32>,
    /// "new" | "reply" | "forward"
    kind: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeInit {
    from: String,
    accounts: Vec<String>,
    to: String,
    subject: String,
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
        subject: prefill.subject,
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

/// Системный диалог выбора файлов открывает ядро; в UI уходят только id, имя и размер.
#[tauri::command]
pub async fn pick_attachments(app: AppHandle, state: State<'_, ComposeState>) -> Result<Vec<AttachmentInfo>, String> {
    let Some(picked) = app.dialog().file().set_title(crate::i18n::text(&app, "Прикрепить файлы")).blocking_pick_files() else {
        return Ok(Vec::new());
    };
    let mut infos = Vec::new();
    for file in picked {
        let Ok(path) = file.into_path() else { continue };
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        if !meta.is_file() {
            continue;
        }
        let id = state.next_file.fetch_add(1, Ordering::Relaxed) + 1;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
        state.files.lock().unwrap().insert(id, path);
        infos.push(AttachmentInfo { id, name, size: meta.len() });
    }
    Ok(infos)
}
