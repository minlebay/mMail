//! Вложения входящих писем: что показывать, встроенные картинки (cid:), безопасные имена файлов.

use std::collections::HashMap;

use base64::Engine;
use lol_html::{element, HtmlRewriter, Settings};
use mail_parser::{Message, MessagePart, MimeHeaders};
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};

/// Встроенные картинки крупнее этого не встраиваем в просмотр.
const MAX_INLINE_IMAGE: usize = 5 * 1024 * 1024;
/// Все встроенные картинки письма вместе (в base64) — с запасом больше лимита письма в 25 МБ.
const MAX_INLINE_TOTAL: usize = 40 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentMeta {
    pub index: usize,
    pub name: String,
    pub mime: String,
    pub size: usize,
    /// Исполняемый файл или обманчивое двойное расширение (`счёт.pdf.exe`).
    pub dangerous: bool,
}

fn mime_of(part: &MessagePart) -> String {
    part.content_type()
        .map(|ct| format!("{}/{}", ct.ctype(), ct.subtype().unwrap_or("octet-stream")).to_ascii_lowercase())
        .unwrap_or_else(|| "application/octet-stream".into())
}

/// Картинка, встроенная в текст письма (`<img src="cid:…">`), а не отдельный файл.
fn is_inline_image(part: &MessagePart) -> bool {
    let inline = part.content_disposition().map_or(true, |d| d.is_inline());
    inline && part.content_id().is_some() && mime_of(part).starts_with("image/")
}

/// Вложения, которые показываем списком (без встроенных в текст картинок).
pub fn visible<'a>(parsed: &'a Message<'a>) -> Vec<&'a MessagePart<'a>> {
    parsed.attachments().filter(|p| !is_inline_image(p)).collect()
}

pub fn describe(parsed: &Message) -> Vec<AttachmentMeta> {
    visible(parsed)
        .into_iter()
        .enumerate()
        .map(|(index, part)| {
            let name = safe_name(part.attachment_name().unwrap_or("вложение"));
            AttachmentMeta { index, dangerous: is_dangerous(&name), mime: mime_of(part), size: part.contents().len(), name }
        })
        .collect()
}

/// Подставляет встроенные картинки в HTML как data: (санитайзер пропускает только data:image/, кроме SVG).
pub fn inline_cid_images(html: &str, parsed: &Message) -> String {
    inline_cid_images_within(html, parsed, MAX_INLINE_TOTAL)
}

/// Подстановка только в `<img src="cid:…">` и не больше `budget` байт на всё письмо: одна картинка,
/// упомянутая тысячи раз, иначе развернула бы письмо в мегабайт в гигабайты (в том числе при
/// фоновой подгрузке). Что не влезло, остаётся `cid:` — санитайзер такую картинку уберёт.
fn inline_cid_images_within(html: &str, parsed: &Message, budget: usize) -> String {
    let mut images: HashMap<String, (String, &MessagePart)> = HashMap::new();
    for part in parsed.attachments().filter(|p| is_inline_image(p)) {
        let (Some(cid), mime) = (part.content_id(), mime_of(part)) else { continue };
        if mime.contains("svg") || part.contents().len() > MAX_INLINE_IMAGE {
            continue;
        }
        images.entry(cid.trim_matches(['<', '>']).to_string()).or_insert((mime, part));
    }
    if images.is_empty() {
        return html.to_string();
    }

    let mut used = 0;
    let mut encoded: HashMap<&str, String> = HashMap::new();
    let settings = Settings::new().append_element_content_handler(element!("img[src]", |el| {
        let src = el.get_attribute("src").unwrap_or_default();
        let src = src.trim();
        let Some(cid) = src.get(..4).filter(|scheme| scheme.eq_ignore_ascii_case("cid:")).map(|_| &src[4..]) else { return Ok(()) };
        // RFC 2392: в ссылке id может быть закодирован (%40 вместо @).
        let decoded = percent_decode_str(cid).decode_utf8_lossy();
        let Some((cid, (mime, part))) = images.get_key_value(cid).or_else(|| images.get_key_value(decoded.as_ref())) else { return Ok(()) };
        let data = encoded
            .entry(cid.as_str())
            .or_insert_with(|| format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(part.contents())));
        if used + data.len() <= budget {
            used += data.len();
            el.set_attribute("src", data)?;
        }
        Ok(())
    }));
    let mut out = Vec::with_capacity(html.len());
    let mut rewriter = HtmlRewriter::new(settings, |chunk: &[u8]| out.extend_from_slice(chunk));
    if rewriter.write(html.as_bytes()).and_then(|()| rewriter.end()).is_err() {
        return html.to_string();
    }
    String::from_utf8(out).unwrap_or_else(|_| html.to_string())
}

/// Имя файла без путей, управляющих символов и скрытых «точечных» имён.
pub fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base.chars().filter(|c| !c.is_control()).collect::<String>().trim().trim_start_matches('.').to_string();
    let cleaned: String = cleaned.chars().take(200).collect();
    if cleaned.is_empty() { "вложение".into() } else { cleaned }
}

const EXECUTABLE: &[&str] = &[
    "exe", "msi", "bat", "cmd", "com", "scr", "pif", "vbs", "vbe", "js", "jse", "wsf", "ps1", "jar", "sh", "bash",
    "run", "bin", "appimage", "desktop", "deb", "rpm", "apk", "dmg", "pkg", "lnk", "hta", "cpl", "reg", "py", "pl",
];

/// Исполняемое расширение — в том числе за «маскировкой» вида `счёт.pdf.exe` или `doc.pdf      .exe`.
pub fn is_dangerous(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.rsplit_once('.').is_some_and(|(_, ext)| EXECUTABLE.contains(&ext.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_cannot_escape_the_chosen_folder() {
        assert_eq!(safe_name("../../.bashrc"), "bashrc");
        assert_eq!(safe_name("C:\\Windows\\evil.dll"), "evil.dll");
        assert_eq!(safe_name("a\u{0}b\nc.txt"), "abc.txt");
        assert_eq!(safe_name(""), "вложение");
    }

    #[test]
    fn flags_executables_and_double_extensions() {
        for bad in ["setup.exe", "run.sh", "x.AppImage", "счёт.pdf.exe", "invoice.pdf.scr", "notes.txt.js", "game.desktop"] {
            assert!(is_dangerous(bad), "{bad}");
        }
        for ok in ["report.pdf", "photo.jpg", "archive.tar.gz", "data.json", "Отчёт.v2.docx"] {
            assert!(!is_dangerous(ok), "{ok}");
        }
    }

    #[test]
    fn lists_files_and_inlines_cid_images() {
        let raw = b"From: a@b.example\r\nContent-Type: multipart/mixed; boundary=X\r\n\r\n--X\r\nContent-Type: multipart/related; boundary=Y\r\n\r\n--Y\r\nContent-Type: text/html\r\n\r\n<img src=\"cid:pic1\">\r\n--Y\r\nContent-Type: image/png\r\nContent-ID: <pic1>\r\nContent-Disposition: inline\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--Y--\r\n--X\r\nContent-Type: application/pdf; name=\"../report.pdf\"\r\nContent-Disposition: attachment; filename=\"../report.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0=\r\n--X--\r\n";
        let parsed = mail_parser::MessageParser::default().parse(&raw[..]).unwrap();
        let files = describe(&parsed);
        assert_eq!(files.len(), 1, "встроенная картинка — не вложение");
        assert_eq!(files[0].name, "report.pdf");
        let html = inline_cid_images(&parsed.body_html(0).unwrap(), &parsed);
        assert!(html.contains("data:image/png;base64,"), "{html}");
    }

    #[test]
    fn cid_images_are_inlined_exactly_and_within_budget() {
        let raw = b"From: a@b.example\r\nContent-Type: multipart/related; boundary=Y\r\n\r\n--Y\r\nContent-Type: text/html\r\n\r\nx\r\n--Y\r\nContent-Type: image/png\r\nContent-ID: <pic1@host>\r\nContent-Disposition: inline\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--Y--\r\n";
        let parsed = mail_parser::MessageParser::default().parse(&raw[..]).unwrap();
        let data = "data:image/png;base64,iVBORw0KGgo=";

        let html = r#"<p>cid:pic1@host</p><img src="CID:pic1%40host"><img src="cid:pic1@host0">"#;
        let out = inline_cid_images(html, &parsed);
        assert_eq!(out.matches(data).count(), 1, "{out}");
        assert!(out.contains("<p>cid:pic1@host</p>") && out.contains("cid:pic1@host0"), "текст и чужие id не трогаем: {out}");

        let many = r#"<img src="cid:pic1@host">"#.repeat(10);
        let out = inline_cid_images_within(&many, &parsed, data.len() * 3);
        assert_eq!(out.matches(data).count(), 3, "сверх лимита картинки не разворачиваются");
    }
}
