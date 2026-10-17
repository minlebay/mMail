//! Командная строка mMail — по образцу Thunderbird, чтобы другие программы (и Claude Code)
//! могли открыть окно нового письма. Письмо только открывается, отправляет всегда человек.
//!
//!   mmail -compose "to='a@b.c,d@e.f',cc='…',bcc='…',subject='Тема',body='Текст',from='я@gmail.com',attachment='/путь/1.pdf,file:///путь/2.jpg'"
//!   mmail -compose "to='a@b.c',subject='Отчёт',message='/путь/текст.txt'"
//!   mmail mailto:a@b.c?subject=Тема&body=Текст
//!   mmail compose --to a@b.c --subject Тема --body Текст --attach /путь/файл
//!
//! Работающему mMail аргументы приходят через D-Bus (tauri-plugin-single-instance) вместе с
//! рабочей папкой вызывающего — от неё считаются относительные пути.

use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;

use crate::compose::{Prefill, MAX_ATTACHMENTS_BYTES};

/// Адресаты и тема — одна строка; текст — до 1 МБ.
const LINE_MAX: usize = 2000;
const BODY_MAX: usize = 1024 * 1024;

/// `None` — аргументов для письма нет (просто показать окно).
pub fn parse(argv: &[String], cwd: &Path) -> Result<Option<Prefill>, String> {
    let args = argv.get(1..).unwrap_or_default();
    let Some((pos, first)) = args.iter().enumerate().find(|(_, a)| is_compose(a) || a.starts_with("mailto:")) else {
        return Ok(None);
    };
    let mut prefill = Prefill { kind: "new".into(), ..Default::default() };
    if first.starts_with("mailto:") {
        mailto(&mut prefill, first)?;
    } else {
        let rest = &args[pos + 1..];
        match rest.first() {
            None => {}
            Some(spec) if spec.starts_with("mailto:") => mailto(&mut prefill, spec)?,
            Some(flag) if flag.starts_with("--") => flags(&mut prefill, rest, cwd)?,
            Some(spec) => thunderbird(&mut prefill, spec, cwd)?,
        }
    }
    Ok(Some(prefill))
}

fn is_compose(arg: &str) -> bool {
    matches!(arg, "-compose" | "--compose" | "compose")
}

/// Thunderbird: `key=value,key='значение, с запятыми',…`.
fn thunderbird(prefill: &mut Prefill, spec: &str, cwd: &Path) -> Result<(), String> {
    for (key, value) in split_spec(spec)? {
        match key.to_ascii_lowercase().as_str() {
            "to" => prefill.to = line("to", &value)?,
            "cc" => prefill.cc = line("cc", &value)?,
            "bcc" => prefill.bcc = line("bcc", &value)?,
            "subject" => prefill.subject = line("subject", &value)?,
            "from" => prefill.from = Some(line("from", &value)?),
            "body" => prefill.body = body(value)?,
            "message" => prefill.body = body(read_text(&resolve(&value, cwd)?)?)?,
            "attachment" => {
                for item in value.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    attach(prefill, item, cwd)?;
                }
            }
            // Письмо всегда открывается в редакторе с форматированием; текст вставляется как текст.
            "format" => {}
            _ => return Err(format!("неизвестный параметр -compose: {key}")),
        }
    }
    Ok(())
}

fn split_spec(spec: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    let mut chars = spec.chars().peekable();
    loop {
        while chars.next_if(|c| *c == ',' || c.is_whitespace()).is_some() {}
        if chars.peek().is_none() {
            return Ok(out);
        }
        let key: String = chars.by_ref().take_while(|c| *c != '=').collect();
        let key = key.trim().to_string();
        if key.is_empty() {
            return Err(format!("-compose: ожидалось «ключ=значение»: {spec}"));
        }
        let value = match chars.peek() {
            Some(&q) if q == '\'' || q == '"' => {
                chars.next();
                let mut value = String::new();
                loop {
                    match chars.next() {
                        Some(c) if c == q => break,
                        Some(c) => value.push(c),
                        None => return Err(format!("-compose: нет закрывающей кавычки у {key}")),
                    }
                }
                value
            }
            _ => chars.by_ref().take_while(|c| *c != ',').collect(),
        };
        out.push((key, value));
    }
}

/// mailto: по RFC 6068 — адресаты в пути, остальное в параметрах. `attach` и прочие
/// параметры не принимаются: ссылка с веб-страницы не должна прикладывать файлы с диска.
fn mailto(prefill: &mut Prefill, url: &str) -> Result<(), String> {
    let rest = &url["mailto:".len()..];
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let decode = |s: &str| percent_decode_str(s).decode_utf8().map(|c| c.into_owned()).map_err(|_| "mailto: не UTF-8".to_string());
    let mut to = vec![decode(path)?];
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = decode(value)?;
        match key.to_ascii_lowercase().as_str() {
            "to" => to.push(value),
            "cc" => prefill.cc = line("cc", &value)?,
            "bcc" => prefill.bcc = line("bcc", &value)?,
            "subject" => prefill.subject = line("subject", &value)?,
            "body" => prefill.body = body(value)?,
            _ => {}
        }
    }
    prefill.to = line("to", &to.into_iter().filter(|a| !a.trim().is_empty()).collect::<Vec<_>>().join(", "))?;
    Ok(())
}

/// `compose --to … --subject … --body … --attach …` (можно повторять --attach).
fn flags(prefill: &mut Prefill, args: &[String], cwd: &Path) -> Result<(), String> {
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        let value = rest.next().ok_or_else(|| format!("нет значения для {flag}"))?;
        match flag.as_str() {
            "--from" => prefill.from = Some(line(flag, value)?),
            "--to" => prefill.to = line(flag, value)?,
            "--cc" => prefill.cc = line(flag, value)?,
            "--bcc" => prefill.bcc = line(flag, value)?,
            "--subject" => prefill.subject = line(flag, value)?,
            "--body" => prefill.body = body(value.clone())?,
            "--attach" => attach(prefill, value, cwd)?,
            _ => return Err(format!("неизвестный параметр {flag}")),
        }
    }
    Ok(())
}

/// Однострочное поле: перевод строки в адресе или теме — попытка дописать заголовок письма.
fn line(name: &str, value: &str) -> Result<String, String> {
    if value.chars().count() > LINE_MAX || value.contains(['\r', '\n']) {
        return Err(format!("{name}: слишком длинная или многострочная строка"));
    }
    Ok(value.trim().to_string())
}

fn body(value: String) -> Result<String, String> {
    if value.len() > BODY_MAX {
        return Err("текст письма больше 1 МБ".into());
    }
    Ok(value.replace("\r\n", "\n"))
}

/// Путь или file:// URL; относительный — от рабочей папки вызывающего.
fn resolve(item: &str, cwd: &Path) -> Result<PathBuf, String> {
    let path = if item.starts_with("file:") {
        url::Url::parse(item).ok().and_then(|u| u.to_file_path().ok()).ok_or_else(|| format!("некорректный адрес файла: {item}"))?
    } else {
        PathBuf::from(item)
    };
    let path = if path.is_absolute() { path } else { cwd.join(path) };
    if !path.is_file() {
        return Err(format!("нет файла {}", path.display()));
    }
    Ok(path)
}

fn attach(prefill: &mut Prefill, item: &str, cwd: &Path) -> Result<(), String> {
    prefill.attach.push(resolve(item, cwd)?);
    let total: u64 = prefill.attach.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
    if total > MAX_ATTACHMENTS_BYTES {
        return Err("вложения больше 25 МБ".into());
    }
    Ok(())
}

fn read_text(path: &Path) -> Result<String, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > BODY_MAX as u64 {
        return Err("текст письма больше 1 МБ".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|_| format!("{}: не UTF-8", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(list: &[&str]) -> Result<Option<Prefill>, String> {
        let argv: Vec<String> = std::iter::once("mmail").chain(list.iter().copied()).map(String::from).collect();
        parse(&argv, &std::env::temp_dir())
    }

    fn temp_file(name: &str, text: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("mmail-cli-{}-{name}", std::process::id()));
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn no_compose_arguments() {
        assert!(run(&[]).unwrap().is_none());
        assert!(run(&["--some-gtk-flag"]).unwrap().is_none());
        let p = run(&["-compose"]).unwrap().unwrap();
        assert_eq!((p.to.as_str(), p.kind.as_str()), ("", "new"));
    }

    #[test]
    fn thunderbird_compose() {
        let a = temp_file("a.txt", "a");
        let b = temp_file("b c.txt", "b");
        let text = temp_file("body.txt", "строка 1\r\nстрока 2");
        let b_url = url::Url::from_file_path(&b).unwrap();
        let spec = format!(
            "to='x@y.z,w@y.z',cc=c@y.z,bcc='h@y.z',subject='Привет, мир',from=me@gmail.com,format=text,attachment='{},{}',message={}",
            a.display(),
            b_url,
            text.file_name().unwrap().to_str().unwrap(),
        );
        let p = run(&["-compose", &spec]).unwrap().unwrap();
        assert_eq!(p.to, "x@y.z,w@y.z");
        assert_eq!((p.cc.as_str(), p.bcc.as_str()), ("c@y.z", "h@y.z"));
        assert_eq!(p.subject, "Привет, мир");
        assert_eq!(p.from.as_deref(), Some("me@gmail.com"));
        assert_eq!(p.body, "строка 1\nстрока 2");
        assert_eq!(p.attach, vec![a.clone(), b.clone()]);
        let p = run(&["-compose", "body=\"Текст, с запятой\""]).unwrap().unwrap();
        assert_eq!(p.body, "Текст, с запятой");
        for f in [a, b, text] {
            std::fs::remove_file(f).unwrap();
        }
    }

    #[test]
    fn mailto_links() {
        let p = run(&["mailto:a@b.c?subject=%D0%9F%D1%80%D0%B8%D0%B2%D0%B5%D1%82+1&cc=c@d.e&body=a%0Ab&to=x@y.z"]).unwrap().unwrap();
        assert_eq!(p.to, "a@b.c, x@y.z");
        assert_eq!(p.subject, "Привет+1");
        assert_eq!((p.cc.as_str(), p.body.as_str()), ("c@d.e", "a\nb"));
        // Вложения ссылкой с сайта не прикладываются.
        let p = run(&["-compose", "mailto:a@b.c?attach=/etc/passwd&attachment=/etc/hosts"]).unwrap().unwrap();
        assert!(p.attach.is_empty());
    }

    #[test]
    fn flag_style() {
        let p = run(&["compose", "--from", "me@gmail.com", "--to", "a@b.c", "--bcc", "h@b.c", "--subject", "Тема", "--body", "x"]).unwrap().unwrap();
        assert_eq!((p.to.as_str(), p.bcc.as_str(), p.subject.as_str(), p.body.as_str()), ("a@b.c", "h@b.c", "Тема", "x"));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(run(&["compose", "--to"]).is_err());
        assert!(run(&["compose", "--cc2", "x"]).is_err());
        assert!(run(&["-compose", "subject='a\nBcc: x@y.z'"]).is_err());
        assert!(run(&["mailto:a@b.c?subject=a%0D%0ABcc:%20x@y.z"]).is_err());
        assert!(run(&["-compose", "to='a@b.c"]).is_err());
        assert!(run(&["-compose", "attachment=/nonexistent/file"]).is_err());
        assert!(run(&["-compose", "newsgroups=x"]).is_err());
    }
}
