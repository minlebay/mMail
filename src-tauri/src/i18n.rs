//! Native UI uses the same catalog as the webview.
use std::sync::OnceLock;
use tauri::{AppHandle, Manager};

pub fn text(app: &AppHandle, key: &str) -> String {
    static MESSAGES: OnceLock<serde_json::Value> = OnceLock::new();
    let messages = MESSAGES.get_or_init(|| serde_json::from_str(include_str!("../../src/lib/translations.json")).expect("valid translation catalog"));
    let language = app.state::<crate::config::ConfigStore>().get().language();
    messages.get(key).and_then(|entry| entry.get(language)).and_then(|s| s.as_str()).unwrap_or(key).to_owned()
}
