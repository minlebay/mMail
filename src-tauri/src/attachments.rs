//! Вложения входящих писем: что показывать, встроенные картинки (cid:), безопасные имена файлов.

use base64::Engine;
use mail_parser::{Message, MessagePart, MimeHeaders};
use serde::Serialize;

/// Встроенные картинки крупнее этого не встраиваем в просмотр.
const MAX_INLINE_IMAGE: usize = 5 * 1024 * 1024;

#[derive(Clone, Serialize)]
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
    let mut html = html.to_string();
    for part in parsed.attachments().filter(|p| is_inline_image(p)) {
        let (Some(cid), mime) = (part.content_id(), mime_of(part)) else { continue };
        if mime.contains("svg") || part.contents().len() > MAX_INLINE_IMAGE {
            continue;
        }
        let cid = cid.trim_matches(['<', '>']);
        let data = base64::engine::general_purpose::STANDARD.encode(part.contents());
        html = html.replace(&format!("cid:{cid}"), &format!("data:{mime};base64,{data}"));
    }
    html
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
}
