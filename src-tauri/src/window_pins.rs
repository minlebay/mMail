//! «Поверх других окон» и «На всех рабочих столах» — кнопки слева в шапке окна.
//!
//! На Wayland GTK не умеет ни того, ни другого (xdg-shell не даёт клиенту такого права),
//! поэтому просим KWin: короткий скрипт через D-Bus (org.kde.KWin /Scripting) находит наше
//! окно по pid и заголовку и ставит keepAbove / onAllDesktops. Без KWin (X11, другие среды) —
//! обычные вызовы Tauri.

use std::collections::HashMap;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use gtk::gio;
use gtk::glib::prelude::*;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State, WebviewWindow};

use crate::config::ConfigStore;

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Pins {
    pub above: bool,
    pub all_desktops: bool,
}

/// Состояние открытых окон (по label); главное окно и настройки ещё и запоминаются в конфиге.
#[derive(Default)]
pub struct PinState(Mutex<HashMap<String, Pins>>);

/// Окна писем каждый раз новые — их состояние не запоминаем.
fn remembered(label: &str) -> bool {
    matches!(label, "main" | "settings")
}

#[tauri::command]
pub fn window_pins(window: WebviewWindow, state: State<'_, PinState>) -> Pins {
    state.0.lock().unwrap().get(window.label()).copied().unwrap_or_default()
}

/// `pin`: "above" | "allDesktops".
#[tauri::command]
pub async fn set_window_pin(window: WebviewWindow, state: State<'_, PinState>, config: State<'_, ConfigStore>, pin: String, on: bool) -> Result<Pins, String> {
    let label = window.label().to_string();
    let mut pins = state.0.lock().unwrap().get(&label).copied().unwrap_or_default();
    match pin.as_str() {
        "above" => pins.above = on,
        "allDesktops" => pins.all_desktops = on,
        _ => return Err("неизвестный режим окна".into()),
    }
    apply(&window, pins).await?;
    state.0.lock().unwrap().insert(label.clone(), pins);
    if remembered(&label) {
        config.update(|c| {
            c.window_pins.insert(label, pins);
        })?;
    }
    Ok(pins)
}

/// После создания окна: вернуть запомненное. KWin видит окно не сразу — ждём, пока оно появится.
pub fn restore(app: &AppHandle, window: &WebviewWindow) {
    let label = window.label().to_string();
    let Some(pins) = remembered(&label).then(|| app.state::<ConfigStore>().get().window_pins.get(&label).copied()).flatten() else { return };
    if pins == Pins::default() {
        return;
    }
    app.state::<PinState>().0.lock().unwrap().insert(label, pins);
    let window = window.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(800)).await;
        if let Err(e) = apply(&window, pins).await {
            eprintln!("mMail: поверх окон / на всех рабочих столах: {e}");
        }
    });
}

async fn apply(window: &WebviewWindow, pins: Pins) -> Result<(), String> {
    let caption = window.title().map_err(|e| e.to_string())?;
    let kwin = tauri::async_runtime::spawn_blocking(move || kwin(&caption, pins)).await.map_err(|e| e.to_string())?;
    if let Err(e) = kwin {
        eprintln!("mMail: KWin недоступен ({e}), пробую средствами окна");
        window.set_always_on_top(pins.above).map_err(|e| e.to_string())?;
        window.set_visible_on_all_workspaces(pins.all_desktops).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Текст скрипта KWin. Заголовок — чужой для JS текст (тема письма), поэтому только через JSON.
fn script(pid: u32, caption: &str, pins: Pins) -> String {
    let caption = serde_json::to_string(caption).unwrap_or_else(|_| "\"\"".into());
    format!(
        "const c = {caption};\n\
         for (const w of workspace.windowList()) {{\n\
           if (w.pid === {pid} && (w.caption === c || w.caption.startsWith(c + \" <\"))) {{\n\
             w.keepAbove = {above};\n\
             w.onAllDesktops = {all};\n\
           }}\n\
         }}\n",
        above = pins.above,
        all = pins.all_desktops,
    )
}

fn kwin(caption: &str, pins: Pins) -> Result<(), String> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let pid = std::process::id();
    let name = format!("mmail-pins-{pid}-{}", NEXT.fetch_add(1, Ordering::Relaxed));
    // Файл скрипта — в личном каталоге сеанса (XDG_RUNTIME_DIR, 0700), сам файл 0600.
    let dir = std::env::var_os("XDG_RUNTIME_DIR").ok_or("нет XDG_RUNTIME_DIR")?;
    let path = std::path::Path::new(&dir).join(format!("{name}.js"));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).map_err(|e| e.to_string())?;
    file.write_all(script(pid, caption, pins).as_bytes()).map_err(|e| e.to_string())?;
    drop(file);

    let result = (|| {
        let bus = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>).map_err(|e| e.to_string())?;
        let call = |object: &str, interface: &str, method: &str, args: Option<&gtk::glib::Variant>| {
            bus.call_sync(Some("org.kde.KWin"), object, interface, method, args, None, gio::DBusCallFlags::NONE, 2000, None::<&gio::Cancellable>)
                .map_err(|e| e.to_string())
        };
        let loaded = call("/Scripting", "org.kde.kwin.Scripting", "loadScript", Some(&(path.to_string_lossy().as_ref(), name.as_str()).to_variant()))?;
        let id = loaded.child_value(0).get::<i32>().ok_or("KWin не загрузил скрипт")?;
        if id < 0 {
            return Err("KWin не загрузил скрипт".into());
        }
        let run = call(&format!("/Scripting/Script{id}"), "org.kde.kwin.Script", "run", None).map(|_| ());
        let _ = call("/Scripting", "org.kde.kwin.Scripting", "unloadScript", Some(&(name.as_str(),).to_variant()));
        run
    })();
    let _ = std::fs::remove_file(&path);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caption_cannot_break_out_of_script() {
        let s = script(42, "x\"; workspace.activeWindow.closeWindow(); \"", Pins { above: true, all_desktops: false });
        assert!(s.contains(r#"const c = "x\"; workspace.activeWindow.closeWindow(); \"";"#), "{s}");
        assert!(s.contains("w.pid === 42") && s.contains("w.keepAbove = true") && s.contains("w.onAllDesktops = false"));
    }
}
