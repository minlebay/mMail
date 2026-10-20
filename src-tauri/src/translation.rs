//! Перевод письма или выделенного текста через Google Translate — только по явному действию.
//! Текст уходит в Google; результат показывается в mMail, а не во вкладке браузера.
//! Длинный текст режется на части по абзацам (у сервиса ограничение на размер запроса).

use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::config::ConfigStore;
use crate::imap::Imap;
use crate::store::Store;

/// Символов в одном запросе к переводчику.
const CHUNK: usize = 4500;
/// Больше этого не переводим целиком — письмо-простыня или вложенный архив переписки.
const MAX_TEXT: usize = 100_000;

/// Языки, на которые можно переводить (коды Google). Код уходит в адрес запроса — только из списка.
/// Названия для списка в плашке — в src/lib/translate.js.
pub const LANGUAGES: &[&str] = &[
    "ru", "en", "sr", "uk", "de", "fr", "es", "it", "pt", "pl", "nl", "cs", "tr", "zh-CN", "ja", "ko",
];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Translation {
    text: String,
    /// Перевод письма для той же песочницы, что и письмо: экранированный текст,
    /// прогнанный через санитайзер без внешних ресурсов и обёрнутый со строгой CSP.
    srcdoc: Option<String>,
    /// Язык оригинала, как его определил переводчик (`en`, `sr`, …).
    source_language: Option<String>,
    /// На какой язык переведено.
    target_language: &'static str,
}

/// Перевод письма (тело скачивается с сервера, если письмо ещё не открывали).
#[tauri::command]
pub async fn translate_message(
    store: State<'_, Store>,
    imap: State<'_, Imap>,
    config: State<'_, ConfigStore>,
    id: u32,
    language: Option<String>,
) -> Result<Translation, String> {
    // Язык выбрали в плашке перевода — запоминаем: на него же пойдут следующие письма и выделенное.
    if let Some(language) = language {
        let code = known(&language).ok_or("неизвестный язык перевода")?;
        config.update(|c| c.translate_to = Some(code.to_string()))?;
    }
    let mut message = store.details(id).ok_or("письмо пропало")?;
    if message.html.is_none() {
        if let (Some(account), Some(uid)) = (message.account.clone(), message.uid) {
            let opened = imap.open(&account, message.folder, uid).await.map_err(|e| e.to_string())?;
            store.set_opened(id, opened);
            message = store.details(id).ok_or("письмо пропало")?;
        }
    }
    let html = message.html.unwrap_or_default();
    let clean = crate::sanitize::clean_html(&html, false);
    let text = mail_parser::decoders::html::html_to_text(&clean.html);
    let text = format!("{}\n\n{}", message.subject, text.trim());
    let mut translation = translate(text, target(&config)).await?;
    translation.srcdoc = Some(safe_document(&translation.text));
    Ok(translation)
}

/// Ответ переводчика — такое же внешнее содержимое, как письмо: никакой разметки из него
/// не берём (всё экранируется), а результат проходит тот же санитайзер и ту же CSP.
fn safe_document(text: &str) -> String {
    let escape = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;");
    let body: String = text
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| format!("<p>{}</p>", escape(p).replace('\n', "<br>")))
        .collect();
    crate::sanitize::wrap(&crate::sanitize::clean_html(&body, false).html)
}

/// Перевод выделенного фрагмента.
#[tauri::command]
pub async fn translate_text(config: State<'_, ConfigStore>, text: String) -> Result<Translation, String> {
    translate(text, target(&config)).await
}

fn known(code: &str) -> Option<&'static str> {
    LANGUAGES.iter().copied().find(|l| *l == code)
}

/// Выбранный в плашке язык, иначе — язык интерфейса.
fn target(config: &ConfigStore) -> &'static str {
    let config = config.get();
    config.translate_to.as_deref().and_then(known).unwrap_or(match config.language() {
        "en" => "en",
        "sr" => "sr",
        _ => "ru",
    })
}

async fn translate(text: String, target: &'static str) -> Result<Translation, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("Нет текста для перевода".into());
    }
    if text.chars().count() > MAX_TEXT {
        return Err("Письмо слишком длинное для перевода".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let mut out = String::new();
        let mut source = None;
        for chunk in chunks(&text, CHUNK) {
            let (translated, lang) = google(&chunk, target)?;
            out.push_str(&translated);
            source = source.or(lang);
        }
        Ok(Translation { text: out, srcdoc: None, source_language: source, target_language: target })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Один запрос к переводчику. Адрес фиксированный, текст — в теле POST, а не в URL.
fn google(text: &str, target: &str) -> Result<(String, Option<String>), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .max_redirects(0)
        .user_agent("Mozilla/5.0")
        .build()
        .into();
    // Тот же адрес, что у расширения Chrome: вход translate.googleapis.com (client=gtx)
    // отвечает страницей «Sorry…» (защита от ботов) на некоторых сетях.
    let url = format!("https://clients5.google.com/translate_a/t?client=dict-chrome-ex&sl=auto&tl={target}");
    let mut response = agent
        .post(&url)
        .send_form([("q", text)])
        .map_err(|e| format!("переводчик недоступен: {e}"))?;
    let body = response.body_mut().read_to_string().map_err(|e| e.to_string())?;
    parse(&body).ok_or_else(|| "переводчик вернул непонятный ответ".to_string())
}

/// Ответ вида `[["перевод","en"]]` (одна строка на каждый параметр `q`).
fn parse(body: &str) -> Option<(String, Option<String>)> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let first = value.get(0)?;
    match first {
        serde_json::Value::Array(pair) => {
            let text = pair.first()?.as_str()?.to_string();
            Some((text, pair.get(1).and_then(|v| v.as_str()).map(str::to_string)))
        }
        serde_json::Value::String(text) => Some((text.clone(), None)),
        _ => None,
    }
}

/// Части не длиннее `limit` символов: по абзацам, длинный абзац — по предложениям, а совсем
/// без знаков препинания — просто по длине. Переводы строк сохраняются.
fn chunks(text: &str, limit: usize) -> Vec<String> {
    let mut pieces: Vec<String> = Vec::new();
    for paragraph in text.split_inclusive('\n') {
        if paragraph.chars().count() <= limit {
            pieces.push(paragraph.to_string());
            continue;
        }
        let mut sentence = String::new();
        for c in paragraph.chars() {
            sentence.push(c);
            let end = matches!(c, '.' | '!' | '?' | '…') || sentence.chars().count() >= limit;
            if end {
                pieces.push(std::mem::take(&mut sentence));
            }
        }
        if !sentence.is_empty() {
            pieces.push(sentence);
        }
    }
    let mut out: Vec<String> = Vec::new();
    for piece in pieces {
        match out.last_mut() {
            Some(last) if last.chars().count() + piece.chars().count() <= limit => last.push_str(&piece),
            _ => out.push(piece),
        }
    }
    out
}

/// «Перевести выделенное»: берём выделение (PRIMARY — не трогает буфер обмена Ctrl+C)
/// и отдаём окну, где оно сделано. Вызывать в главном потоке.
pub fn selected_text(app: AppHandle, window_label: String) {
    let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_PRIMARY);
    clipboard.request_text(move |_, text| {
        let text = text.map(|s| s.to_string()).filter(|s| !s.trim().is_empty());
        let target = app.get_webview_window(&window_label).map(|w| w.label().to_string()).unwrap_or_else(|| "main".into());
        match text {
            Some(text) => {
                let _ = app.emit_to(target.as_str(), "translate-selection", text);
            }
            None => {
                let _ = app.emit_to(target.as_str(), "operation-error", "Не удалось получить выделение. Выделите текст заново.");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_text_is_split_without_losing_anything() {
        let text = format!("{}\n{}\n{}", "Короткий абзац.", "Предложение. ".repeat(800), "x".repeat(12_000));
        let parts = chunks(&text, CHUNK);
        assert!(parts.len() > 3);
        assert!(parts.iter().all(|p| p.chars().count() <= CHUNK), "каждая часть в пределах лимита");
        assert_eq!(parts.concat(), text, "склеенные части — исходный текст");
    }

    #[test]
    fn short_paragraphs_are_merged() {
        assert_eq!(chunks("а\nб\nв", CHUNK), vec!["а\nб\nв".to_string()]);
    }

    /// Живой запрос к переводчику: `cargo test -- --ignored live_google`.
    #[test]
    #[ignore]
    fn live_google() {
        let (text, lang) = google("Hello. How are you?\nSecond line.", "ru").unwrap();
        assert!(text.contains('\n') && lang.as_deref() == Some("en"), "{text:?} {lang:?}");
    }

    #[test]
    fn target_languages_are_safe_for_the_url() {
        // Код подставляется в адрес запроса — только буквы и дефис.
        assert!(LANGUAGES.iter().all(|l| l.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-')));
        assert_eq!(known("de"), Some("de"));
        assert_eq!(known("de&tl=x"), None);
    }

    #[test]
    fn translated_markup_is_inert() {
        let doc = safe_document("Заголовок\n\n<img src=\"https://tracker.example/p.gif\"> <script>alert(1)</script>\nстрока");
        assert!(!doc.contains("<img") && !doc.contains("<script"), "разметка из перевода не исполняется: {doc}");
        assert!(doc.contains("&lt;script&gt;"), "а показывается как текст");
        assert!(doc.contains("Content-Security-Policy"), "та же CSP, что у письма");
    }

    #[test]
    fn parses_google_response() {
        let body = r#"[["Привет. Как вы?\nВторая строка.","en"]]"#;
        assert_eq!(parse(body), Some(("Привет. Как вы?\nВторая строка.".to_string(), Some("en".to_string()))));
        assert_eq!(parse(r#"["Привет"]"#), Some(("Привет".to_string(), None)));
        assert_eq!(parse("<html>captcha</html>"), None);
    }
}
