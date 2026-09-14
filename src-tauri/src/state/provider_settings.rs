//! Which providers are fetched and which ones reach the tray icon.
//!
//! The frontend persists this in settings.json under `providers`; Rust keeps
//! its own copy because the poller runs with the popup closed and cannot ask
//! the WebView. Loaded from the file at startup, replaced by
//! `set_provider_settings` whenever the user changes it.

use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ProviderToggle {
    pub enabled: bool,
    pub tray: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct ProviderSettings {
    pub claude: ProviderToggle,
    pub kimi: ProviderToggle,
    pub gpt: ProviderToggle,
}

impl Default for ProviderSettings {
    /// What the app did before this setting existed: Claude and Kimi on and in
    /// the tray (Kimi still appears only once a key is saved). GPT is opt-in —
    /// its credentials are found on disk rather than pasted, so defaulting it
    /// on would start sending a token to a new host just because Codex CLI
    /// happens to be installed.
    fn default() -> Self {
        Self {
            claude: ProviderToggle { enabled: true, tray: true },
            kimi: ProviderToggle { enabled: true, tray: true },
            gpt: ProviderToggle { enabled: false, tray: false },
        }
    }
}

impl ProviderSettings {
    /// Whether the user picked this provider for the tray. Unknown ids never are.
    pub fn in_tray(&self, id: &str) -> bool {
        let toggle = match id {
            "claude" => self.claude,
            "kimi" => self.kimi,
            "gpt" => self.gpt,
            _ => return false,
        };
        toggle.enabled && toggle.tray
    }
}

#[derive(Default)]
pub struct ProviderSettingsState(Mutex<ProviderSettings>);

impl ProviderSettingsState {
    pub fn new(settings: ProviderSettings) -> Self {
        Self(Mutex::new(settings))
    }

    pub fn get(&self) -> ProviderSettings {
        self.0.lock().map(|s| *s).unwrap_or_default()
    }

    pub fn set(&self, settings: ProviderSettings) {
        if let Ok(mut current) = self.0.lock() {
            *current = settings;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_keep_the_pre_setting_behaviour_and_gpt_opt_in() {
        let s = ProviderSettings::default();
        assert!(s.in_tray("claude"));
        assert!(s.in_tray("kimi"));
        assert!(!s.gpt.enabled);
        assert!(!s.in_tray("gpt"));
    }

    #[test]
    fn deserializes_what_the_frontend_writes() {
        let s: ProviderSettings = serde_json::from_value(serde_json::json!({
            "claude": {"enabled": true, "tray": false},
            "kimi": {"enabled": false, "tray": false},
            "gpt": {"enabled": true, "tray": true}
        }))
        .unwrap();
        assert!(!s.in_tray("claude"));
        assert!(!s.kimi.enabled);
        assert!(s.in_tray("gpt"));
    }

    #[test]
    fn a_missing_provider_takes_its_default() {
        let s: ProviderSettings = serde_json::from_value(serde_json::json!({
            "claude": {"enabled": false, "tray": false}
        }))
        .unwrap();
        assert!(!s.claude.enabled);
        assert_eq!(s.kimi, ProviderSettings::default().kimi);
    }

    #[test]
    fn a_disabled_provider_is_never_in_the_tray() {
        let mut s = ProviderSettings::default();
        s.claude = ProviderToggle { enabled: false, tray: true };
        assert!(!s.in_tray("claude"));
        assert!(!s.in_tray("other"));
    }
}
