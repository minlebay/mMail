//! Внешнее содержимое писем по явному разрешению пользователя.
//! Картинки скачивает ядро (без cookies и Referer) и встраивает как data:, поэтому
//! iframe письма по-прежнему не ходит в сеть сам: его CSP остаётся `img-src data:`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::{IpAddr, ToSocketAddrs};
use std::time::Duration;

use base64::Engine;
use lol_html::{element, HtmlRewriter, Settings};
use url::Url;

const MAX_IMAGES: usize = 60;
const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
const MAX_REDIRECTS: usize = 3;
const PARALLEL: usize = 6;

/// Скачивает картинки из очищенного HTML и подставляет их как data: URI.
/// Всё, что не удалось или нельзя загрузить, из письма убирается.
pub async fn inline(html: String) -> String {
    let urls = collect(&html);
    let mut fetched = HashMap::new();
    for chunk in urls.chunks(PARALLEL) {
        let tasks: Vec<_> = chunk
            .iter()
            .cloned()
            .map(|url| tauri::async_runtime::spawn_blocking(move || (fetch_image(&url), url)))
            .collect();
        for task in tasks {
            if let Ok((Some(data), url)) = task.await {
                fetched.insert(url, data);
            }
        }
    }
    rewrite(&html, &fetched)
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
                add(el.get_attribute("src").unwrap_or_default());
            }
            Ok(())
        }))
        .append_element_content_handler(element!("[style]", |el| {
            for url in style_urls(&el.get_attribute("style").unwrap_or_default()) {
                add(url);
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
            match fetched.get(&el.get_attribute("src").unwrap_or_default()) {
                Some(data) => el.set_attribute("src", data)?,
                None => el.remove_attribute("src"),
            }
            Ok(())
        }))
        .append_element_content_handler(element!("[style]", |el| {
            let mut style = el.get_attribute("style").unwrap_or_default();
            for url in style_urls(&style) {
                let replacement = fetched.get(&url).map_or("none", String::as_str);
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

fn fetch_image(url: &str) -> Option<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .max_redirects(0)
        .http_status_as_error(false)
        .user_agent("Mozilla/5.0")
        .build()
        .into();

    let mut current = Url::parse(url).ok()?;
    for _ in 0..=MAX_REDIRECTS {
        if !public_host(&current) {
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
        if !mime.starts_with("image/") {
            return None;
        }
        let bytes = response.body_mut().with_config().limit(MAX_IMAGE_BYTES).read_to_vec().ok()?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        return Some(format!("data:{mime};base64,{encoded}"));
    }
    None
}

/// Не даём письму через ядро стучаться в локальную сеть (роутер, localhost и т.п.).
fn public_host(url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let (Some(host), Some(port)) = (url.host_str(), url.port_or_known_default()) else { return false };
    let Ok(addrs) = (host.trim_matches(['[', ']']), port).to_socket_addrs() else { return false };
    let addrs: Vec<_> = addrs.collect();
    !addrs.is_empty() && addrs.iter().all(|a| is_public(a.ip()))
}

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
                || a == 0
                || a >= 224)
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
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
    fn refuses_local_network() {
        for url in ["http://127.0.0.1/x.png", "http://192.168.1.1/x.png", "http://10.0.0.5/", "http://[::1]/", "http://169.254.169.254/latest", "file:///etc/passwd"] {
            assert!(!public_host(&Url::parse(url).unwrap()), "{url}");
        }
        assert!(is_public("93.184.216.34".parse().unwrap()));
    }
}
