//! Ссылки из писем: поиск обманок («текст говорит github.com, ведёт на evil.example»)
//! и открытие во внешнем браузере только после подтверждения пользователем.

use std::cell::RefCell;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use lol_html::{element, text, HtmlRewriter, Settings};
use serde::Serialize;
use url::Url;

/// Ссылка, видимый текст которой выглядит как адрес другого домена.
pub struct Deceptive {
    pub text: String,
    pub real_host: String,
}

/// Ищет обманные ссылки в уже очищенном HTML.
pub fn find_deceptive(html: &str) -> Vec<Deceptive> {
    let links: RefCell<Vec<(String, String)>> = RefCell::new(Vec::new());
    let settings = Settings::new()
        .append_element_content_handler(element!("a[href]", |el| {
            let href = el.get_attribute("href").unwrap_or_default();
            links.borrow_mut().push((href, String::new()));
            Ok(())
        }))
        .append_element_content_handler(text!("a[href]", |chunk| {
            if let Some((_, text)) = links.borrow_mut().last_mut() {
                text.push_str(chunk.as_str());
            }
            Ok(())
        }));
    let mut rewriter = HtmlRewriter::new(settings, |_: &[u8]| {});
    if rewriter.write(html.as_bytes()).and_then(|()| rewriter.end()).is_err() {
        return Vec::new();
    }

    links
        .into_inner()
        .into_iter()
        .filter_map(|(href, text)| {
            let text = text.trim().to_string();
            let claimed = host_in_text(&text)?;
            let real = Url::parse(&href).ok()?.host_str()?.to_ascii_lowercase();
            (!same_site(&claimed, &real)).then_some(Deceptive { text, real_host: real })
        })
        .collect()
}

/// Если текст ссылки выглядит как адрес («github.com/x», «https://www.paypal.com») — его хост.
fn host_in_text(text: &str) -> Option<String> {
    if text.is_empty() || text.contains(char::is_whitespace) {
        return None;
    }
    let with_scheme = if text.contains("://") { text.to_string() } else { format!("http://{text}") };
    let host = Url::parse(&with_scheme).ok()?.host_str()?.to_ascii_lowercase();
    // Нужна хотя бы одна точка: иначе любое слово («Выбрать») стало бы «доменом» xn--….
    let (_, tld) = host.rsplit_once('.')?;
    let plain_tld = tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic());
    (plain_tld || tld.starts_with("xn--")).then_some(host)
}

/// `docs.github.com` — тот же сайт, что и `github.com`; `github.com.evil.example` — нет.
fn same_site(claimed: &str, real: &str) -> bool {
    let claimed = claimed.strip_prefix("www.").unwrap_or(claimed);
    let real = real.strip_prefix("www.").unwrap_or(real);
    real == claimed || real.ends_with(&format!(".{claimed}"))
}

/// Разрешены только веб-ссылки и почта; всё остальное (file:, javascript:, data:, …) не открываем.
pub fn openable(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https" | "mailto")
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkRequest {
    id: u64,
    url: String,
    /// Хост в ASCII: омоглифы видны как `xn--…`.
    host: String,
    insecure: bool,
    punycode: bool,
}

/// Адрес, ждущий подтверждения. UI возвращает только id, поэтому открыть
/// произвольный URL через ядро нельзя — только тот, что пользователь кликнул.
#[derive(Default)]
pub struct PendingLink {
    next_id: AtomicU64,
    current: Mutex<Option<(u64, Url)>>,
}

impl PendingLink {
    pub fn request(&self, url: Url) -> LinkRequest {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let host = url.host_str().unwrap_or_default().to_string();
        let request = LinkRequest {
            id,
            url: url.to_string(),
            punycode: host.split('.').any(|label| label.starts_with("xn--")),
            insecure: url.scheme() == "http",
            host,
        };
        *self.current.lock().unwrap() = Some((id, url));
        request
    }

    fn take(&self, id: u64) -> Option<Url> {
        let mut current = self.current.lock().unwrap();
        match current.take() {
            Some((pending, url)) if pending == id => Some(url),
            other => {
                *current = other;
                None
            }
        }
    }
}

#[tauri::command]
pub fn open_link(pending: tauri::State<'_, PendingLink>, id: u64) -> Result<(), String> {
    let url = pending.take(id).ok_or("ссылка устарела")?;
    // Без shell: URL уходит отдельным аргументом, подстановки невозможны.
    Command::new("xdg-open").arg(url.as_str()).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hosts(html: &str) -> Vec<(String, String)> {
        find_deceptive(html).into_iter().map(|d| (d.text, d.real_host)).collect()
    }

    #[test]
    fn flags_text_pointing_elsewhere() {
        let found = hosts(r#"<a href="https://github-verify.evil.example/login">https://github.com/settings</a>"#);
        assert_eq!(found, [("https://github.com/settings".into(), "github-verify.evil.example".into())]);
        let nested = hosts(r#"<a href="https://evil.example/"><b>paypal.com</b></a>"#);
        assert_eq!(nested.len(), 1);
    }

    #[test]
    fn flags_cyrillic_domain_text() {
        assert_eq!(hosts(r#"<a href="https://evil.example/">госуслуги.рф</a>"#).len(), 1);
    }

    #[test]
    fn flags_lookalike_suffix() {
        assert_eq!(hosts(r#"<a href="https://github.com.evil.example/">github.com</a>"#).len(), 1);
    }

    #[test]
    fn accepts_honest_links() {
        assert!(hosts(r#"<a href="https://docs.github.com/x">github.com</a>"#).is_empty());
        assert!(hosts(r#"<a href="https://www.github.com/x">https://github.com/x</a>"#).is_empty());
        assert!(hosts(r#"<a href="https://habr.com/1">Открыть статью</a>"#).is_empty());
        assert!(hosts(r#"<a href="https://x.example/">e.g.</a>"#).is_empty());
        assert!(hosts(r#"<a href="https://click.example/">Выбрать</a>"#).is_empty(), "слово — не адрес");
        assert!(hosts(r#"<a href="https://click.example/">Телефоны</a>"#).is_empty());
    }

    #[test]
    fn only_web_and_mail_are_openable() {
        for ok in ["https://a.example", "http://a.example", "mailto:a@b.example"] {
            assert!(openable(&Url::parse(ok).unwrap()), "{ok}");
        }
        for bad in ["file:///etc/passwd", "javascript:alert(1)", "data:text/html,x", "smb://host/share"] {
            assert!(!openable(&Url::parse(bad).unwrap()), "{bad}");
        }
    }

    #[test]
    fn pending_link_requires_matching_id() {
        let p = PendingLink::default();
        let first = p.request(Url::parse("https://a.example").unwrap());
        let second = p.request(Url::parse("https://b.example").unwrap());
        assert!(p.take(first.id).is_none(), "старый id не должен открывать новую ссылку");
        assert_eq!(p.take(second.id).unwrap().as_str(), "https://b.example/");
        assert!(p.take(second.id).is_none(), "ссылка открывается один раз");
    }
}
