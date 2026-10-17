//! Настройки приложения в `~/.config/dev.minlebay.mmail/config.json`.
//! Секретов здесь нет: пароли живут только в KWallet.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    /// Цветовая тема UI (id из src/lib/themes.js).
    pub theme: String,
    /// Непрозрачность фона окна, 0.15–0.85; `None` — по умолчанию темы.
    pub opacity: Option<f32>,
    /// Масштаб интерфейса по умолчанию, 0.5–2.0; `None` — 100%.
    pub zoom: Option<f64>,
    /// Средний клик по заголовку сворачивает окно; `None` — включено.
    pub middle_click_minimize: Option<bool>,
    /// Отправители, у которых внешнее содержимое показывается сразу (адреса в нижнем регистре).
    pub trusted_senders: BTreeSet<String>,
}

pub struct ConfigStore {
    path: PathBuf,
    config: Mutex<Config>,
}

impl ConfigStore {
    pub fn load(dir: PathBuf) -> Self {
        let path = dir.join("config.json");
        let config = fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { path, config: Mutex::new(config) }
    }

    pub fn get(&self) -> Config {
        self.config.lock().unwrap().clone()
    }

    pub fn update(&self, change: impl FnOnce(&mut Config)) -> Result<(), String> {
        let mut config = self.config.lock().unwrap();
        change(&mut config);
        self.save(&config).map_err(|e| format!("не удалось сохранить настройки: {e}"))
    }

    /// Запись через временный файл и rename: файл не бывает наполовину записан.
    fn save(&self, config: &Config) -> std::io::Result<()> {
        let dir = self.path.parent().expect("config path has a parent");
        fs::create_dir_all(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        let tmp = self.path.with_extension("json.tmp");
        let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        file.write_all(&serde_json::to_vec_pretty(config)?)?;
        file.sync_all()?;
        fs::rename(tmp, &self.path)
    }
}
