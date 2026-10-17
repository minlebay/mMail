//! Google Календарь: список календарей и события через Calendar API v3.
//! События не кэшируются на диске — только запрашиваются на видимый период.

use std::time::Duration;

use chrono::{DateTime, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, State};

use crate::config::ConfigStore;
use crate::google;

const API: &str = "https://www.googleapis.com/calendar/v3";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarInfo {
    id: String,
    name: String,
    color: String,
    primary: bool,
    /// Можно создавать и менять события.
    writable: bool,
    /// Показывается в сетке (скрытые — в настройках mMail, не в Google).
    visible: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    id: String,
    calendar_id: String,
    title: String,
    /// RFC 3339 для событий со временем, `YYYY-MM-DD` — для событий на весь день.
    start: String,
    /// Конец не включается (у событий на день — следующий день, как в Google).
    end: String,
    all_day: bool,
    location: String,
    /// Описание как текст: Google хранит его с HTML, а это чужое содержимое.
    description: String,
    /// Повторяющееся: правка меняет только это повторение.
    recurring: bool,
}

/// Данные события из формы.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    title: String,
    all_day: bool,
    /// RFC 3339 или `YYYY-MM-DD` (весь день, конец не включается).
    start: String,
    end: String,
    #[serde(default)]
    location: String,
    #[serde(default)]
    description: String,
}

#[tauri::command]
pub async fn calendar_list(app: AppHandle, config: State<'_, ConfigStore>) -> Result<Vec<CalendarInfo>, String> {
    let hidden = config.get().hidden_calendars;
    let body = api(&app, "GET", &format!("{API}/users/me/calendarList?maxResults=250"), None).await?;
    let mut list: Vec<CalendarInfo> = body
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let id = c.get("id")?.as_str()?.to_string();
            let role = c.get("accessRole").and_then(Value::as_str).unwrap_or("reader");
            Some(CalendarInfo {
                name: c.get("summaryOverride").or_else(|| c.get("summary")).and_then(Value::as_str).unwrap_or(&id).to_string(),
                color: c.get("backgroundColor").and_then(Value::as_str).filter(|v| is_hex_color(v)).unwrap_or("#8b7bff").to_string(),
                primary: c.get("primary").and_then(Value::as_bool).unwrap_or(false),
                writable: matches!(role, "owner" | "writer"),
                visible: !hidden.contains(&id),
                id,
            })
        })
        .collect();
    list.sort_by(|a, b| b.primary.cmp(&a.primary).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(list)
}

#[tauri::command]
pub fn calendar_set_visible(config: State<'_, ConfigStore>, id: String, visible: bool) -> Result<(), String> {
    config.update(|c| {
        if visible {
            c.hidden_calendars.remove(&id);
        } else {
            c.hidden_calendars.insert(id);
        }
    })
}

/// События видимых календарей в промежутке `[from, to)` (RFC 3339).
#[tauri::command]
pub async fn calendar_events(app: AppHandle, config: State<'_, ConfigStore>, calendars: Vec<String>, from: String, to: String) -> Result<Vec<Event>, String> {
    DateTime::parse_from_rfc3339(&from).map_err(|_| "некорректное начало периода")?;
    DateTime::parse_from_rfc3339(&to).map_err(|_| "некорректный конец периода")?;
    let hidden = config.get().hidden_calendars;
    let mut events = Vec::new();
    for calendar in calendars.iter().filter(|c| !hidden.contains(*c)).take(50) {
        let mut page: Option<String> = None;
        loop {
            let mut url = url::Url::parse(&event_url(calendar, None)?).map_err(|e| e.to_string())?;
            url.query_pairs_mut()
                .append_pair("timeMin", &from)
                .append_pair("timeMax", &to)
                .append_pair("singleEvents", "true")
                .append_pair("orderBy", "startTime")
                .append_pair("maxResults", "2500");
            if let Some(token) = &page {
                url.query_pairs_mut().append_pair("pageToken", token);
            }
            let body = api(&app, "GET", url.as_str(), None).await?;
            events.extend(body.get("items").and_then(Value::as_array).into_iter().flatten().filter_map(|e| to_event(e, calendar)));
            page = body.get("nextPageToken").and_then(Value::as_str).map(str::to_string);
            if page.is_none() {
                break;
            }
        }
    }
    Ok(events)
}

/// Создать (`event_id: None`) или изменить событие.
#[tauri::command]
pub async fn calendar_save_event(app: AppHandle, calendar_id: String, event_id: Option<String>, draft: Draft) -> Result<(), String> {
    let body = event_body(&draft)?;
    match event_id {
        Some(id) => api(&app, "PATCH", &event_url(&calendar_id, Some(&id))?, Some(body)).await?,
        None => api(&app, "POST", &event_url(&calendar_id, None)?, Some(body)).await?,
    };
    Ok(())
}

#[tauri::command]
pub async fn calendar_delete_event(app: AppHandle, calendar_id: String, event_id: String) -> Result<(), String> {
    api(&app, "DELETE", &event_url(&calendar_id, Some(&event_id))?, None).await?;
    Ok(())
}

fn event_url(calendar_id: &str, event_id: Option<&str>) -> Result<String, String> {
    let mut url = url::Url::parse(API).map_err(|e| e.to_string())?;
    {
        let mut path = url.path_segments_mut().map_err(|_| "url")?;
        path.push("calendars").push(calendar_id).push("events");
        if let Some(id) = event_id {
            path.push(id);
        }
    }
    Ok(url.into())
}

fn event_body(draft: &Draft) -> Result<Value, String> {
    let title = draft.title.trim();
    if title.chars().count() > 1000 || draft.description.len() > 50_000 || draft.location.len() > 1000 {
        return Err("слишком длинное поле".into());
    }
    let (start, end) = if draft.all_day {
        let start = NaiveDate::parse_from_str(&draft.start, "%Y-%m-%d").map_err(|_| "некорректная дата начала")?;
        let end = NaiveDate::parse_from_str(&draft.end, "%Y-%m-%d").map_err(|_| "некорректная дата конца")?;
        if end <= start {
            return Err("конец раньше начала".into());
        }
        (json!({ "date": draft.start, "dateTime": null }), json!({ "date": draft.end, "dateTime": null }))
    } else {
        let start = DateTime::parse_from_rfc3339(&draft.start).map_err(|_| "некорректное время начала")?;
        let end = DateTime::parse_from_rfc3339(&draft.end).map_err(|_| "некорректное время конца")?;
        if end <= start {
            return Err("конец раньше начала".into());
        }
        (json!({ "dateTime": start.to_rfc3339(), "date": null }), json!({ "dateTime": end.to_rfc3339(), "date": null }))
    };
    Ok(json!({
        "summary": if title.is_empty() { "(без названия)" } else { title },
        "location": draft.location.trim(),
        "description": draft.description,
        "start": start,
        "end": end,
    }))
}

fn to_event(e: &Value, calendar: &str) -> Option<Event> {
    if e.get("status").and_then(Value::as_str) == Some("cancelled") {
        return None;
    }
    let (start, all_day) = time_of(e.get("start")?)?;
    let (end, _) = time_of(e.get("end")?)?;
    let description = e.get("description").and_then(Value::as_str).unwrap_or_default();
    Some(Event {
        id: e.get("id")?.as_str()?.to_string(),
        calendar_id: calendar.to_string(),
        title: e.get("summary").and_then(Value::as_str).unwrap_or("(без названия)").to_string(),
        start,
        end,
        all_day,
        location: e.get("location").and_then(Value::as_str).unwrap_or_default().to_string(),
        description: html_to_text(description),
        recurring: e.get("recurringEventId").is_some(),
    })
}

/// `{dateTime}` → RFC 3339 в местном поясе, `{date}` → дата (весь день).
fn time_of(v: &Value) -> Option<(String, bool)> {
    if let Some(date) = v.get("date").and_then(Value::as_str) {
        return Some((date.to_string(), true));
    }
    let at = DateTime::parse_from_rfc3339(v.get("dateTime")?.as_str()?).ok()?;
    Some((at.with_timezone(&Local).to_rfc3339(), false))
}

/// Описание события в Google — HTML от кого угодно (приглашения извне). Показываем только текст.
fn html_to_text(html: &str) -> String {
    if !html.contains('<') && !html.contains('&') {
        return html.to_string();
    }
    let clean = crate::sanitize::clean_html(html, false);
    mail_parser::decoders::html::html_to_text(&clean.html).trim().to_string()
}

fn is_hex_color(v: &str) -> bool {
    v.len() == 7 && v.starts_with('#') && v[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Запрос к Calendar API с токеном; на 401 — один раз с обновлённым токеном.
async fn api(app: &AppHandle, method: &'static str, url: &str, body: Option<Value>) -> Result<Value, String> {
    for attempt in 0..2 {
        let token = google::access_token(app).await?;
        let (url, body) = (url.to_string(), body.clone());
        let result = tauri::async_runtime::spawn_blocking(move || request(method, &url, &token, body)).await.map_err(|e| e.to_string())?;
        match result {
            Err(Unauthorized) if attempt == 0 => google::forget_access_token(app).await,
            Err(Unauthorized) => return Err("Google отклонил доступ — подключите календарь заново".into()),
            Ok(value) => return value,
        }
    }
    unreachable!()
}

struct Unauthorized;

fn request(method: &str, url: &str, token: &str, body: Option<Value>) -> Result<Result<Value, String>, Unauthorized> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .build()
        .into();
    let auth = format!("Bearer {token}");
    let sent = match (method, body) {
        ("GET", _) => agent.get(url).header("Authorization", &auth).call(),
        ("DELETE", _) => agent.delete(url).header("Authorization", &auth).call(),
        ("PATCH", Some(b)) => agent.patch(url).header("Authorization", &auth).header("Content-Type", "application/json").send(b.to_string()),
        ("POST", Some(b)) => agent.post(url).header("Authorization", &auth).header("Content-Type", "application/json").send(b.to_string()),
        _ => return Ok(Err("неподдерживаемый запрос".into())),
    };
    let mut response = match sent {
        Ok(r) => r,
        Err(e) => return Ok(Err(format!("нет связи с Google: {e}"))),
    };
    let status = response.status().as_u16();
    if status == 401 {
        return Err(Unauthorized);
    }
    let text = response.body_mut().read_to_string().unwrap_or_default();
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status >= 400 {
        let reason = value.pointer("/error/message").and_then(Value::as_str).unwrap_or("ошибка");
        return Ok(Err(format!("Google Календарь: {reason}")));
    }
    Ok(Ok(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_ids_are_escaped_in_urls() {
        let url = event_url("ru.russian#holiday@group.v.calendar.google.com", Some("abc/../x")).unwrap();
        assert!(url.starts_with("https://www.googleapis.com/calendar/v3/calendars/"));
        assert!(url.contains("%23holiday") && !url.contains("/../"), "{url}");
    }

    #[test]
    fn reads_events_and_drops_markup() {
        let v = json!({
            "id": "e1", "summary": "Встреча", "status": "confirmed",
            "start": {"dateTime": "2026-10-20T10:00:00+02:00"}, "end": {"dateTime": "2026-10-20T11:00:00+02:00"},
            "description": "<b>Привет</b><img src=\"https://t.example/p\"> <script>alert(1)</script>"
        });
        let e = to_event(&v, "me@gmail.com").unwrap();
        assert!(!e.all_day && e.description.contains("Привет") && !e.description.contains('<'), "{}", e.description);
        let all_day = json!({"id": "e2", "start": {"date": "2026-10-20"}, "end": {"date": "2026-10-21"}});
        assert!(to_event(&all_day, "c").unwrap().all_day);
        assert!(to_event(&json!({"id": "e3", "status": "cancelled", "start": {"date": "2026-10-20"}, "end": {"date": "2026-10-21"}}), "c").is_none());
    }

    #[test]
    fn drafts_are_validated() {
        let d = |all_day, start: &str, end: &str| Draft { title: "x".into(), all_day, start: start.into(), end: end.into(), location: String::new(), description: String::new() };
        assert!(event_body(&d(false, "2026-10-20T10:00:00+02:00", "2026-10-20T11:00:00+02:00")).is_ok());
        assert!(event_body(&d(false, "2026-10-20T11:00:00+02:00", "2026-10-20T10:00:00+02:00")).is_err());
        assert!(event_body(&d(true, "2026-10-20", "2026-10-21")).is_ok());
        assert!(event_body(&d(true, "2026-10-20", "nonsense")).is_err());
    }
}
