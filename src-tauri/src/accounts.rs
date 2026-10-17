//! Почтовые ящики в KWallet (Secret Service): запись с атрибутами `service=mmail`,
//! `account=<адрес>` и паролем приложения в секрете. Пароль из KWallet читается
//! только на время входа на сервер и затирается из памяти (zeroize).

use std::collections::HashMap;

use secret_service::{EncryptionType, SecretService};
use zeroize::Zeroizing;

use crate::imap::Result;

pub struct Provider {
    pub imap_host: &'static str,
    /// Authserv-id серверов провайдера: верим только их Authentication-Results.
    pub trusted_authserv: &'static [&'static str],
}

pub const YANDEX: Provider = Provider { imap_host: "imap.yandex.com", trusted_authserv: &[".yandex.net", ".yandex.ru"] };
pub const GMAIL: Provider = Provider { imap_host: "imap.gmail.com", trusted_authserv: &["mx.google.com"] };

pub fn provider_for(email: &str) -> Option<&'static Provider> {
    let domain = email.rsplit_once('@')?.1.to_ascii_lowercase();
    match domain.as_str() {
        "gmail.com" | "googlemail.com" => Some(&GMAIL),
        "yandex.ru" | "yandex.com" | "ya.ru" | "yandex.by" | "yandex.kz" | "narod.ru" => Some(&YANDEX),
        _ => None,
    }
}

#[derive(Clone)]
pub struct Account {
    pub email: String,
    pub provider: &'static Provider,
}

impl Account {
    pub fn new(email: &str) -> Result<Self> {
        let email = email.trim().to_lowercase();
        let provider = provider_for(&email).ok_or("поддерживаются только ящики Яндекса и Gmail")?;
        Ok(Self { email, provider })
    }
}

const SERVICE: &str = "mmail";

async fn connect() -> Result<SecretService<'static>> {
    Ok(SecretService::connect(EncryptionType::Dh).await?)
}

/// Адреса всех ящиков (без паролей).
pub async fn list() -> Result<Vec<String>> {
    let ss = connect().await?;
    let found = ss.search_items(HashMap::from([("service", SERVICE)])).await?;
    let mut emails = Vec::new();
    for item in found.unlocked.iter().chain(found.locked.iter()) {
        if let Some(email) = item.get_attributes().await?.remove("account") {
            emails.push(email);
        }
    }
    emails.sort();
    emails.dedup();
    Ok(emails)
}

pub async fn password(email: &str) -> Result<Zeroizing<String>> {
    let ss = connect().await?;
    let found = ss.search_items(HashMap::from([("service", SERVICE), ("account", email)])).await?;
    let item = found.unlocked.into_iter().chain(found.locked).next().ok_or("пароль ящика не найден в KWallet")?;
    item.unlock().await?;
    let secret = Zeroizing::new(item.get_secret().await?);
    Ok(Zeroizing::new(String::from_utf8(secret.to_vec())?))
}

pub async fn save(account: &Account, password: &str) -> Result<()> {
    let ss = connect().await?;
    let collection = ss.get_default_collection().await?;
    collection.unlock().await?;
    collection
        .create_item(
            &format!("mMail: {}", account.email),
            HashMap::from([("service", SERVICE), ("account", account.email.as_str())]),
            password.as_bytes(),
            true,
            "text/plain",
        )
        .await?;
    Ok(())
}

pub async fn remove(email: &str) -> Result<()> {
    let ss = connect().await?;
    let found = ss.search_items(HashMap::from([("service", SERVICE), ("account", email)])).await?;
    for item in found.unlocked.iter().chain(found.locked.iter()) {
        item.delete().await?;
    }
    Ok(())
}
