//! Вход в Google (OAuth 2.0 для настольных приложений, RFC 8252): страница входа открывается
//! в браузере, ответ принимает одноразовый приёмник на 127.0.0.1 (случайный порт), обмен кода —
//! с PKCE (RFC 7636) и проверкой `state`. Ключ продления (refresh token) и данные клиента
//! хранятся только в KWallet, токен доступа — только в памяти.

use std::time::{Duration, Instant};

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use zeroize::Zeroizing;

use crate::accounts;
use crate::config::ConfigStore;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
/// Календари (чтение и запись) и адрес аккаунта — чтобы показать, кто подключён.
const SCOPE: &str = "openid email https://www.googleapis.com/auth/calendar";
/// Сколько ждать, пока человек войдёт в браузере.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Serialize, Deserialize)]
struct Client {
    client_id: String,
    client_secret: String,
}

struct AccessToken {
    value: Zeroizing<String>,
    expires: Instant,
}

/// Токен доступа в памяти; обновляется по ключу продления из KWallet.
#[derive(Default)]
pub struct Google {
    token: tokio::sync::Mutex<Option<AccessToken>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleStatus {
    /// Адрес подключённого аккаунта.
    account: Option<String>,
}

#[tauri::command]
pub fn google_status(config: State<'_, ConfigStore>) -> GoogleStatus {
    GoogleStatus { account: config.get().google_account }
}

/// Подключение: сохранить клиента (из Google Cloud Console), войти в браузере, получить ключ продления.
#[tauri::command]
pub async fn google_connect(app: AppHandle, client_id: String, client_secret: String) -> Result<String, String> {
    let client = Client { client_id: client_id.trim().to_string(), client_secret: client_secret.trim().to_string() };
    if !client.client_id.ends_with(".apps.googleusercontent.com") || client.client_id.len() > 200 {
        return Err("Идентификатор клиента должен оканчиваться на .apps.googleusercontent.com".into());
    }
    if client.client_secret.is_empty() || client.client_secret.len() > 200 || client.client_secret.chars().any(char::is_whitespace) {
        return Err("Некорректный секрет клиента".into());
    }
    let tokens = authorize(&client).await?;
    let refresh = tokens.refresh_token.ok_or("Google не выдал ключ продления — попробуйте ещё раз")?;
    let email = tokens.id_token.as_deref().and_then(email_from_id_token).unwrap_or_else(|| "Google".into());

    let client_json = Zeroizing::new(serde_json::to_vec(&client).map_err(|e| e.to_string())?);
    accounts::remove_google_secrets().await.map_err(|e| e.to_string())?;
    accounts::set_google_secret("client", "mMail: клиент Google OAuth", &client_json).await.map_err(|e| e.to_string())?;
    accounts::set_google_secret("refresh", &format!("mMail: Google Календарь ({email})"), refresh.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    *app.state::<Google>().token.lock().await =
        Some(AccessToken { value: Zeroizing::new(tokens.access_token), expires: Instant::now() + Duration::from_secs(tokens.expires_in.saturating_sub(60)) });
    app.state::<ConfigStore>().update(|c| c.google_account = Some(email.clone()))?;
    let _ = app.emit("google-changed", ());
    Ok(email)
}

/// Отключение: отзываем доступ у Google и стираем всё своё.
#[tauri::command]
pub async fn google_disconnect(app: AppHandle) -> Result<(), String> {
    if let Ok(Some(refresh)) = accounts::google_secret("refresh").await {
        let token = String::from_utf8_lossy(&refresh).into_owned();
        // Отзыв — по возможности: без сети всё равно стираем локальные данные.
        let _ = tauri::async_runtime::spawn_blocking(move || post_form(REVOKE_URL, &[("token", &token)])).await;
    }
    accounts::remove_google_secrets().await.map_err(|e| e.to_string())?;
    *app.state::<Google>().token.lock().await = None;
    app.state::<ConfigStore>().update(|c| c.google_account = None)?;
    let _ = app.emit("google-changed", ());
    Ok(())
}

/// Действующий токен доступа (обновляется по ключу продления, когда истёк).
pub async fn access_token(app: &AppHandle) -> Result<Zeroizing<String>, String> {
    let google = app.state::<Google>();
    let mut cached = google.token.lock().await;
    if let Some(token) = cached.as_ref().filter(|t| t.expires > Instant::now()) {
        return Ok(token.value.clone());
    }
    let refresh = accounts::google_secret("refresh").await.map_err(|e| e.to_string())?.ok_or("Google Календарь не подключён")?;
    let client_json = accounts::google_secret("client").await.map_err(|e| e.to_string())?.ok_or("Google Календарь не подключён")?;
    let client: Client = serde_json::from_slice(&client_json).map_err(|e| e.to_string())?;
    let refresh = Zeroizing::new(String::from_utf8_lossy(&refresh).into_owned());
    let body = tauri::async_runtime::spawn_blocking(move || {
        post_form(
            TOKEN_URL,
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh.as_str()),
                ("client_id", &client.client_id),
                ("client_secret", &client.client_secret),
            ],
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    let tokens: TokenResponse = serde_json::from_value(body).map_err(|_| "Google отказал в обновлении доступа — подключите календарь заново")?;
    let token = AccessToken { value: Zeroizing::new(tokens.access_token), expires: Instant::now() + Duration::from_secs(tokens.expires_in.saturating_sub(60)) };
    let value = token.value.clone();
    *cached = Some(token);
    Ok(value)
}

/// Токен устарел раньше срока (Google ответил 401) — забыть его.
pub async fn forget_access_token(app: &AppHandle) {
    *app.state::<Google>().token.lock().await = None;
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
    refresh_token: Option<String>,
    id_token: Option<String>,
}

async fn authorize(client: &Client) -> Result<TokenResponse, String> {
    let verifier = random_token(32)?;
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_token(16)?;

    // Только петлевой адрес: ответ Google не может прийти из сети.
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| e.to_string())?;
    let redirect = format!("http://127.0.0.1:{}", listener.local_addr().map_err(|e| e.to_string())?.port());
    let mut url = url::Url::parse(AUTH_URL).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("client_id", &client.client_id)
        .append_pair("redirect_uri", &redirect)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPE)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent");
    std::process::Command::new("xdg-open").arg(url.as_str()).spawn().map_err(|e| format!("не удалось открыть браузер: {e}"))?;

    let code = tokio::time::timeout(LOGIN_TIMEOUT, receive_code(&listener, &state))
        .await
        .map_err(|_| "Вход не завершён за 5 минут")??;
    let (client_id, client_secret) = (client.client_id.clone(), client.client_secret.clone());
    let body = tauri::async_runtime::spawn_blocking(move || {
        post_form(
            TOKEN_URL,
            &[
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("redirect_uri", &redirect),
                ("client_id", &client_id),
                ("client_secret", &client_secret),
                ("code_verifier", &verifier),
            ],
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    serde_json::from_value(body).map_err(|_| "Google не выдал токен".to_string())
}

/// Ждём перенаправления браузера с `?code=…&state=…`. Прочие запросы (favicon) отклоняем.
async fn receive_code(listener: &TcpListener, state: &str) -> Result<Zeroizing<String>, String> {
    loop {
        let (mut stream, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 8192];
        let mut len = 0;
        while len < buf.len() {
            let n = stream.read(&mut buf[len..]).await.map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            len += n;
            if buf[..len].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let request = String::from_utf8_lossy(&buf[..len]).into_owned();
        let path = request.lines().next().and_then(|line| line.split_whitespace().nth(1)).unwrap_or("/");
        let Ok(url) = url::Url::parse(&format!("http://127.0.0.1{path}")) else { continue };
        let query: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
        if query.get("state").map(String::as_str) != Some(state) {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            continue;
        }
        let (title, result) = match (query.get("code"), query.get("error")) {
            (Some(code), _) => ("Вход выполнен — вкладку можно закрыть и вернуться в mMail.", Ok(Zeroizing::new(code.clone()))),
            (_, Some(error)) => ("Вход отменён — можно вернуться в mMail.", Err(format!("Google: {error}"))),
            _ => ("Неожиданный ответ — попробуйте ещё раз в mMail.", Err("Google не прислал код".into())),
        };
        let page = format!("<!doctype html><meta charset=utf-8><title>mMail</title><body style=\"font:16px sans-serif;padding:40px\">{title}</body>");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{page}",
            page.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
        return result;
    }
}

fn random_token(bytes: usize) -> Result<String, String> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| e.to_string())?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf))
}

/// Адрес из id_token: он получен напрямую от Google по TLS в ответ на наш обмен кода,
/// поэтому подпись не проверяем (OpenID Connect Core, 3.1.3.7).
fn email_from_id_token(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value.get("email")?.as_str().map(str::to_string)
}

/// POST формы к Google; тело ответа — JSON (и при ошибке тоже: в нём описание).
pub(crate) fn post_form(url: &str, form: &[(&str, &str)]) -> Result<serde_json::Value, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut response = agent.post(url).send_form(form.iter().copied()).map_err(|e| format!("нет связи с Google: {e}"))?;
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
    if status >= 400 {
        let reason = value.get("error_description").or_else(|| value.get("error")).and_then(|v| v.as_str()).unwrap_or("ошибка");
        return Err(format!("Google: {reason}"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_email_from_id_token() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"email":"me@gmail.com","sub":"1"}"#);
        assert_eq!(email_from_id_token(&format!("h.{payload}.s")).as_deref(), Some("me@gmail.com"));
        assert_eq!(email_from_id_token("garbage"), None);
    }

    #[tokio::test]
    async fn redirect_needs_the_right_state() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let browser = tokio::spawn(async move {
            for path in ["/favicon.ico", "/?state=forged&code=evil", "/?state=good&code=abc"] {
                let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).await.unwrap();
                let mut out = Vec::new();
                let _ = s.read_to_end(&mut out).await;
            }
        });
        let code = receive_code(&listener, "good").await.unwrap();
        assert_eq!(code.as_str(), "abc", "подделанный state не принимается");
        browser.await.unwrap();
    }
}
