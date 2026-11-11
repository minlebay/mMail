//! Тело письма — в отдельном WebKit поверх окна, как `<browser id="messagepane">` у Thunderbird.
//!
//! Зачем не iframe: масштаб. CSS zoom на iframe WebKitGTK применяет только при загрузке документа,
//! transform мылит, а скрипт масштаба внутри письма ослаблял изоляцию и всё равно тормозил на
//! каждом щелчке (IPC → страница → postMessage → раскладка). Здесь масштаб — родной `zoom_level`
//! движка прямо из обработчика колёсика в ядре.
//!
//! Изоляция строже прежней: свой эфемерный контекст (ни cookies, ни кеша, ни общего с окном
//! приложения), JavaScript выключен полностью, CSP документа та же, а любая навигация
//! запрещена — ссылка уходит в подтверждение, как и раньше.
//!
//! Где показывать письмо, решает страница: она присылает прямоугольник места под письмо
//! (`mail_view_bounds`) и прячет вид, пока поверх открыт диалог (нативный вид всегда сверху).

use std::cell::RefCell;
use std::sync::Mutex;

use gtk::prelude::*;
use gtk::{gdk, glib};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use url::Url;
use webkit2gtk::{
    HitTestResultExt, NavigationPolicyDecision, NavigationPolicyDecisionExt, NavigationType, PolicyDecisionExt,
    PolicyDecisionType, SettingsExt, URIRequestExt, UserContentInjectedFrames, UserContentManager, UserContentManagerExt,
    UserStyleLevel, UserStyleSheet, WebContext, WebView, WebViewExt,
};

pub const ZOOM_MIN: f64 = 0.5;
pub const ZOOM_MAX: f64 = 3.0;

/// Адрес документа письма: `load_html` без базового адреса — `about:blank`.
const BLANK: &str = "about:blank";

struct Mail {
    view: WebView,
    overlay: gtk::Overlay,
    /// Пользовательская таблица стилей: `--mmail-z` — масштаб письма. Свечение, отступы и полоса
    /// прокрутки делятся на него и на экране не растут с масштабом. Меняется без перезагрузки.
    styles: UserContentManager,
    /// Масштаб интерфейса: страница присылает CSS-пиксели, виджету нужны логические.
    app_zoom: f64,
    /// Масштаб самого письма (Ctrl + колёсико), поверх масштаба интерфейса.
    mail_zoom: f64,
    /// Место под письмо в CSS-пикселях страницы: x, y, ширина, высота.
    bounds: (f64, f64, f64, f64),
    shown: bool,
}

thread_local! {
    // GTK-объекты живут только в главном потоке; команды попадают сюда через run_on_main_thread.
    static MAIL: RefCell<Option<Mail>> = const { RefCell::new(None) };
}

#[derive(Clone, Serialize)]
struct MailZoom {
    zoom: f64,
}

/// Адрес ссылки под курсором и номер изменения. Событие `mail-link-hover` несёт только
/// сигнал; сам адрес главное окно забирает командой, чтобы он не попадал в другие окна.
#[derive(Default)]
pub struct MailHover {
    state: Mutex<(u64, Option<String>)>,
}

#[derive(Clone, Serialize)]
pub struct MailHoverState {
    revision: u64,
    /// `None` — курсор не над ссылкой (в том числе ушёл из письма).
    url: Option<String>,
}

impl MailHover {
    /// Запоминает адрес; `None` — адрес не изменился, событие не нужно.
    fn set(&self, url: Option<String>) -> Option<u64> {
        let mut state = self.state.lock().unwrap();
        if state.1 == url {
            return None;
        }
        state.0 += 1;
        state.1 = url;
        Some(state.0)
    }

    fn get(&self) -> MailHoverState {
        let state = self.state.lock().unwrap();
        MailHoverState { revision: state.0, url: state.1.clone() }
    }
}

/// Запоминает адрес под курсором и будит главное окно. Данные — только в состоянии ядра.
fn set_hover(app: &AppHandle, url: Option<String>) {
    if app.state::<MailHover>().set(url).is_some() {
        let _ = app.emit_to("main", "mail-link-hover", ());
    }
}

/// Встраивает вид письма поверх основного webview главного окна. Вызывать в главном потоке.
pub fn install(main_view: &WebView, app: AppHandle, app_zoom: f64) {
    // Tauri кладёт webview окна в GtkBox; дочерний webview Tauri положил бы рядом, а не поверх,
    // поэтому основной вид переезжает в GtkOverlay, а письмо — слоем над ним.
    let Some(parent) = main_view.parent().and_then(|p| p.downcast::<gtk::Box>().ok()) else {
        eprintln!("mMail: webview окна не в GtkBox — вид письма не встроен");
        return;
    };
    let position = parent.child_property::<i32>(main_view, "position");
    let overlay = gtk::Overlay::new();
    parent.remove(main_view);
    overlay.add(main_view);
    parent.pack_start(&overlay, true, true, 0);
    parent.reorder_child(&overlay, position);

    let styles = UserContentManager::new();
    let view = WebView::builder().web_context(&WebContext::new_ephemeral()).user_content_manager(&styles).build();
    if let Some(settings) = WebViewExt::settings(&view) {
        settings.set_enable_javascript(false);
        settings.set_enable_developer_extras(false);
        settings.set_enable_webgl(false);
        settings.set_enable_media(false);
    }
    view.set_background_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));
    // Растянут на весь слой и ограничен отступами со всех сторон: по запрошенному размеру
    // GtkOverlay выдал бы «естественный» размер WebKit, а он бывает больше — письмо наезжало на кнопки.
    view.set_halign(gtk::Align::Fill);
    view.set_valign(gtk::Align::Fill);
    overlay.add_overlay(&view);
    overlay.show_all();
    view.hide();

    let links = app.clone();
    view.connect_decide_policy(move |_, decision, kind| decide(&links, decision, kind));
    install_link_hover(&view, app.clone());
    install_zoom_wheel(&view, app.clone());
    crate::context_menu::install(&view, app, "mail".into());

    let mail = Mail { view, overlay, styles, app_zoom, mail_zoom: 1.0, bounds: (0.0, 0.0, 0.0, 0.0), shown: false };
    set_zoom_var(&mail);
    MAIL.with(|m| *m.borrow_mut() = Some(mail));
}

/// Своя загрузка документа письма — да; всё остальное (клик по ссылке, новое окно, редирект) — нет.
/// Ссылку не открываем, а просим главное окно подтвердить, как раньше с iframe.
fn decide(app: &AppHandle, decision: &webkit2gtk::PolicyDecision, kind: PolicyDecisionType) -> bool {
    if !matches!(kind, PolicyDecisionType::NavigationAction | PolicyDecisionType::NewWindowAction) {
        return false;
    }
    let Some(navigation) = decision.downcast_ref::<NavigationPolicyDecision>() else {
        decision.ignore();
        return true;
    };
    let action = navigation.navigation_action();
    let uri = action.as_ref().and_then(|a| a.request()).and_then(|r| r.uri()).map(|u| u.to_string()).unwrap_or_default();
    let clicked = action.as_ref().is_some_and(|a| a.navigation_type() == NavigationType::LinkClicked);
    if kind == PolicyDecisionType::NavigationAction && !clicked && uri == BLANK {
        return false;
    }
    if let Ok(url) = Url::parse(&uri) {
        if crate::links::openable(&url) {
            app.state::<crate::links::PendingLink>().request(url);
            let _ = app.emit_to("main", "link-request", ());
        }
    }
    decision.ignore();
    true
}

/// Наведение на ссылку: адрес уходит странице, она показывает его в подвале письма.
/// `mouse-target-changed` срабатывает при смене цели, а `leave-notify-event` сбрасывает
/// строку, когда курсор уходит из вида письма (иначе последний адрес остался бы висеть).
fn install_link_hover(view: &WebView, app: AppHandle) {
    let target_app = app.clone();
    view.connect_mouse_target_changed(move |_, hit, _| {
        let url = hit
            .context_is_link()
            .then(|| hit.link_uri())
            .flatten()
            .and_then(|uri| link_to_show(uri.as_str()));
        set_hover(&target_app, url);
    });

    let leave_app = app;
    view.connect_leave_notify_event(move |_, _| {
        set_hover(&leave_app, None);
        glib::Propagation::Proceed
    });
}

/// Только http(s) и mailto. Санитайзер и так вырезает прочие схемы из `href`, но строка
/// состояния не должна показывать `file:` или `javascript:`, даже если такая ссылка просочится.
fn link_to_show(uri: &str) -> Option<String> {
    let url = Url::parse(uri).ok()?;
    crate::links::openable(&url).then(|| uri.to_string())
}

/// Ctrl + колёсико над письмом — родной масштаб WebKit, без IPC и раскладки страницы приложения.
/// Странице уходит только новое значение для плашки «130%».
fn install_zoom_wheel(view: &WebView, app: AppHandle) {
    view.connect_scroll_event(move |_, event| {
        if !event.state().contains(gdk::ModifierType::CONTROL_MASK) {
            return glib::Propagation::Proceed;
        }
        let dy = match event.direction() {
            gdk::ScrollDirection::Up => -1.0,
            gdk::ScrollDirection::Down => 1.0,
            gdk::ScrollDirection::Smooth => event.delta().1,
            _ => return glib::Propagation::Proceed,
        };
        // Щелчок колеса — ±10%; степень, чтобы рывок тачпада не увёл масштаб в ноль.
        let zoom = with_mail(|m| {
            let next = (m.mail_zoom * 1.1f64.powf(-dy)).clamp(ZOOM_MIN, ZOOM_MAX);
            m.mail_zoom = if (next - 1.0).abs() < 0.02 { 1.0 } else { next };
            m.view.set_zoom_level(m.app_zoom * m.mail_zoom);
            set_zoom_var(m);
            m.mail_zoom
        });
        if let Some(zoom) = zoom {
            let _ = app.emit_to("main", "mail-zoom", MailZoom { zoom });
        }
        glib::Propagation::Stop
    });
}

fn with_mail<R>(f: impl FnOnce(&mut Mail) -> R) -> Option<R> {
    MAIL.with(|m| m.borrow_mut().as_mut().map(f))
}

fn set_zoom_var(m: &Mail) {
    m.styles.remove_all_style_sheets();
    let css = format!(":root{{--mmail-z:{}}}", m.mail_zoom);
    m.styles.add_style_sheet(&UserStyleSheet::new(&css, UserContentInjectedFrames::TopFrame, UserStyleLevel::User, &[], &[]));
}

fn apply(m: &Mail) {
    let z = m.app_zoom;
    let (x, y, w, h) = m.bounds;
    let px = |v: f64| (v * z).round().max(0.0) as i32;
    let (width, height) = (m.overlay.allocated_width(), m.overlay.allocated_height());
    m.view.set_margin_start(px(x));
    m.view.set_margin_top(px(y));
    m.view.set_margin_end((width - px(x + w)).max(0));
    m.view.set_margin_bottom((height - px(y + h)).max(0));
    m.view.set_zoom_level(m.app_zoom * m.mail_zoom);
    m.view.set_visible(m.shown && w >= 1.0 && h >= 1.0);
}

fn on_main(app: &AppHandle, f: impl FnOnce(&mut Mail) + Send + 'static) -> Result<(), String> {
    app.run_on_main_thread(move || {
        with_mail(|m| {
            f(m);
            apply(m);
        });
    })
    .map_err(|e| e.to_string())
}

/// Масштаб интерфейса поменялся — письмо держит свой масштаб относительно него.
pub fn set_app_zoom(app: &AppHandle, zoom: f64) {
    let _ = on_main(app, move |m| m.app_zoom = zoom);
}

/// Адрес ссылки под курсором для главного окна. Событие `mail-link-hover` — только сигнал.
#[tauri::command]
pub fn mail_link_hover(hover: tauri::State<'_, MailHover>) -> MailHoverState {
    hover.get()
}

/// Документ письма (уже очищенный ядром; схему и мягкие края подставила страница).
#[tauri::command]
pub fn mail_view_load(app: AppHandle, html: String) -> Result<(), String> {
    let result = on_main(&app, move |m| m.view.load_html(&html, None));
    // Старый адрес не должен остаться в подвале, пока новый документ ещё грузится.
    set_hover(&app, None);
    result
}

/// Видимость отдельно от загрузки: диалог поверх прячет письмо, не сбрасывая его прокрутку.
#[tauri::command]
pub fn mail_view_visible(app: AppHandle, visible: bool) -> Result<(), String> {
    let result = on_main(&app, move |m| m.shown = visible);
    if !visible {
        set_hover(&app, None);
    }
    result
}

/// Место под письмо в CSS-пикселях страницы главного окна.
#[tauri::command]
pub fn mail_view_bounds(app: AppHandle, x: f64, y: f64, width: f64, height: f64) -> Result<(), String> {
    if ![x, y, width, height].iter().all(|v| v.is_finite()) {
        return Err("недопустимые размеры".into());
    }
    on_main(&app, move |m| m.bounds = (x, y, width, height))
}

/// Плашка «130%» — вернуть масштаб письма.
#[tauri::command]
pub fn mail_view_zoom(app: AppHandle, zoom: f64) -> Result<(), String> {
    if !zoom.is_finite() {
        return Err("недопустимый масштаб".into());
    }
    let zoom = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
    on_main(&app, move |m| {
        m.mail_zoom = zoom;
        set_zoom_var(m);
    })?;
    let _ = app.emit_to("main", "mail-zoom", MailZoom { zoom });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_state_reports_each_change_once() {
        let hover = MailHover::default();
        assert_eq!(hover.get().revision, 0);
        assert_eq!(hover.set(Some("https://a.example/".into())), Some(1));
        assert_eq!(hover.set(Some("https://a.example/".into())), None, "тот же адрес — без события");
        assert_eq!(hover.set(None), Some(2));
        assert_eq!(hover.get().url, None);
    }

    #[test]
    fn only_openable_links_reach_the_status_line() {
        assert_eq!(link_to_show("https://a.example/path?q=1").as_deref(), Some("https://a.example/path?q=1"));
        assert_eq!(link_to_show("http://a.example").as_deref(), Some("http://a.example"));
        assert_eq!(link_to_show("mailto:a@b.example").as_deref(), Some("mailto:a@b.example"));
        for bad in ["file:///etc/passwd", "javascript:alert(1)", "data:text/html,x", "not a url"] {
            assert!(link_to_show(bad).is_none(), "{bad}");
        }
    }
}
