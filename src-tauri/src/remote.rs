//! Внешнее содержимое писем по явному разрешению пользователя.
//! Картинки скачивает ядро (без cookies и Referer) и встраивает как data:, поэтому
//! iframe письма по-прежнему не ходит в сеть сам: его CSP остаётся `img-src data:`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use base64::Engine;
use lol_html::{element, HtmlRewriter, Settings};
use ureq::config::Config;
use ureq::http::Uri;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};
use url::Url;

const MAX_IMAGES: usize = 60;
const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
/// Общий бюджет на все внешние картинки одного письма. 60 картинок по 5 МБ — это
/// сотни мегабайт в памяти и в HTML, поэтому ограничиваем и сумму.
const MAX_TOTAL_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const MAX_REDIRECTS: usize = 3;
const PARALLEL: usize = 6;

/// Скачивает картинки из очищенного HTML и подставляет их как data: URI.
/// Всё, что не удалось или нельзя загрузить, из письма убирается.
pub async fn inline(html: String) -> String {
    let urls = collect(&html);
    let mut fetched = HashMap::new();
    let mut total = 0usize;
    for chunk in urls.chunks(PARALLEL) {
        let tasks: Vec<_> = chunk
            .iter()
            .cloned()
            .map(|url| tauri::async_runtime::spawn_blocking(move || (fetch_image(&url), url)))
            .collect();
        for task in tasks {
            if let Ok((Some(data), url)) = task.await {
                if fits_total(total, data.len()) {
                    total += data.len();
                    fetched.insert(url, data);
                }
            }
        }
    }
    rewrite(&html, &fetched)
}

fn fits_total(total: usize, data_len: usize) -> bool {
    total.saturating_add(data_len) <= MAX_TOTAL_IMAGE_BYTES
}

/// Адреса картинок: `<img src>` и `url(…)` в стилях. Трекинг-пиксели 1×1 пропускаем.
fn collect(html: &str) -> Vec<String> {
    let urls = RefCell::new(Vec::<String>::new());
    let add = |url: String| {
        let mut urls = urls.borrow_mut();
        if urls.len() < MAX_IMAGES && !urls.contains(&url) {
            urls.push(url);
        }
    };
    let settings = Settings::new()
        .append_element_content_handler(element!("img[src]", |el| {
            if !is_pixel(el.get_attribute("width"), el.get_attribute("height")) {
                add(unescape(&el.get_attribute("src").unwrap_or_default()));
            }
            Ok(())
        }))
        .append_element_content_handler(element!("[style]", |el| {
            for url in style_urls(&el.get_attribute("style").unwrap_or_default()) {
                add(unescape(&url));
            }
            Ok(())
        }));
    let mut rewriter = HtmlRewriter::new(settings, |_: &[u8]| {});
    let _ = rewriter.write(html.as_bytes()).and_then(|()| rewriter.end());
    urls.into_inner().into_iter().filter(|u| u.starts_with("https://") || u.starts_with("http://")).collect()
}

fn rewrite(html: &str, fetched: &HashMap<String, String>) -> String {
    let settings = Settings::new()
        .append_element_content_handler(element!("img[src]", |el| {
            match fetched.get(&unescape(&el.get_attribute("src").unwrap_or_default())) {
                Some(data) => el.set_attribute("src", data)?,
                None => el.remove_attribute("src"),
            }
            Ok(())
        }))
        .append_element_content_handler(element!("[style]", |el| {
            let mut style = el.get_attribute("style").unwrap_or_default();
            for url in style_urls(&style) {
                let replacement = fetched.get(&unescape(&url)).map_or("none", String::as_str);
                style = style.replace(&url, replacement);
            }
            el.set_attribute("style", &style)?;
            Ok(())
        }));
    let mut out = Vec::new();
    let mut rewriter = HtmlRewriter::new(settings, |chunk: &[u8]| out.extend_from_slice(chunk));
    if rewriter.write(html.as_bytes()).and_then(|()| rewriter.end()).is_err() {
        return String::new();
    }
    String::from_utf8(out).unwrap_or_default()
}

/// lol_html отдаёт атрибуты как есть, с сущностями: без этого `?e=1&amp;t=…` уходит на сервер
/// буквально, и подписанные ссылки (логотипы на media.licdn.com) отвечают 403. После ammonia
/// в атрибутах бывают только эти три сущности.
fn unescape(value: &str) -> String {
    value.replace("&quot;", "\"").replace("&nbsp;", "\u{a0}").replace("&amp;", "&")
}

fn is_pixel(width: Option<String>, height: Option<String>) -> bool {
    let tiny = |v: Option<String>| v.and_then(|v| v.trim_end_matches("px").trim().parse::<u32>().ok()).is_some_and(|n| n <= 2);
    tiny(width) || tiny(height)
}

/// Адреса внутри `url(…)`. Стили с экранированием санитайзер уже выбросил.
fn style_urls(style: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let lower = style.to_ascii_lowercase();
    let mut from = 0;
    while let Some(start) = lower[from..].find("url(").map(|i| from + i + 4) {
        let Some(end) = lower[start..].find(')').map(|i| start + i) else { break };
        let url = style[start..end].trim().trim_matches(['"', '\'']).trim();
        if !url.is_empty() {
            urls.push(url.to_string());
        }
        from = end;
    }
    urls
}

/// Адреса проверяет сам резолвер ureq, и соединение идёт ровно на проверенные IP. Отдельная
/// проверка до запроса не годится: DNS с TTL 0 ответил бы ей публичным адресом, а при
/// подключении — 192.168.1.1 (DNS rebinding).
#[derive(Debug, Default)]
struct PublicOnly(DefaultResolver);

impl Resolver for PublicOnly {
    fn resolve(&self, uri: &Uri, config: &Config, timeout: NextTimeout) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let addrs = self.0.resolve(uri, config, timeout)?;
        if addrs.iter().all(|a| is_public(a.ip())) {
            Ok(addrs)
        } else {
            Err(ureq::Error::HostNotFound)
        }
    }
}

fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .max_redirects(0)
        .http_status_as_error(false)
        .user_agent("Mozilla/5.0")
        // Через прокси резолвер видел бы адрес прокси, а не сайта — проверка потеряла бы смысл.
        .proxy(None)
        .build();
    ureq::Agent::with_parts(config, DefaultConnector::default(), PublicOnly::default())
}

fn fetch_image(url: &str) -> Option<String> {
    let agent = agent();
    let mut current = Url::parse(url).ok()?;
    for _ in 0..=MAX_REDIRECTS {
        if !matches!(current.scheme(), "http" | "https") {
            return None;
        }
        let mut response = agent.get(current.as_str()).call().ok()?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            let location = response.headers().get("location")?.to_str().ok()?;
            current = current.join(location).ok()?;
            continue;
        }
        if status != 200 {
            return None;
        }
        let mime = response.headers().get("content-type")?.to_str().ok()?.split(';').next()?.trim().to_ascii_lowercase();
        if !is_image_mime(&mime) {
            return None;
        }
        let bytes = response.body_mut().with_config().limit(MAX_IMAGE_BYTES).read_to_vec().ok()?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        return Some(format!("data:{mime};base64,{encoded}"));
    }
    None
}

/// Тип из ответа чужого сервера попадает внутрь `data:` в атрибуте и в `url(…)` стиля — пускаем
/// только `image/<подтип>` без скобок, кавычек и пробелов, которыми можно дописать свой CSS.
fn is_image_mime(mime: &str) -> bool {
    mime.strip_prefix("image/").is_some_and(|sub| {
        let sub = sub.to_ascii_lowercase();
        !sub.is_empty()
            && !sub.starts_with("svg")
            && sub.len() <= 64
            && sub
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b))
    })
}

/// Не даём письму через ядро стучаться в локальную сеть (роутер, localhost и т.п.).
/// Для IPv6 дополнительно разбираем transition-механизмы: в NAT64/6to4-адресе может быть
/// зашит приватный IPv4, и простая проверка «это не fc00::/7» его пропустит.
fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || (a == 100 && (64..128).contains(&b)) // CGNAT
                || (a == 198 && (b == 18 || b == 19)) // benchmark 198.18.0.0/15
                || a == 0
                || a >= 224)
        }
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            let first = segments[0];
            // IPv4-mapped ::ffff:a.b.c.d — проверяем вложенный адрес.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            // 6to4 2002:AABB:CCDD::/48 — в AABB:CCDD лежит IPv4.
            if first == 0x2002 {
                let v4 = Ipv4Addr::new(
                    (segments[1] >> 8) as u8,
                    segments[1] as u8,
                    (segments[2] >> 8) as u8,
                    segments[2] as u8,
                );
                return is_public(IpAddr::V4(v4));
            }
            // NAT64 well-known prefix 64:ff9b::/96 — последние 32 бита IPv4.
            // 64:ff9b:1::/48 (local-use) и Teredo 2001:0::/32 не разбираем — запрещаем целиком.
            if first == 0x0064 && segments[1] == 0xff9b {
                if segments[2] == 0 && segments[3] == 0 && segments[4] == 0 && segments[5] == 0 {
                    let v4 = Ipv4Addr::new(
                        (segments[6] >> 8) as u8,
                        segments[6] as u8,
                        (segments[7] >> 8) as u8,
                        segments[7] as u8,
                    );
                    return is_public(IpAddr::V4(v4));
                }
                return false;
            }
            if first == 0x2001 && segments[1] == 0x0000 {
                return false; // Teredo
            }
            if first == 0x2001 && segments[1] == 0x0db8 {
                return false; // documentation 2001:db8::/32
            }
            // multicast ff00::/8
            if first & 0xff00 == 0xff00 {
                return false;
            }
            !(v6.is_loopback()
                || v6.is_unspecified()
                || (first & 0xfe00) == 0xfc00 // unique local
                || (first & 0xffc0) == 0xfe80 // link-local
                || v6.to_ipv4_mapped().is_some_and(|v4| !is_public(IpAddr::V4(v4))))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_images_but_not_pixels() {
        let html = r#"<img src="https://a.example/1.png"><img src="https://t.example/p.gif" width="1" height="1">
<div style="background:url('https://a.example/bg.jpg')">x</div><img src="data:image/png;base64,AA">"#;
        assert_eq!(collect(html), ["https://a.example/1.png", "https://a.example/bg.jpg"]);
    }

    #[test]
    fn rewrites_fetched_and_drops_rest() {
        let html = r#"<img src="https://a.example/1.png"><img src="https://b.example/2.png"><div style="background:url(https://c.example/x.png)">x</div>"#;
        let fetched = HashMap::from([("https://a.example/1.png".to_string(), "data:image/png;base64,AA".to_string())]);
        let out = rewrite(html, &fetched);
        assert!(out.contains(r#"src="data:image/png;base64,AA""#));
        assert!(!out.contains("b.example") && !out.contains("c.example"), "{out}");
    }

    #[test]
    fn decodes_entities_in_urls() {
        let html = r#"<img src="https://a.example/1.png?e=1&amp;t=2"><div style="background:url(https://a.example/b.png?x=1&amp;y=2)">x</div>"#;
        assert_eq!(collect(html), ["https://a.example/1.png?e=1&t=2", "https://a.example/b.png?x=1&y=2"]);
        let fetched = HashMap::from([
            ("https://a.example/1.png?e=1&t=2".to_string(), "data:image/png;base64,AA".to_string()),
            ("https://a.example/b.png?x=1&y=2".to_string(), "data:image/png;base64,BB".to_string()),
        ]);
        let out = rewrite(html, &fetched);
        assert!(out.contains(r#"src="data:image/png;base64,AA""#) && out.contains("url(data:image/png;base64,BB)"), "{out}");
    }

    #[test]
    fn refuses_local_network() {
        for url in ["http://127.0.0.1/x.png", "http://192.168.1.1/x.png", "http://10.0.0.5/", "http://[::1]/", "http://169.254.169.254/latest", "http://localhost/x.png", "file:///etc/passwd"] {
            assert!(fetch_image(url).is_none(), "{url}");
        }
        assert!(is_public("93.184.216.34".parse().unwrap()));
    }

    #[test]
    fn only_plain_image_types() {
        for ok in [
            "image/png",
            "image/jpeg",
            "image/vnd.microsoft.icon",
            "image/x-icon",
        ] {
            assert!(is_image_mime(ok), "{ok}");
        }
        for bad in [
            "image/",
            "text/html",
            "image/svg+xml",
            "image/svg",
            "image/png),url(https://t.example/x",
            "image/png\"",
            "image/x y",
            "image/png;base64",
        ] {
            assert!(!is_image_mime(bad), "{bad}");
        }
    }

    #[test]
    fn total_image_budget_is_bounded() {
        assert!(fits_total(0, MAX_TOTAL_IMAGE_BYTES));
        assert!(!fits_total(MAX_TOTAL_IMAGE_BYTES, 1));
        assert!(!fits_total(usize::MAX, 1), "переполнение не обходит лимит");
    }

    #[test]
    fn refuses_ipv6_transition_addresses_with_private_ipv4() {
        for ip in [
            "2002:c0a8:0101::",     // 6to4 -> 192.168.1.1
            "64:ff9b::c0a8:0101",   // NAT64 -> 192.168.1.1
            "64:ff9b:1::c0a8:0101", // local-use NAT64
            "2001:0:4136:e378::",   // Teredo
            "2001:db8::1",          // documentation
            "ff02::1",              // multicast
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        assert!(
            is_public("2002:5db8:d822::".parse().unwrap()),
            "6to4 с публичным IPv4"
        );
        assert!(
            is_public("64:ff9b::5db8:d822".parse().unwrap()),
            "NAT64 с публичным IPv4"
        );
    }

    #[test]
    fn resolver_refuses_local_addresses() {
        // Проверка в резолвере — то есть на тех адресах, куда ureq и подключится.
        let timeout = NextTimeout { after: ureq::unversioned::transport::time::Duration::NotHappening, reason: ureq::Timeout::Global };
        let config = Config::default();
        for uri in ["http://127.0.0.1/", "http://localhost:8080/", "https://[::1]/", "http://192.168.0.1/"] {
            let uri: Uri = uri.parse().unwrap();
            assert!(PublicOnly::default().resolve(&uri, &config, timeout).is_err(), "{uri}");
        }
        let public: Uri = "http://93.184.216.34/".parse().unwrap();
        assert!(PublicOnly::default().resolve(&public, &config, timeout).is_ok());
    }
}
