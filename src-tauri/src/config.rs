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
    /// Язык интерфейса: ru, en или sr (латиница).
    pub language: String,
    /// Цветовая тема UI (id из src/lib/themes.js).
    pub theme: String,
    /// Схема: `dark`, `light` или `system` (как в KDE); `None` — как в системе.
    pub scheme: Option<String>,
    /// Непрозрачность фона окна, 0.15–0.85; `None` — по умолчанию темы.
    pub opacity: Option<f32>,
    /// Размытие фона KWin; старые конфиги сохраняют включённый эффект.
    pub blur_enabled: Option<bool>,
    /// Показывать календарь справа от почты, когда хватает ширины.
    pub wide_layout: Option<bool>,
    /// Масштаб интерфейса по умолчанию, 0.5–2.0; `None` — 100%.
    pub zoom: Option<f64>,
    /// Как часто проверять новые письма, секунд; `None` — 10.
    pub poll_seconds: Option<u64>,
    /// Средний клик по заголовку сворачивает окно; `None` — включено.
    pub middle_click_minimize: Option<bool>,
    /// Где виден значок: `panelTray` (панель и трей), `tray` (только трей), `panel` (только панель);
    /// `None` — панель и трей.
    pub tray_mode: Option<String>,
    /// Значок в трее: `color`, `auto` (монохромный по теме KDE), `light`, `dark`; `None` — цветной.
    pub tray_icon: Option<String>,
    /// Размеры окон между запусками: `main`, `compose`.
    pub windows: std::collections::BTreeMap<String, crate::window_state::Geometry>,
    /// «Поверх других окон» и «На всех рабочих столах» главного окна и настроек.
    pub window_pins: std::collections::BTreeMap<String, crate::window_pins::Pins>,
    /// Вид календаря при запуске: `month`, `week`, `day`; `None` — неделя.
    pub calendar_view: Option<String>,
    /// В клетке месяца названия событий переносятся по строкам, а не обрезаются; `None` — обрезаются.
    pub calendar_full_titles: Option<bool>,
    /// Мягкое свечение вокруг листа письма вместо резкого края на стекле; `None` — включено.
    pub soft_mail_edges: Option<bool>,
    /// Кнопки «Свернуть» и «Развернуть» в шапке; `None` — показывать.
    pub window_buttons: Option<bool>,
    /// Кнопки «Поверх других окон» и «На всех рабочих столах» в шапке; `None` — показывать.
    pub pin_buttons: Option<bool>,
    /// Уведомления о новых письмах во «Входящих»; `None` — включены.
    pub notify_mail: Option<bool>,
    /// Напоминания о событиях календаря; `None` — включены.
    pub notify_events: Option<bool>,
    /// Подключённый аккаунт Google Календаря (ключи — в KWallet).
    pub google_account: Option<String>,
    /// Календари, скрытые в сетке (id календарей Google).
    pub hidden_calendars: std::collections::BTreeSet<String>,
    /// Показывать в дереве общий раздел «Все ящики»; `None` — показывать.
    pub show_all_accounts: Option<bool>,
    /// Крестик прячет окно в трей, а не закрывает приложение; `None` — прячет.
    pub close_to_tray: Option<bool>,
    /// За сколько дней хранить на диске содержимое писем (заголовки — всегда);
    /// `None` — всё, `Some(0)` — только до следующего запуска.
    pub body_days: Option<u32>,
    /// Язык перевода писем (код Google из translation::LANGUAGES); `None` — язык интерфейса.
    pub translate_to: Option<String>,
    /// Отправители, у которых внешнее содержимое показывается сразу (адреса в нижнем регистре).
    pub trusted_senders: BTreeSet<String>,
}

pub struct ConfigStore {
    path: PathBuf,
    config: Mutex<Config>,
}

pub const POLL_CHOICES: [u64; 5] = [5, 10, 30, 60, 300];
pub const BODY_DAYS_CHOICES: [u32; 6] = [0, 30, 90, 180, 365, 1826];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TrayMode {
    PanelTray,
    Tray,
    Panel,
}

impl TrayMode {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "panelTray" => Self::PanelTray,
            "tray" => Self::Tray,
            "panel" => Self::Panel,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::PanelTray => "panelTray",
            Self::Tray => "tray",
            Self::Panel => "panel",
        }
    }

    pub fn tray(self) -> bool {
        self != Self::Panel
    }
}

impl Config {
    pub fn language(&self) -> &'static str {
        match self.language.as_str() {
            "en" => "en",
            "sr" => "sr",
            _ => "ru",
        }
    }

    pub fn tray_mode(&self) -> TrayMode {
        self.tray_mode.as_deref().and_then(TrayMode::parse).unwrap_or(TrayMode::PanelTray)
    }

    /// Письма старше этого Unix-времени хранятся без содержимого; `None` — хранить всё.
    pub fn body_cutoff(&self) -> Option<i64> {
        let days = self.body_days.filter(|d| BODY_DAYS_CHOICES.contains(d))?;
        Some(chrono::Utc::now().timestamp() - i64::from(days) * 86_400)
    }

    pub fn poll_seconds(&self) -> u64 {
        self.poll_seconds.filter(|s| POLL_CHOICES.contains(s)).unwrap_or(10)
    }
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

#[cfg(test)]
mod language_tests {
    use super::*;

    #[test]
    fn blur_preference_round_trips_and_defaults_to_enabled() {
        let old: Config = serde_json::from_str("{}").unwrap();
        assert!(old.blur_enabled.unwrap_or(true));
        for enabled in [false, true] {
            let config = Config { blur_enabled: Some(enabled), ..Default::default() };
            let restored: Config = serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
            assert_eq!(restored.blur_enabled, Some(enabled));
        }
    }

    #[test]
    fn legacy_config_keeps_russian() {
        let config: Config = serde_json::from_str(r#"{"theme":"graphite","zoom":1.1}"#).unwrap();
        assert_eq!(config.language(), "ru");
        assert_eq!(config.zoom, Some(1.1));
    }

    #[test]
    fn language_survives_saving_and_loading() {
        let dir = std::env::temp_dir().join(format!("mmail-language-test-{}", std::process::id()));
        let store = ConfigStore::load(dir.clone());
        for code in ["ru", "en", "sr"] {
            store.update(|c| c.language = code.into()).unwrap();
            assert_eq!(ConfigStore::load(dir.clone()).get().language(), code);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn config_is_private_to_the_user() {
        let dir = std::env::temp_dir().join(format!("mmail-config-perms-{}", std::process::id()));
        let store = ConfigStore::load(dir.clone());
        store.update(|c| { c.trusted_senders.insert("a@b.example".into()); }).unwrap();
        let mode = |p: &std::path::Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("config.json")), 0o600, "в настройках — адреса доверенных отправителей и аккаунт Google");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unknown_language_falls_back_to_russian() {
        assert_eq!(Config { language: "unknown".into(), ..Default::default() }.language(), "ru");
    }
}
