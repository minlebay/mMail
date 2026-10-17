//! Родное контекстное меню: GtkMenu (тема Breeze) с размытием KWin под ним вместо HTML-меню.
//!
//! Правый клик сначала получает страница: она решает, какие нужны пункты, и присылает их
//! командой `context_menu_items` (или пустой список). Затем WebKit сообщает ядру о самом клике —
//! сигнал `context-menu` с событием мыши, нужным Wayland для всплывающего окна. Что пришло
//! вторым, то и показывает меню. Если страница пунктов не дала (поле ввода, выделенный текст,
//! письмо во фрейме) — только безопасные действия правки WebKit: без навигации и без обхода
//! подтверждения ссылок.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use gtk::gio::prelude::*;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, WebviewWindow};
use webkit2gtk::{ContextMenuAction, ContextMenuExt, ContextMenuItemExt, WebViewExt};

/// Сколько ждать пунктов от страницы после клика (письмо во фрейме их не пришлёт).
const PAGE_WAIT: Duration = Duration::from_millis(300);
/// Пункты и клик старше этого друг к другу не относятся.
const PAIR_WINDOW: Duration = Duration::from_secs(1);
const CORNER_RADIUS: i32 = 12;

#[derive(Clone, Deserialize)]
pub struct PageMenu {
    token: u64,
    items: Vec<PageItem>,
    colors: Colors,
}

#[derive(Clone, Deserialize)]
pub struct PageItem {
    /// Номер пункта в странице; у разделителя нет.
    id: Option<u32>,
    #[serde(default)]
    label: String,
    #[serde(default)]
    danger: bool,
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    separator: bool,
}

/// Цвета темы приложения (CSS-цвета из переменных страницы).
#[derive(Clone, Deserialize)]
pub struct Colors {
    surface: String,
    text: String,
    accent: String,
    danger: String,
    border: String,
}

struct Click {
    event: gdk::Event,
    /// Разрешённые действия правки WebKit для этого места.
    edit: Vec<(String, gio::Action)>,
    /// Есть выделенный текст (WebKit предлагает «Копировать») — можно перевести.
    has_selection: bool,
    view: webkit2gtk::WebView,
    at: Instant,
}

#[derive(Default)]
struct Pending {
    page: Option<(PageMenu, Instant)>,
    click: Option<Click>,
}

thread_local! {
    // GTK живёт в главном потоке: и сигнал WebKit, и показ меню — там же.
    static PENDING: RefCell<HashMap<String, Pending>> = RefCell::new(HashMap::new());
    static CSS: gtk::CssProvider = gtk::CssProvider::new();
}

fn label(action: ContextMenuAction) -> Option<&'static str> {
    use ContextMenuAction::*;
    Some(match action {
        Copy => "Копировать",
        Cut => "Вырезать",
        Paste => "Вставить",
        PasteAsPlainText => "Вставить без форматирования",
        Delete => "Удалить",
        SelectAll => "Выделить всё",
        CopyLinkToClipboard => "Копировать адрес ссылки",
        CopyImageToClipboard => "Копировать изображение",
        CopyImageUrlToClipboard => "Копировать адрес изображения",
        _ => return None,
    })
}

pub fn install(view: &webkit2gtk::WebView, app: AppHandle, window_label: String) {
    view.connect_context_menu(move |view, menu, event, _| {
        let items = menu.items();
        let has_selection = items.iter().any(|item| item.stock_action() == ContextMenuAction::Copy);
        let edit: Vec<(String, gio::Action)> = items
            .into_iter()
            .filter_map(|item| Some((crate::i18n::text(&app, label(item.stock_action())?), item.gaction()?)))
            .collect();
        let click = Click { event: event.clone(), edit, has_selection, view: view.clone(), at: Instant::now() };
        PENDING.with(|p| p.borrow_mut().entry(window_label.clone()).or_default().click = Some(click));
        let (app, label) = (app.clone(), window_label.clone());
        if !try_show(&app, &label) {
            // Письмо во фрейме и прочее, где страница пунктов не пришлёт.
            glib::timeout_add_local_once(PAGE_WAIT, move || show_edit_only(&app, &label));
        }
        // Своё меню WebKit не показываем никогда.
        true
    });
}

/// Пункты от страницы для ближайшего правого клика.
#[tauri::command]
pub fn context_menu_items(app: AppHandle, window: WebviewWindow, menu: PageMenu) -> Result<(), String> {
    let c = &menu.colors;
    if ![&c.surface, &c.text, &c.accent, &c.danger, &c.border].into_iter().all(|v| safe_color(v)) {
        return Err("недопустимый цвет".into());
    }
    if menu.items.len() > 64 || menu.items.iter().any(|i| i.label.len() > 200) {
        return Err("слишком большое меню".into());
    }
    let label = window.label().to_string();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        PENDING.with(|p| p.borrow_mut().entry(label.clone()).or_default().page = Some((menu, Instant::now())));
        try_show(&handle, &label);
    })
    .map_err(|e| e.to_string())
}

/// Клик и пункты страницы оба на месте — показываем. Возвращает, показано ли меню.
fn try_show(app: &AppHandle, window_label: &str) -> bool {
    let ready = PENDING.with(|p| {
        let mut p = p.borrow_mut();
        let pending = p.get_mut(window_label)?;
        let (page_at, click_at) = (pending.page.as_ref()?.1, pending.click.as_ref()?.at);
        let gap = if page_at > click_at { page_at - click_at } else { click_at - page_at };
        if gap > PAIR_WINDOW {
            // Устаревшая половина — выбрасываем более старую.
            if page_at < click_at { pending.page = None } else { pending.click = None }
            return None;
        }
        Some((pending.page.take()?.0, pending.click.take()?))
    });
    let Some((page, click)) = ready else { return false };
    if page.items.is_empty() {
        popup(app, window_label, click, None);
    } else {
        popup(app, window_label, click, Some(page));
    }
    true
}

fn show_edit_only(app: &AppHandle, window_label: &str) {
    let click = PENDING.with(|p| p.borrow_mut().get_mut(window_label).and_then(|pending| pending.click.take()));
    if let Some(click) = click {
        popup(app, window_label, click, None);
    }
}

fn popup(app: &AppHandle, window_label: &str, click: Click, page: Option<PageMenu>) {
    if page.is_none() && click.edit.is_empty() {
        return;
    }
    let menu = gtk::Menu::new();
    menu.style_context().add_class("mmail-menu");
    match &page {
        Some(page) => {
            for item in &page.items {
                if item.separator {
                    menu.append(&gtk::SeparatorMenuItem::new());
                    continue;
                }
                let Some(id) = item.id else { continue };
                let entry = gtk::MenuItem::with_label(&item.label);
                entry.set_sensitive(!item.disabled);
                if item.danger {
                    entry.style_context().add_class("danger");
                }
                let (app, label, token) = (app.clone(), window_label.to_string(), page.token);
                entry.connect_activate(move |_| {
                    let _ = app.emit_to(label.as_str(), "context-menu-pick", serde_json::json!({ "token": token, "id": id }));
                });
                menu.append(&entry);
            }
        }
        None => {
            for (text, action) in &click.edit {
                let entry = gtk::MenuItem::with_label(text);
                entry.set_sensitive(action.is_enabled());
                let action = action.clone();
                entry.connect_activate(move |_| action.activate(None));
                menu.append(&entry);
            }
            // Окно перевода выделенного есть только в главном окне (письма читают там).
            if click.has_selection && window_label == "main" {
                menu.append(&gtk::SeparatorMenuItem::new());
                let entry = gtk::MenuItem::with_label(&crate::i18n::text(app, "Перевести выделенное"));
                let (app, label) = (app.clone(), window_label.to_string());
                entry.connect_activate(move |_| crate::translation::selected_text(app.clone(), label.clone()));
                menu.append(&entry);
            }
        }
    }
    style(page.as_ref().map(|p| &p.colors));

    // Полупрозрачное всплывающее окно (RGBA) — иначе вместо стекла будет чёрный фон.
    if let Some(top) = menu.toplevel().and_then(|t| t.downcast::<gtk::Window>().ok()) {
        top.style_context().add_class("mmail-menu-window");
        if let Some(visual) = WidgetExt::screen(&top).and_then(|s| s.rgba_visual()) {
            top.set_visual(Some(&visual));
        }
        top.set_app_paintable(true);
        crate::blur::enable_popup(&top, CORNER_RADIUS);
    }
    menu.set_attach_widget(Some(&click.view));
    // Меню одноразовое: уничтожаем после закрытия, когда выбранный пункт уже сработал.
    menu.connect_deactivate(|menu| {
        let menu = menu.clone();
        glib::idle_add_local_once(move || unsafe { menu.destroy() });
    });
    menu.show_all();
    menu.popup_at_pointer(Some(&click.event));
}

/// Оформление меню под тему приложения: стекло (полупрозрачный фон над размытием KWin).
fn style(colors: Option<&Colors>) {
    let (surface, text, accent, danger, border) = match colors {
        Some(c) => (c.surface.as_str(), c.text.as_str(), c.accent.as_str(), c.danger.as_str(), c.border.as_str()),
        None => ("rgba(36, 31, 48, 0.62)", "#f1eef8", "rgba(139, 123, 255, 0.28)", "#ff6b8b", "rgba(255, 255, 255, 0.14)"),
    };
    let css = format!(
        "window.mmail-menu-window, window.mmail-menu-window decoration {{
            background: transparent; box-shadow: none; border: none; border-radius: {r}px; margin: 0;
         }}
         menu.mmail-menu, .mmail-menu {{
            background-color: {surface}; color: {text}; border: 1px solid {border};
            border-radius: {r}px; padding: 6px; margin: 0; box-shadow: none;
         }}
         .mmail-menu menuitem {{ padding: 7px 14px; border-radius: 7px; min-height: 0; color: {text}; }}
         .mmail-menu menuitem label {{ color: {text}; }}
         .mmail-menu menuitem:hover {{ background-color: {accent}; }}
         .mmail-menu menuitem.danger label {{ color: {danger}; }}
         .mmail-menu menuitem:disabled label {{ opacity: 0.45; }}
         .mmail-menu separator {{ margin: 5px 6px; min-height: 1px; background-color: {border}; }}",
        r = CORNER_RADIUS
    );
    CSS.with(|provider| {
        if let Err(e) = provider.load_from_data(css.as_bytes()) {
            eprintln!("mMail: стиль меню: {e}");
            return;
        }
        if let Some(screen) = gdk::Screen::default() {
            gtk::StyleContext::add_provider_for_screen(&screen, provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
    });
}

/// Цвет из CSS-переменной страницы; в CSS GTK попадает только такой.
fn safe_color(value: &str) -> bool {
    value.len() <= 64
        && !value.is_empty()
        && value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '#' | '(' | ')' | ',' | '.' | ' ' | '%'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_editing_and_copy_actions_are_allowed() {
        assert!(label(ContextMenuAction::Copy).is_some());
        assert!(label(ContextMenuAction::Paste).is_some());
        assert!(label(ContextMenuAction::CopyLinkToClipboard).is_some());
        for action in [ContextMenuAction::OpenLink, ContextMenuAction::OpenLinkInNewWindow,
            ContextMenuAction::DownloadLinkToDisk, ContextMenuAction::OpenFrameInNewWindow,
            ContextMenuAction::Reload, ContextMenuAction::GoBack, ContextMenuAction::InspectElement] {
            assert!(label(action).is_none());
        }
    }

    #[test]
    fn page_colors_cannot_inject_css() {
        assert!(safe_color("rgba(36, 31, 48, 0.62)"));
        assert!(safe_color("#ff6b8b"));
        assert!(!safe_color("red; } * { background: url(http://x) "));
        assert!(!safe_color("red}"));
        assert!(!safe_color(""));
    }
}
