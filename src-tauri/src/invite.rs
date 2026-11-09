//! Приглашения на встречу в письмах: разбор `text/calendar` (iCalendar) для кнопки
//! «Добавить в календарь». Разбираем только то, что нужно форме события: название, время,
//! место, описание и правило повторения. Вложенный календарь — чужой файл, поэтому поля
//! ограничены по длине и проходят через [`display_safe`]/[`console_safe`].

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use mail_parser::{Message, MessageParser, MessagePart, MimeHeaders};
use serde::Serialize;

use crate::text::{console_safe, display_safe};

/// Больше мегабайта календарь не разбираем: приглашению хватает нескольких килобайт.
const MAX_ICS_BYTES: usize = 1024 * 1024;
const MAX_TITLE: usize = 1000;
const MAX_LOCATION: usize = 1000;
const MAX_DESCRIPTION: usize = 50_000;
const MAX_RECURRENCE: usize = 100;
const MAX_RECURRENCE_LINE: usize = 2000;

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Invite {
    pub title: String,
    pub all_day: bool,
    /// RFC 3339 для события со временем, `YYYY-MM-DD` — для события на весь день.
    pub start: String,
    /// Конец не включается (у события на день — следующий день, как в Google).
    pub end: String,
    pub location: String,
    pub description: String,
    /// Строки RRULE/RDATE/EXDATE для Google Календаря.
    pub recurrence: Vec<String>,
    pub cancelled: bool,
}

/// Находит первое приглашение в исходнике письма. `None` — календарного вложения нет
/// или это ответ участника (`METHOD:REPLY`), а не приглашение.
pub fn find(raw: &[u8]) -> Option<Invite> {
    let parsed = MessageParser::default().parse(raw)?;
    find_in_message(&parsed)
}

fn find_in_message(parsed: &Message) -> Option<Invite> {
    // Не только attachments(): HTML-only приглашение Outlook иногда классифицируется
    // как текстовое тело, а не как вложение — календарь надо найти в любом случае.
    parsed
        .parts
        .iter()
        .filter(|part| is_calendar(part) && part.contents().len() <= MAX_ICS_BYTES)
        .find_map(|part| {
            let text = match part.text_contents() {
                Some(text) => text.to_string(),
                None => String::from_utf8_lossy(part.contents()).into_owned(),
            };
            parse(&text)
        })
}

fn is_calendar(part: &MessagePart) -> bool {
    let mime = part
        .content_type()
        .map(|ct| format!("{}/{}", ct.ctype(), ct.subtype().unwrap_or_default()))
        .unwrap_or_default()
        .to_ascii_lowercase();
    mime == "text/calendar"
        || mime == "application/ics"
        || part
            .attachment_name()
            .is_some_and(|name| name.to_ascii_lowercase().ends_with(".ics"))
}

fn parse(text: &str) -> Option<Invite> {
    let props: Vec<Prop> = unfold(text)
        .iter()
        .filter_map(|line| Prop::parse(line))
        .collect();
    let method = props
        .iter()
        .find(|p| p.name == "METHOD")
        .map(|p| p.value.trim().to_ascii_uppercase())
        .unwrap_or_default();
    // Ответ участника («приду / не приду») — не приглашение, кнопки ему не нужны.
    if method == "REPLY" {
        return None;
    }

    let begin = props
        .iter()
        .position(|p| p.name == "BEGIN" && p.value.eq_ignore_ascii_case("VEVENT"))?;
    let end = props
        .iter()
        .enumerate()
        .skip(begin + 1)
        .find(|(_, p)| p.name == "END" && p.value.eq_ignore_ascii_case("VEVENT"))
        .map(|(i, _)| i)
        .unwrap_or(props.len());
    let event = &props[begin + 1..end];
    let get = |name: &str| event.iter().find(|p| p.name == name);

    let start = get("DTSTART")?;
    let (all_day, start, end) = times(start, get("DTEND"), get("DURATION"))?;
    let cancelled = method == "CANCEL"
        || get("STATUS").is_some_and(|p| p.value.trim().eq_ignore_ascii_case("CANCELLED"));

    let text = |name: &str, limit: usize| {
        get(name)
            .map(|p| unescape(&p.value))
            .map(|s| display_safe(&s).into_owned())
            .unwrap_or_default()
            .chars()
            .take(limit)
            .collect()
    };
    let description = get("DESCRIPTION")
        .map(|p| console_safe(&unescape(&p.value)).into_owned())
        .unwrap_or_default()
        .chars()
        .take(MAX_DESCRIPTION)
        .collect();
    let recurrence = event
        .iter()
        .filter(|p| matches!(p.name.as_str(), "RRULE" | "RDATE" | "EXDATE" | "EXRULE"))
        .filter(|p| !p.raw.is_empty() && p.raw.len() <= MAX_RECURRENCE_LINE)
        .map(|p| p.raw.clone())
        .take(MAX_RECURRENCE)
        .collect();

    Some(Invite {
        title: text("SUMMARY", MAX_TITLE),
        all_day,
        start,
        end,
        location: text("LOCATION", MAX_LOCATION),
        description,
        recurrence,
        cancelled,
    })
}

/// Строки iCalendar можно переносить: продолжение начинается с пробела или табуляции.
fn unfold(text: &str) -> Vec<String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines: Vec<String> = Vec::new();
    for line in text.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(last) = lines.last_mut() {
                last.push_str(&line[1..]);
            }
        } else {
            lines.push(line.to_string());
        }
    }
    lines
}

#[derive(Debug)]
struct Prop {
    name: String,
    params: Vec<(String, String)>,
    value: String,
    /// Строка без переносов — для RRULE/RDATE/EXDATE, которые уходят в Google как есть.
    raw: String,
}

impl Prop {
    fn parse(line: &str) -> Option<Self> {
        // Двоеточие внутри кавычек — часть параметра (например, TZID="UTC+03:00"), а не разделитель.
        let mut quoted = false;
        let mut colon = None;
        for (i, c) in line.char_indices() {
            match c {
                '"' => quoted = !quoted,
                ':' if !quoted => {
                    colon = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let (head, value) = line.split_at(colon?);
        let mut parts = head.split(';');
        let name = parts
            .next()?
            .trim()
            .rsplit('.')
            .next()?
            .to_ascii_uppercase();
        if name.is_empty() {
            return None;
        }
        let params = parts
            .filter_map(|part| {
                let (key, value) = part.split_once('=')?;
                Some((
                    key.trim().to_ascii_uppercase(),
                    value.trim().trim_matches('"').to_string(),
                ))
            })
            .collect();
        Some(Self {
            name,
            params,
            value: value[1..].to_string(),
            raw: line.to_string(),
        })
    }

    fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// `text/calendar` разворачивает escape-последовательности RFC 5545: `\n`, `\,`, `\;`, `\\`.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') | Some('N') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(',') => out.push(','),
            Some(';') => out.push(';'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn is_date_value(prop: &Prop) -> bool {
    prop.param("VALUE")
        .is_some_and(|v| v.eq_ignore_ascii_case("DATE"))
        || (prop.value.len() == 8 && prop.value.bytes().all(|b| b.is_ascii_digit()))
}

fn parse_date(prop: &Prop) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(prop.value.trim(), "%Y%m%d").ok()
}

fn parse_datetime(prop: &Prop) -> Option<DateTime<Local>> {
    let value = prop.value.trim();
    let utc = value.ends_with('Z') || value.ends_with('z');
    let core = if utc {
        &value[..value.len() - 1]
    } else {
        value
    };
    let naive = NaiveDateTime::parse_from_str(core, "%Y%m%dT%H%M%S")
        .or_else(|_| NaiveDateTime::parse_from_str(core, "%Y%m%dT%H%M"))
        .ok()?;
    let utc = utc
        || prop
            .param("TZID")
            .is_some_and(|tz| tz.eq_ignore_ascii_case("UTC") || tz.eq_ignore_ascii_case("GMT"));
    if utc {
        return Some(Utc.from_utc_datetime(&naive).with_timezone(&Local));
    }
    // TZID без базы часовых поясов трактуем как местное «настенное» время: для приглашений
    // из российских календарей (Europe/Moscow) это совпадает с системным поясом.
    Local
        .from_local_datetime(&naive)
        .earliest()
        .or_else(|| Local.from_local_datetime(&naive).latest())
        // Перевод часов вперёд: такого местного времени не существует, берём UTC.
        .or_else(|| Some(Utc.from_utc_datetime(&naive).with_timezone(&Local)))
}

fn times(
    start: &Prop,
    end: Option<&Prop>,
    duration: Option<&Prop>,
) -> Option<(bool, String, String)> {
    if is_date_value(start) {
        let start = parse_date(start)?;
        let end = end
            .and_then(parse_date)
            .or_else(|| {
                duration
                    .and_then(|p| parse_duration(&p.value))
                    .and_then(|d| start.checked_add_signed(d))
            })
            .unwrap_or_else(|| start.checked_add_signed(Duration::days(1)).unwrap_or(start));
        let end = if end <= start {
            start.checked_add_signed(Duration::days(1))?
        } else {
            end
        };
        return Some((
            true,
            start.format("%Y-%m-%d").to_string(),
            end.format("%Y-%m-%d").to_string(),
        ));
    }

    let start = parse_datetime(start)?;
    let end = end
        .and_then(parse_datetime)
        .or_else(|| {
            duration
                .and_then(|p| parse_duration(&p.value))
                .map(|d| start + d)
        })
        .unwrap_or_else(|| start + Duration::hours(1));
    let end = if end <= start {
        start + Duration::hours(1)
    } else {
        end
    };
    Some((false, start.to_rfc3339(), end.to_rfc3339()))
}

/// Длительность RFC 5545: `PT1H`, `PT30M`, `P1D`, `P1W`.
fn parse_duration(value: &str) -> Option<Duration> {
    let value = value.trim();
    let mut chars = value.strip_prefix('P')?.chars().peekable();
    let mut days = 0i64;
    let mut hours = 0i64;
    let mut minutes = 0i64;
    let mut seconds = 0i64;
    let mut number = String::new();
    let mut in_time = false;
    while let Some(c) = chars.next() {
        match c {
            'T' => in_time = true,
            '0'..='9' => number.push(c),
            'W' => {
                days += number.parse::<i64>().ok()? * 7;
                number.clear();
            }
            'D' => {
                days += number.parse::<i64>().ok()?;
                number.clear();
            }
            'H' if in_time => {
                hours = number.parse().ok()?;
                number.clear();
            }
            'M' if in_time => {
                minutes = number.parse().ok()?;
                number.clear();
            }
            'S' if in_time => {
                seconds = number.parse().ok()?;
                number.clear();
            }
            _ => return None,
        }
    }
    Some(
        Duration::days(days)
            + Duration::hours(hours)
            + Duration::minutes(minutes)
            + Duration::seconds(seconds),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const INVITE: &str = concat!(
        "From: a@example.com\r\n",
        "Content-Type: multipart/mixed; boundary=X\r\n",
        "\r\n",
        "--X\r\n",
        "Content-Type: text/plain\r\n",
        "\r\n",
        "Приглашаю на встречу.\r\n",
        "--X\r\n",
        "Content-Type: text/calendar; method=REQUEST; charset=UTF-8\r\n",
        "Content-Disposition: attachment; filename=\"invite.ics\"\r\n",
        "\r\n",
        "BEGIN:VCALENDAR\r\n",
        "VERSION:2.0\r\n",
        "METHOD:REQUEST\r\n",
        "BEGIN:VEVENT\r\n",
        "UID:abc@example.com\r\n",
        "SUMMARY:Встреча по проекту\r\n",
        "DTSTART:20261110T100000Z\r\n",
        "DTEND:20261110T110000Z\r\n",
        "LOCATION:Онлайн\r\n",
        "DESCRIPTION:Обсудим план\\nСсылка: https://meet.example.com/x\r\n",
        "RRULE:FREQ=WEEKLY;BYDAY=MO\r\n",
        "END:VEVENT\r\n",
        "END:VCALENDAR\r\n",
        "--X--\r\n",
    );

    #[test]
    fn finds_calendar_attachment_and_reads_event() {
        let invite = find(INVITE.as_bytes()).unwrap();
        assert_eq!(invite.title, "Встреча по проекту");
        assert_eq!(invite.location, "Онлайн");
        assert_eq!(
            invite.description,
            "Обсудим план\nСсылка: https://meet.example.com/x"
        );
        assert_eq!(invite.recurrence, ["RRULE:FREQ=WEEKLY;BYDAY=MO"]);
        assert!(!invite.all_day && !invite.cancelled);
        let start = DateTime::parse_from_rfc3339(&invite.start).unwrap();
        assert_eq!(
            start.with_timezone(&Utc),
            "2026-11-10T10:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
    }

    #[test]
    fn reads_all_day_and_tzid_events() {
        let all_day = concat!(
            "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\n",
            "SUMMARY:Отпуск\r\nDTSTART;VALUE=DATE:20261110\r\nDTEND;VALUE=DATE:20261112\r\n",
            "END:VEVENT\r\nEND:VCALENDAR\r\n"
        );
        let invite = parse(all_day).unwrap();
        assert!(invite.all_day);
        assert_eq!(
            (invite.start.as_str(), invite.end.as_str()),
            ("2026-11-10", "2026-11-12")
        );

        let tzid = concat!(
            "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\n",
            "SUMMARY:Созвон\r\nDTSTART;TZID=Europe/Moscow:20261110T100000\r\n",
            "DURATION:PT30M\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        );
        let invite = parse(tzid).unwrap();
        assert!(
            invite.start.starts_with("2026-11-10T10:00:00"),
            "{}",
            invite.start
        );
        assert!(
            invite.end.starts_with("2026-11-10T10:30:00"),
            "{}",
            invite.end
        );
    }

    #[test]
    fn handles_folded_lines_escapes_and_cancellations() {
        let folded = concat!(
            "BEGIN:VCALENDAR\r\nMETHOD:CANCEL\r\nBEGIN:VEVENT\r\n",
            "SUMMARY:Длинное имя встречи, которое перенесли \r\n",
            " на вторую строку\r\n",
            "DTSTART:20261110T100000Z\r\nSTATUS:CANCELLED\r\n",
            "DESCRIPTION:Строка 1\\nСтрока 2\\, продолжение\r\n",
            "END:VEVENT\r\nEND:VCALENDAR\r\n"
        );
        let invite = parse(folded).unwrap();
        assert!(invite.cancelled);
        assert_eq!(
            invite.title,
            "Длинное имя встречи, которое перенесли на вторую строку"
        );
        assert_eq!(invite.description, "Строка 1\nСтрока 2, продолжение");
    }

    #[test]
    fn finds_inline_calendar_in_alternative_body() {
        let raw = concat!(
            "From: a@example.com\r\nContent-Type: multipart/alternative; boundary=X\r\n\r\n",
            "--X\r\nContent-Type: text/html\r\n\r\n<p>Приглашение</p>\r\n",
            "--X\r\nContent-Type: text/calendar; method=REQUEST\r\n\r\n",
            "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nSUMMARY:Созвон\r\n",
            "DTSTART:20261110T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
            "--X--\r\n"
        );
        assert_eq!(find(raw.as_bytes()).unwrap().title, "Созвон");
    }

    #[test]
    fn ignores_replies_and_other_attachments() {
        let reply = concat!("BEGIN:VCALENDAR\r\nMETHOD:REPLY\r\nBEGIN:VEVENT\r\nSUMMARY:Ответ\r\nDTSTART:20261110T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n");
        assert!(parse(reply).is_none());

        let plain = concat!(
            "From: a@example.com\r\nContent-Type: text/plain\r\n\r\n",
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nSUMMARY:x\r\nDTSTART:20261110T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        );
        assert!(find(plain.as_bytes()).is_none());
    }

    #[test]
    fn rejects_oversized_and_control_text() {
        let dirty = concat!(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\n",
            "SUMMARY:Встреча\u{202e} evil\r\n",
            "DTSTART:20261110T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        );
        let invite = parse(dirty).unwrap();
        assert!(!invite.title.contains('\u{202e}'), "{:?}", invite.title);

        let giant = format!("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nSUMMARY:{}\r\nDTSTART:20261110T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n", "x".repeat(MAX_TITLE + 100));
        assert_eq!(parse(&giant).unwrap().title.chars().count(), MAX_TITLE);
    }
}
