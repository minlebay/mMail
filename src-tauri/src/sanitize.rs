//! Очистка HTML писем. Первый из двух слоёв защиты: второй — sandbox-iframe с CSP в UI.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use std::cell::{Cell, RefCell};

use ammonia::{Builder, UrlRelative, UrlRelativeEvaluate};
use lol_html::{element, rewrite_str, Settings};

/// Политика документа письма: никаких сетевых запросов и скриптов, только inline-стили и
/// data:-картинки. Сам вид письма (mailview.rs) и так без JavaScript — это второй слой.
const FRAME_CSP: &str =
    "default-src 'none'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'; form-action 'none'";

const BASE_CSS: &str = "\
html,body{margin:0;background:transparent;color:#e7e4f0;\
font:15px/1.6 'Inter','Noto Sans',system-ui,sans-serif;overflow-wrap:anywhere}\
body{padding:0 0 16px}\
a{color:#b3a8ff}\
img{max-width:100%;height:auto}\
table{max-width:100%}\
blockquote{margin:0 0 0 .6em;padding-left:.8em;border-left:2px solid rgba(255,255,255,.2);color:#bdb8cc}\
:root{--ink:255,255,255}\
html[data-scheme=light]{--ink:24,20,40}\
html[data-scheme=light],html[data-scheme=light] body{color:#1d1a26}\
html[data-scheme=light] a{color:#5a49d6}\
html[data-scheme=light] blockquote{border-left-color:rgba(24,20,40,.2);color:#5b5768}\
html:has([data-mmail-scroll]),html:has([data-mmail-scroll]) body{background:transparent!important;\
overflow:hidden!important;height:100%;padding:0}\
[data-mmail-scroll]{position:fixed;inset:0;overflow:auto;overflow-y:scroll}\
[data-mmail-canvas]{background:var(--canvas);padding:8px;box-sizing:border-box;width:fit-content;min-width:100%}\
html[data-soft] [data-mmail-frame]{padding:calc(20px / var(--z)) calc(28px / var(--z)) calc(24px / var(--z))}\
html[data-soft] [data-mmail-canvas]{box-shadow:0 0 calc(20px / var(--z)) calc(7px / var(--z)) var(--canvas)}\
html[data-soft] [data-mmail-scroll]{--fade:calc(16px / var(--z));\
mask-image:linear-gradient(#000,#000),\
linear-gradient(to right,transparent,#000 var(--fade),#000 calc(100% - var(--fade)),transparent),\
linear-gradient(transparent,#000 var(--fade),#000 calc(100% - var(--fade)),transparent);\
mask-size:var(--scrollbar-size) 100%,calc(100% - var(--scrollbar-size)) 100%,calc(100% - var(--scrollbar-size)) 100%;\
mask-position:right top,left top,left top;mask-repeat:no-repeat;mask-composite:add,intersect,add}";

const SCROLLBAR_CSS: &str = include_str!("../../src/scrollbars.css");

/// После SCROLLBAR_CSS: `--mmail-z` — масштаб письма из пользовательской таблицы вида письма
/// (mailview.rs). Полоса прокрутки и наши отступы делятся на него — на экране не растут с масштабом.
const ZOOM_CSS: &str = "html:root{--z:var(--mmail-z,1);--scrollbar-size:calc(10px / var(--z))}";

const GENERIC_ATTRS: &[&str] = &[
    "style", "align", "valign", "width", "height", "bgcolor", "color", "border",
    "cellpadding", "cellspacing", "dir",
];

pub struct Clean {
    pub html: String,
    /// Сколько внешних ресурсов (картинок, url() в стилях) вырезано.
    pub blocked_remote: usize,
}

/// `allow_remote`: оставить http(s)-картинки, чтобы ядро скачало их и встроило (remote.rs).
pub fn clean_html(dirty: &str, allow_remote: bool) -> Clean {
    clean(dirty, allow_remote, false)
}

/// Как `clean_html`, но с `<style>` и `class`: письма вроде LinkedIn задают цвета классами, без них
/// текст наследует цвет нашей темы и сливается с фоном. Только для iframe письма — там стили
/// никуда не выходят за пределы документа, а CSP не даёт им ходить в сеть. В редактор (цитата
/// в ответе) такое пускать нельзя: `<style>` там перекрасил бы всё окно.
pub fn clean_message(dirty: &str, allow_remote: bool) -> Clean {
    clean(dirty, allow_remote, true)
}

fn clean(dirty: &str, allow_remote: bool, styles: bool) -> Clean {
    let blocked = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&blocked);
    let relative_counter = Arc::clone(&blocked);
    let mut builder = Builder::default();
    builder.add_tags(&["font"]).add_tag_attributes("font", &["face", "size"]).add_generic_attributes(GENERIC_ATTRS);
    if styles {
        builder.add_tags(&["style"]).rm_clean_content_tags(&["style"]).add_generic_attributes(&["class"]);
    }
    let html = builder
        .url_schemes(HashSet::from(["http", "https", "mailto", "data"]))
        // Относительные URL выкидываются до attribute_filter; `//host/…` — это внешний ресурс, считаем его.
        .url_relative(UrlRelative::Custom(Box::new(DropRelative(relative_counter))))
        .link_rel(Some("noopener noreferrer nofollow"))
        .set_tag_attribute_value("a", "target", "_blank")
        .attribute_filter(move |element, attribute, value| {
            filter_attribute(&counter, allow_remote, element, attribute, value)
        })
        .clean(dirty)
        .to_string();
    let html = if styles { clean_style_blocks(&html) } else { html };
    Clean { html, blocked_remote: blocked.load(Ordering::Relaxed) }
}

/// Содержимое `<style>` после ammonia: атрибутов у тега нет, а `</style` внутри быть не может —
/// парсер закончил бы на нём элемент. Поэтому блоки находятся простым поиском.
fn clean_style_blocks(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("<style>").map(|i| i + "<style>".len()) {
        let Some(end) = rest[start..].find("</style>").map(|i| start + i) else { break };
        out.push_str(&rest[..start]);
        out.push_str(&clean_css(&rest[start..end]));
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Внешние ресурсы из таблиц стилей не грузим (их и CSP не пустит): `url(…)` превращается в
/// неизвестную функцию, и браузер отбрасывает только это объявление. Экранирование и `@import`
/// — признак обхода фильтров, такой блок выкидываем целиком.
fn clean_css(css: &str) -> String {
    let lower = css.to_ascii_lowercase();
    if lower.contains('\\') || lower.contains("@import") || lower.contains("expression(") {
        return String::new();
    }
    let mut out = String::with_capacity(css.len());
    let mut from = 0;
    while let Some(at) = ["url(", "image-set("].iter().filter_map(|f| lower[from..].find(f)).min().map(|i| from + i) {
        out.push_str(&css[from..at]);
        out.push_str("blocked-");
        from = at;
        // Следующий поиск начинается после имени функции, иначе цикл найдёт её снова.
        let name = if lower[at..].starts_with("url(") { 4 } else { 10 };
        out.push_str(&css[at..at + name]);
        from += name;
    }
    out.push_str(&css[from..]);
    out
}

struct DropRelative(Arc<AtomicUsize>);

impl UrlRelativeEvaluate<'_> for DropRelative {
    fn evaluate<'url>(&self, url: &'url str) -> Option<Cow<'url, str>> {
        if url.starts_with("//") {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        None
    }
}

fn filter_attribute<'u>(
    blocked: &AtomicUsize,
    allow_remote: bool,
    element: &str,
    attribute: &str,
    value: &'u str,
) -> Option<Cow<'u, str>> {
    let lower = value.trim().to_ascii_lowercase();
    match (element, attribute) {
        ("img", "src") => {
            if lower.starts_with("data:image/") && !lower.starts_with("data:image/svg") {
                return Some(value.into());
            }
            let remote = lower.starts_with("https://") || lower.starts_with("http://");
            if remote || lower.starts_with("//") {
                blocked.fetch_add(1, Ordering::Relaxed);
            }
            (remote && allow_remote).then(|| value.into())
        }
        ("a", "href") => {
            let ok = ["https://", "http://", "mailto:"].iter().any(|p| lower.starts_with(p));
            ok.then(|| value.into())
        }
        (_, "style") => {
            if lower.contains('\\') || lower.contains("expression(") || lower.contains("@import") {
                // Экранирование в CSS (`u\72l(`) — классический обход фильтров; такой стиль выкидываем целиком.
                None
            } else if lower.contains("url(") || lower.contains("image-set(") {
                blocked.fetch_add(1, Ordering::Relaxed);
                (allow_remote && !lower.contains("image-set(")).then(|| value.into())
            } else {
                Some(value.into())
            }
        }
        _ => Some(value.into()),
    }
}

/// Холст HTML-письма. Почтовые программы рисуют письмо на белом, и рассылки на это рассчитывают:
/// где цвет не задан, подразумевается чёрный на белом. Поэтому «богатое» письмо (таблицы,
/// картинки, фон) кладём на холст: фон `<body>` письма, иначе фон первого блока на всю ширину
/// (обёртка рассылки), иначе белый. Простой текст и короткие письма людей остаются на стекле.
pub struct Canvas {
    pub color: String,
    pub dark: bool,
}

/// `dirty` — исходный HTML (санитайзер выбрасывает `<body>` вместе с его фоном), `clean` — очищенный.
pub fn canvas(dirty: &str, clean: &str) -> Option<Canvas> {
    let body = RefCell::new(None);
    let _ = rewrite_str(dirty, Settings::new().append_element_content_handler(element!("body", |el| {
        *body.borrow_mut() = background(el);
        Ok(())
    })));

    // Первый блок с фоном и на всю ли он ширину. Дальше смотреть незачем: внутренние блоки
    // (карточка 600 px, плашки) лежат уже на холсте.
    let first = RefCell::new(None::<(String, bool)>);
    let rich = Cell::new(false);
    let settings = Settings::new()
        .append_element_content_handler(element!("table, td, th, div, center, img", |el| {
            let tag = el.tag_name();
            rich.set(rich.get() || tag == "img" || tag == "table");
            if tag != "img" && first.borrow().is_none() {
                if let Some(color) = background(el) {
                    *first.borrow_mut() = Some((color, full_width(el)));
                }
            }
            Ok(())
        }))
        // Письмо само красит текст или фон (серый подвал, кнопка-ссылка) — значит, свёрстано под
        // белый фон: на стекле такой текст пропадает.
        .append_element_content_handler(element!("[style], [bgcolor], font[color]", |el| {
            let style = el.get_attribute("style").unwrap_or_default();
            if el.has_attribute("bgcolor") || el.has_attribute("color") || background(el).is_some() || sets_text_color(&style) {
                rich.set(true);
            }
            Ok(())
        }));
    let _ = rewrite_str(clean, settings);

    let (body, first) = (body.into_inner(), first.into_inner());
    if body.is_none() && first.is_none() && !rich.get() {
        return None;
    }
    let color = body.or(first.and_then(|(c, full)| full.then_some(c))).unwrap_or_else(|| "#ffffff".into());
    Some(Canvas { dark: is_dark(&color), color })
}

fn background(el: &lol_html::html_content::Element) -> Option<String> {
    el.get_attribute("style")
        .and_then(|s| style_background(&s))
        .or_else(|| el.get_attribute("bgcolor").filter(|c| is_plain_color(c)).map(|c| c.trim().to_string()))
}

/// Есть ли во встроенном стиле объявление `color:` (не `background-color`, не `border-color`).
fn sets_text_color(style: &str) -> bool {
    style.split(';').any(|decl| decl.split_once(':').is_some_and(|(name, value)| name.trim().eq_ignore_ascii_case("color") && !value.trim().is_empty()))
}

/// Блок на всю ширину: `width="100%"`, `width:100%` в стиле или `div` без заданной ширины.
fn full_width(el: &lol_html::html_content::Element) -> bool {
    let style = el.get_attribute("style").unwrap_or_default().to_ascii_lowercase().replace(' ', "");
    match el.get_attribute("width") {
        Some(w) => w.trim() == "100%",
        None if style.contains("width:100%") => true,
        None => matches!(el.tag_name().as_str(), "div" | "center") && !style.contains("width"),
    }
}

/// Тёмный ли холст: по яркости для `#hex`, для имён — по короткому списку тёмных.
fn is_dark(color: &str) -> bool {
    let lower = color.trim().to_ascii_lowercase();
    let Some(hex) = lower.strip_prefix('#') else {
        return ["black", "navy", "darkblue", "midnightblue", "maroon", "purple", "indigo", "darkgreen", "darkslategray", "darkslategrey", "dimgray", "dimgrey"].contains(&lower.as_str());
    };
    let hex: String = if hex.len() == 3 { hex.chars().flat_map(|c| [c, c]).collect() } else { hex.to_string() };
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_or(255.0, f64::from);
    0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4) < 128.0
}

/// Цвет из `background-color:` или первого слова `background:` во встроенном стиле.
/// lol_html отдаёт атрибут с сущностями, но в цвете их не бывает — такой просто не пройдёт проверку.
fn style_background(style: &str) -> Option<String> {
    style.split(';').rev().find_map(|decl| {
        let (name, value) = decl.split_once(':')?;
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().trim_end_matches("!important").trim();
        let color = match name.as_str() {
            "background-color" => value,
            "background" => value.split_whitespace().next()?,
            _ => return None,
        };
        is_plain_color(color).then(|| color.to_string())
    })
}

/// Цвет попадает в наш CSS — пускаем только `#hex` и имена из букв. `transparent` и
/// служебные слова не дают сплошного фона.
fn is_plain_color(c: &str) -> bool {
    let c = c.trim();
    let lower = c.to_ascii_lowercase();
    if ["transparent", "none", "inherit", "initial", "unset", "currentcolor"].contains(&lower.as_str()) {
        return false;
    }
    match c.strip_prefix('#') {
        Some(hex) => [3, 6].contains(&hex.len()) && hex.bytes().all(|b| b.is_ascii_hexdigit()),
        None => !c.is_empty() && c.len() <= 20 && c.bytes().all(|b| b.is_ascii_alphabetic()),
    }
}

/// Документ без холста (перевод письма).
pub fn wrap(body: &str) -> String {
    wrap_message(body, None)
}

/// Документ письма; с холстом письмо лежит на его цвете.
pub fn wrap_message(body: &str, canvas: Option<&Canvas>) -> String {
    // Цвет прошёл is_plain_color: только `#hex` или буквы, дописать свой CSS через него нельзя.
    let body = match canvas {
        // Прокрутка и маска краёв — на своём блоке, а не на <body>: стили письма часто красят body,
        // а его фон уходит на весь холст страницы мимо маски. Холст шире окна, если письмо не
        // ужимается (таблица 600 px при 300%), — тогда появляется горизонтальная прокрутка.
        // Свечение (до 27 px) уже бокового отступа (28 px) и в полосу прокрутки не попадает.
        Some(c) => format!(
            "<div data-mmail-scroll><div data-mmail-frame><div data-mmail-canvas style=\"--canvas:{}\">{body}</div></div></div>",
            c.color
        ),
        None => body.to_string(),
    };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta http-equiv=\"Content-Security-Policy\" content=\"{FRAME_CSP}\">\
<meta name=\"referrer\" content=\"no-referrer\">\
<style>{BASE_CSS}{SCROLLBAR_CSS}{ZOOM_CSS}</style></head><body>{body}</body></html>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean_html_blocked(dirty: &str) -> Clean {
        clean_html(dirty, false)
    }

    #[test]
    fn allow_remote_keeps_only_http_images() {
        let c = clean_html(
            r#"<img src="https://a.example/1.png"><img src="javascript:x"><div style="background:url(https://a.example/bg.png)">a</div><div style="background:u\72l(https://t.example/x)">b</div>"#,
            true,
        );
        assert!(c.html.contains(r#"src="https://a.example/1.png""#));
        assert!(c.html.contains("url(https://a.example/bg.png)"));
        assert!(!c.html.contains("javascript") && !c.html.contains("t.example"), "{}", c.html);
    }

    #[test]
    fn strips_active_content() {
        let c = clean_html_blocked(
            r#"<script>alert(1)</script><img src="x" onerror="alert(2)">
<svg onload="alert(3)"></svg><iframe src="https://evil.example"></iframe>
<form action="https://evil.example"><input name="p"></form>
<meta http-equiv="refresh" content="0;url=https://evil.example">
<a href="javascript:alert(4)">js</a><a href="data:text/html,hi">data</a>"#,
        );
        for bad in ["<script", "onerror", "onload", "<iframe", "<form", "<input", "<meta", "javascript:", "data:text"] {
            assert!(!c.html.contains(bad), "{bad} survived: {}", c.html);
        }
    }

    #[test]
    fn blocks_remote_resources() {
        let c = clean_html_blocked(
            r#"<img src="https://t.example/p.gif"><img src="//t.example/q.gif">
<div style="background:url(https://t.example/bg.png)">a</div>
<div style="background:u\72l(https://t.example/x)">b</div>"#,
        );
        assert_eq!(c.blocked_remote, 3);
        assert!(!c.html.contains("t.example"));
    }

    #[test]
    fn frame_document_forbids_network_and_scripts() {
        let doc = wrap_message("<p>x</p>", None);
        // Второй слой после санитайзера: даже пропущенный тег не сходит в сеть и не выполнит скрипт.
        assert!(doc.contains(r#"http-equiv="Content-Security-Policy""#), "{doc}");
        for rule in ["default-src 'none'", "img-src data:", "base-uri 'none'", "form-action 'none'"] {
            assert!(FRAME_CSP.contains(rule), "{rule}");
        }
        assert!(!FRAME_CSP.contains("script-src") && !FRAME_CSP.contains("http"), "скрипты и сеть не разрешаются: {FRAME_CSP}");
        assert!(doc.contains(r#"<meta name="referrer" content="no-referrer">"#));

        assert!(!doc.contains("<script"), "в документе письма скриптов нет: {doc}");
    }

    #[test]
    fn links_open_in_new_window_without_referrer() {
        // target=_blank — чтобы клик дошёл до ядра как запрос окна и прошёл подтверждение.
        let c = clean_html_blocked(r#"<a href="https://a.example" target="_self" rel="opener">a</a>"#);
        assert!(c.html.contains(r#"target="_blank""#) && c.html.contains("noreferrer") && !c.html.contains("opener\""), "{}", c.html);
    }

    #[test]
    fn message_keeps_styles_without_resources() {
        let c = clean_message(
            r#"<style>.f{color:#000}.b{background:url(https://t.example/b.png)}</style><p class="f" onclick="x()">a</p>"#,
            true,
        );
        assert!(c.html.contains(".f{color:#000}") && c.html.contains(r#"class="f""#), "{}", c.html);
        assert!(c.html.contains("blocked-url(") && !c.html.contains(":url("), "{}", c.html);
        assert!(!c.html.contains("onclick"));
        for evil in ["<style>@import 'https://t.example/x.css';p{}</style>", r"<style>p{background:u\72l(https://t.example/x)}</style>"] {
            let c = clean_message(evil, true);
            assert!(!c.html.contains("t.example"), "{}", c.html);
        }
        let c = clean_message("<style>p{}</style><script>alert(1)</script></style><img src=x onerror=alert(2)>", false);
        assert!(!c.html.contains("<script") && !c.html.contains("onerror"), "{}", c.html);
    }

    #[test]
    fn canvas_follows_the_letter() {
        let color = |dirty: &str| canvas(dirty, &clean_message(dirty, false).html).map(|c| (c.color, c.dark));
        // Простой текст (так его отдаёт mail-parser) и короткое письмо человека — без холста.
        assert!(color("<html><body>Привет<br/>как дела</body></html>").is_none());
        assert!(color(r#"<div dir="ltr">Hi <b>there</b></div>"#).is_none());
        assert!(color(r#"<p style="margin:0;border-color:red">x</p>"#).is_none());
        // Свой цвет текста или кнопка-ссылка с фоном — свёрстано под белый.
        assert_eq!(color(r#"<p style="font-size:12px; color: #999">footer</p>"#), Some(("#ffffff".into(), false)));
        assert_eq!(color(r#"<p><a href="https://a.example" style="background:#00e38c">Go</a></p>"#), Some(("#ffffff".into(), false)));
        assert_eq!(color(r#"<font color="gray">x</font>"#), Some(("#ffffff".into(), false)));
        // Карточка 600 px на прозрачном — белый холст, как в Thunderbird.
        let card = r##"<table width="100%"><tr><td><table width="600" bgcolor="#FFFFFF"><tr><td>x</td></tr></table></td></tr></table>"##;
        assert_eq!(color(card), Some(("#ffffff".into(), false)));
        // Обёртка на всю ширину задаёт цвет холста.
        assert_eq!(color(r##"<div style="background-color:#ddd6f3"><table width="600"><tr><td>x</td></tr></table></div>"##), Some(("#ddd6f3".into(), false)));
        // Фон <body> — даже если санитайзер его выбросил.
        assert_eq!(color(r##"<html><body bgcolor="#111111"><p>x</p></body></html>"##), Some(("#111111".into(), true)));
        // Попытка дописать CSS через цвет не проходит — остаётся белый.
        assert_eq!(color(r#"<body bgcolor="red;background:url(x)"><img src="data:image/png;base64,AA"></body>"#), Some(("#ffffff".into(), false)));
    }

    #[test]
    fn frame_wraps_canvas() {
        let doc = wrap_message("<p>x</p>", Some(&Canvas { color: "#ddd6f3".into(), dark: false }));
        assert!(doc.contains(r##"<body><div data-mmail-scroll><div data-mmail-frame><div data-mmail-canvas style="--canvas:#ddd6f3"><p>x</p></div></div></div></body>"##), "{doc}");
    }

    #[test]
    fn quotes_keep_no_styles() {
        // Цитата в редакторе ответа: <style> там действовал бы на всё окно.
        let c = clean_html("<style>body{display:none}</style><p class=\"x\">a</p>", false);
        assert!(!c.html.contains("display") && !c.html.contains("class"), "{}", c.html);
    }

    #[test]
    fn keeps_safe_formatting() {
        let c = clean_html_blocked(
            r#"<p style="color:red">hi</p><a href="https://github.com">gh</a><img src="data:image/png;base64,AAAA">"#,
        );
        assert!(c.html.contains(r#"style="color:red""#));
        assert!(c.html.contains(r#"href="https://github.com""#));
        assert!(c.html.contains("noopener"));
        assert!(c.html.contains("data:image/png"));
        assert_eq!(c.blocked_remote, 0);
    }
}
