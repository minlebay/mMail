use tauri::State;
use crate::config::ConfigStore;

fn google_url(text: &str, language: &str) -> Result<url::Url, String> {
    let text = text.trim();
    if text.is_empty() { return Err("Нет текста для перевода".into()); }
    if text.chars().count() > 5000 { return Err("Текст нужно разделить на части до 5000 символов".into()); }
    let language = match language { "en" => "en", "sr" => "sr", _ => "ru" };
    let mut url = url::Url::parse("https://translate.google.com/").unwrap();
    url.query_pairs_mut().append_pair("sl", "auto").append_pair("tl", language).append_pair("text", text).append_pair("op", "translate");
    Ok(url)
}

#[tauri::command]
pub fn translate_text(config: State<'_, ConfigStore>, text: String) -> Result<(), String> {
    let url = google_url(&text, config.get().language())?;
    std::process::Command::new("xdg-open").arg(url.as_str()).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn selected_text(app: tauri::AppHandle) {
    use tauri::Emitter;
    // PRIMARY holds the current highlighted text on GTK/Wayland. Reading it does not
    // overwrite the user's copy/paste clipboard. Called only by the explicit menu action.
    let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_PRIMARY);
    clipboard.request_text(move |_, text| {
        match text.filter(|s| !s.trim().is_empty()) {
            Some(text) => { let _ = app.emit_to("main", "translate-selection", text); }
            None => { let _ = app.emit_to("main", "operation-error", "Не удалось получить выделение. Выделите текст заново."); }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn translation_url_encodes_text_and_has_fixed_origin() {
        let url = google_url("Привет &tl=evil # /?\n世界", "sr").unwrap();
        assert_eq!(url.host_str(), Some("translate.google.com"));
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(pairs.get("tl").unwrap(), "sr");
        assert_eq!(pairs.get("text").unwrap(), "Привет &tl=evil # /?\n世界");
    }
    #[test]
    fn empty_and_oversized_text_are_rejected_without_truncation() {
        assert!(google_url("  ", "ru").is_err());
        assert!(google_url(&"я".repeat(5001), "ru").is_err());
        assert!(google_url(&"я".repeat(5000), "en").is_ok());
    }
}
