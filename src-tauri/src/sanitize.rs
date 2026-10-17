//! Очистка HTML писем. Первый из двух слоёв защиты: второй — sandbox-iframe с CSP в UI.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use ammonia::{Builder, UrlRelative, UrlRelativeEvaluate};

/// Политика внутри iframe письма: никаких сетевых запросов, только inline-стили и data:-картинки.
const FRAME_CSP: &str =
    "default-src 'none'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'; form-action 'none'";

const BASE_CSS: &str = "\
html,body{margin:0;background:transparent;color:#e7e4f0;\
font:15px/1.6 'Inter','Noto Sans',system-ui,sans-serif;overflow-wrap:anywhere}\
body{padding:0 4px 16px 0}\
a{color:#b3a8ff}\
img{max-width:100%;height:auto}\
table{max-width:100%}\
blockquote{margin:0 0 0 .6em;padding-left:.8em;border-left:2px solid rgba(255,255,255,.2);color:#bdb8cc}\
::-webkit-scrollbar{width:8px}\
::-webkit-scrollbar-thumb{background:rgba(255,255,255,.15);border-radius:4px}";

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
    let blocked = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&blocked);
    let relative_counter = Arc::clone(&blocked);
    let html = Builder::default()
        .add_tags(&["font"])
        .add_tag_attributes("font", &["face", "size"])
        .add_generic_attributes(GENERIC_ATTRS)
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
    Clean { html, blocked_remote: blocked.load(Ordering::Relaxed) }
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

/// Полный документ для `<iframe srcdoc>`.
pub fn wrap(body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta http-equiv=\"Content-Security-Policy\" content=\"{FRAME_CSP}\">\
<meta name=\"referrer\" content=\"no-referrer\">\
<style>{BASE_CSS}</style></head><body>{body}</body></html>"
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
