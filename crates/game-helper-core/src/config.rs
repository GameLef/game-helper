//! Existing config.json contract; unknown fields survive typed edits.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

fn yes() -> bool {
    true
}
fn routes_dir() -> String {
    "routes".into()
}
fn stop_key() -> String {
    "F12".into()
}
fn one() -> usize {
    1
}
fn run_action() -> String {
    "run_route".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ahk_path: Option<String>,
    #[serde(default = "routes_dir")]
    pub routes_dir: String,
    #[serde(default)]
    pub hotkeys: Hotkeys,
    #[serde(default = "yes")]
    pub auto_start_daemon: bool,
    #[serde(default = "yes")]
    pub auto_reload_on_save: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            ahk_path: None,
            routes_dir: routes_dir(),
            hotkeys: Hotkeys::default(),
            auto_start_daemon: true,
            auto_reload_on_save: true,
            extra: Map::new(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hotkeys {
    #[serde(default = "stop_key")]
    pub stop_all: String,
    #[serde(default)]
    pub bindings: Vec<Binding>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            stop_all: stop_key(),
            bindings: Vec::new(),
            extra: Map::new(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    pub hotkey: String,
    #[serde(default = "run_action")]
    pub action: String,
    pub route: String,
    #[serde(default = "one")]
    pub start_at: usize,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
impl Config {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
        let config: Self = serde_json::from_slice(bytes).context("config.json 格式错误")?;
        ensure!(
            config.routes_dir == "routes",
            "当前 AHK 执行端仅支持 routesDir=routes，配置未修改"
        );
        Ok(config)
    }
    pub fn set_binding(&mut self, stem: &str, hotkey: &str, start_at: usize) -> Result<()> {
        ensure!(start_at > 0, "起始点必须 ≥1");
        // config.json feeds the AHK daemon directly: persist AHK syntax, never
        // the human notation. Empty input stays empty (unbind).
        let hotkey = hotkey_to_ahk(hotkey)?;
        ensure!(
            hotkey.is_empty()
                || !self
                    .hotkeys
                    .bindings
                    .iter()
                    .any(|b| b.route != stem && b.hotkey.eq_ignore_ascii_case(&hotkey)),
            "热键已绑定到其他路线"
        );
        if hotkey.is_empty() {
            self.hotkeys.bindings.retain(|b| b.route != stem);
        } else if let Some(binding) = self.hotkeys.bindings.iter_mut().find(|b| b.route == stem) {
            binding.hotkey = hotkey;
            binding.start_at = start_at;
        } else {
            self.hotkeys.bindings.push(Binding {
                hotkey,
                action: run_action(),
                route: stem.into(),
                start_at,
                extra: Map::new(),
            });
        }
        Ok(())
    }
    /// Binding lookup for the route list's hotkey column.
    pub fn binding_for(&self, stem: &str) -> Option<&Binding> {
        self.hotkeys.bindings.iter().find(|b| b.route == stem)
    }
}

/// Human modifiers become AHK syntax. Existing AHK notation and single keys,
/// including the user's apostrophe binding, remain intact.
pub fn hotkey_to_ahk(input: &str) -> Result<String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(String::new());
    }
    if input.len() == 1 || input.starts_with(['^', '!', '#', '+', '~', '$', '*']) {
        return Ok(input.into());
    }
    let text = input.replace('-', "+").replace(' ', "");
    let mut prefix = String::new();
    let mut key = None;
    for part in text.split('+') {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => prefix.push('^'),
            "alt" => prefix.push('!'),
            "shift" => prefix.push('+'),
            "win" | "windows" | "super" => prefix.push('#'),
            "" => anyhow::bail!("热键包含空按键"),
            _ => {
                ensure!(key.is_none(), "热键只能包含一个主键");
                key = Some(part.to_string());
            }
        }
    }
    let key = key.context("热键缺少主键")?;
    Ok(format!("{prefix}{key}"))
}
