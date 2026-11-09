//! Existing config.json contract; unknown fields survive typed edits.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

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
    /// Legacy bindings remain active; newly created bindings explicitly start off.
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
impl Config {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
        let mut config: Self = serde_json::from_slice(bytes).context("config.json 格式错误")?;
        ensure!(
            config.routes_dir == "routes",
            "当前 AHK 执行端仅支持 routesDir=routes，配置未修改"
        );
        // Normalize in memory only. ConfigFile compares against a parsed original,
        // preserving its byte-for-byte no-op-save contract even for legacy files.
        let stop = hotkey_canonical(&config.hotkeys.stop_all)?;
        let mut active = BTreeSet::new();
        for binding in config.hotkeys.bindings.iter_mut().rev() {
            if !binding.enabled || binding.action != "run_route" {
                continue;
            }
            let key = hotkey_canonical(&binding.hotkey)?;
            if key.is_empty() || stop_conflicts(&key, &stop) || !active.insert(key) {
                binding.enabled = false;
            }
        }
        Ok(config)
    }

    pub fn set_binding(&mut self, stem: &str, hotkey: &str, start_at: usize) -> Result<()> {
        ensure!(start_at > 0, "起始点必须 ≥1");
        // config.json feeds AHK directly: retain native notation in storage.
        let hotkey = hotkey_to_ahk(hotkey)?;
        let key = hotkey_canonical(&hotkey)?;
        if hotkey.is_empty() {
            self.hotkeys.bindings.retain(|b| b.route != stem);
            return Ok(());
        }
        self.ensure_not_stop(&key)?;
        let existing = self.hotkeys.bindings.iter().position(|b| b.route == stem);
        let enabled = existing.is_some_and(|i| self.hotkeys.bindings[i].enabled);
        // All fallible checks precede mutation, including checks of other bindings.
        let conflicts = if enabled {
            self.conflicting_bindings(stem, &key)?
        } else {
            Vec::new()
        };
        if let Some(index) = existing {
            self.hotkeys.bindings[index].hotkey = hotkey;
            self.hotkeys.bindings[index].start_at = start_at;
        } else {
            self.hotkeys.bindings.push(Binding {
                hotkey,
                action: run_action(),
                route: stem.into(),
                start_at,
                enabled: false,
                extra: Map::new(),
            });
        }
        for index in conflicts {
            self.hotkeys.bindings[index].enabled = false;
        }
        Ok(())
    }

    /// Enable this route and atomically disable any equivalent active binding.
    /// Disabling never restores a previously displaced route, and can repair an
    /// invalid binding without needing to parse its hotkey.
    pub fn set_binding_enabled(&mut self, stem: &str, enabled: bool) -> Result<()> {
        let index = self
            .hotkeys
            .bindings
            .iter()
            .position(|b| b.route == stem)
            .context("路线尚未绑定快捷键")?;
        let conflicts = if enabled {
            let binding = &self.hotkeys.bindings[index];
            ensure!(binding.action == "run_route", "此绑定不是路线执行快捷键");
            let key = hotkey_canonical(&binding.hotkey)?;
            ensure!(!key.is_empty(), "路线尚未绑定快捷键");
            self.ensure_not_stop(&key)?;
            self.conflicting_bindings(stem, &key)?
        } else {
            Vec::new()
        };
        for other in conflicts {
            self.hotkeys.bindings[other].enabled = false;
        }
        self.hotkeys.bindings[index].enabled = enabled;
        Ok(())
    }

    fn ensure_not_stop(&self, key: &str) -> Result<()> {
        let stop = hotkey_canonical(&self.hotkeys.stop_all)?;
        ensure!(!stop_conflicts(key, &stop), "路线快捷键不能占用全局急停键");
        Ok(())
    }

    fn conflicting_bindings(&self, stem: &str, key: &str) -> Result<Vec<usize>> {
        let mut conflicts = Vec::new();
        for (index, binding) in self.hotkeys.bindings.iter().enumerate() {
            if binding.route != stem
                && binding.enabled
                && binding.action == "run_route"
                && hotkey_canonical(&binding.hotkey)? == key
            {
                conflicts.push(index);
            }
        }
        Ok(conflicts)
    }

    /// Binding lookup for the route list's hotkey column.
    pub fn binding_for(&self, stem: &str) -> Option<&Binding> {
        self.hotkeys.bindings.iter().find(|b| b.route == stem)
    }
}

fn stop_conflicts(key: &str, stop: &str) -> bool {
    if key.is_empty() || stop.is_empty() {
        return false;
    }
    let (Ok(key), Ok(stop)) = (parse_native(key), parse_native(stop)) else {
        return false;
    };
    if key.key.strip_suffix(" up").unwrap_or(&key.key)
        != stop.key.strip_suffix(" up").unwrap_or(&stop.key)
    {
        return false;
    }
    // For each modifier, bits represent none / left / right / both held.
    // Wildcards allow extra modifiers; sided variants still overlap a generic
    // stop key (and key-up must not start playback after a key-down stop).
    fn states(hotkey: &NativeHotkey, order: usize) -> u8 {
        let mut allowed = 0b1111;
        let mut required = false;
        for &(candidate, side) in &hotkey.modifiers {
            if candidate == order {
                required = true;
                allowed &= match side {
                    1 => 0b1010,
                    2 => 0b1100,
                    _ => 0b1110,
                };
            }
        }
        if !required && !hotkey.flags.contains(&'*') {
            0b0001
        } else {
            allowed
        }
    }
    (0..4).all(|order| states(&key, order) & states(&stop, order) != 0)
}

fn human_modifier(part: &str) -> Option<&'static str> {
    Some(match part.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => "^",
        "alt" => "!",
        "shift" => "+",
        "win" | "windows" | "super" => "#",
        "lctrl" | "lcontrol" => "<^",
        "rctrl" | "rcontrol" => ">^",
        "lalt" => "<!",
        "ralt" => ">!",
        "lshift" => "<+",
        "rshift" => ">+",
        "lwin" => "<#",
        "rwin" => ">#",
        _ => return None,
    })
}

/// Human modifiers become AHK syntax. Native notation and single keys,
/// including apostrophes, remain intact. This validates modifier structure;
/// the AHK runtime remains responsible for its full key-name vocabulary.
pub fn hotkey_to_ahk(input: &str) -> Result<String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(String::new());
    }
    if input.chars().count() == 1
        || input.starts_with(['^', '!', '#', '+', '~', '$', '*', '<', '>'])
    {
        parse_native(input)?;
        return Ok(input.into());
    }
    let text = input.replace('-', "+");
    let mut prefix = String::new();
    let mut key = None;
    for part in text.split('+').map(str::trim) {
        if let Some(modifier) = human_modifier(part) {
            prefix.push_str(modifier);
        } else {
            ensure!(!part.is_empty(), "热键包含空按键");
            ensure!(key.is_none(), "热键只能包含一个主键");
            key = Some(part);
        }
    }
    let key = key.context("热键缺少主键")?;
    Ok(format!("{prefix}{key}"))
}

struct NativeHotkey {
    modifiers: BTreeSet<(usize, u8)>,
    flags: BTreeSet<char>,
    key: String,
}

fn parse_native(input: &str) -> Result<NativeHotkey> {
    let mut modifiers = BTreeSet::new();
    let mut flags = BTreeSet::new();
    if input.chars().count() == 1 {
        return Ok(NativeHotkey {
            modifiers,
            flags,
            key: input.into(),
        });
    }
    let mut chars = input.char_indices().peekable();
    let mut key_start = input.len();
    while let Some((index, character)) = chars.next() {
        match character {
            '~' | '$' | '*' => {
                flags.insert(character);
            }
            '<' | '>' => {
                let (_, modifier) = chars.next().context("侧键标记后缺少修饰键")?;
                let order =
                    modifier_order(modifier).context("侧键标记后必须为 Ctrl、Alt、Shift 或 Win")?;
                modifiers.insert((order, if character == '<' { 1 } else { 2 }));
            }
            '^' | '!' | '+' | '#' => {
                modifiers.insert((modifier_order(character).unwrap(), 0));
            }
            _ => {
                key_start = index;
                break;
            }
        }
    }
    let key = input[key_start..]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    ensure!(!key.is_empty(), "热键缺少主键");
    Ok(NativeHotkey {
        modifiers,
        flags,
        key,
    })
}

fn modifier_order(character: char) -> Option<usize> {
    "^!+#".chars().position(|candidate| candidate == character)
}

fn canonical_key(key: &str) -> String {
    let lower = key.to_ascii_lowercase();
    // Treat press and release variants separately, while accepting AHK aliases.
    let (key, suffix) = lower
        .strip_suffix(" up")
        .map_or((lower.as_str(), ""), |key| (key, " up"));
    let key = match key {
        "esc" => "escape",
        "return" => "enter",
        "del" => "delete",
        "ins" => "insert",
        "bs" => "backspace",
        _ => key,
    };
    format!("{key}{suffix}")
}

/// Comparison key shared with the daemon. Modifier order and key case do not
/// matter; left/right modifiers and wildcard/release variants remain distinct.
/// `$` (hook) and `~` (pass through) do not create a second physical shortcut.
pub fn hotkey_canonical(input: &str) -> Result<String> {
    let native = hotkey_to_ahk(input)?;
    if native.is_empty() {
        return Ok(String::new());
    }
    let parsed = parse_native(&native)?;
    let mut result = String::new();
    if parsed.flags.contains(&'*') {
        result.push('*');
    }
    for (order, side) in parsed.modifiers {
        if side != 0 {
            result.push(if side == 1 { '<' } else { '>' });
        }
        result.push(b"^!+#"[order] as char);
    }
    result.push_str(&canonical_key(&parsed.key));
    Ok(result)
}

/// Human-readable shortcut for route lists and inputs. Invalid user text stays
/// visible so it can be repaired rather than silently replaced.
pub fn hotkey_display(input: &str) -> String {
    let Ok(native) = hotkey_to_ahk(input) else {
        return input.trim().into();
    };
    if native.is_empty() {
        return String::new();
    }
    let Ok(parsed) = parse_native(&native) else {
        return input.trim().into();
    };
    let mut parts = Vec::new();
    for (order, side) in parsed.modifiers {
        let name = ["Ctrl", "Alt", "Shift", "Win"][order];
        let side = match side {
            1 => "L",
            2 => "R",
            _ => "",
        };
        parts.push(format!("{side}{name}"));
    }
    let key = canonical_key(&parsed.key);
    let (key, suffix) = key
        .strip_suffix(" up")
        .map_or((key.as_str(), ""), |key| (key, " Up"));
    let key = match key {
        "escape" => "Esc".into(),
        "enter" => "Enter".into(),
        "delete" => "Delete".into(),
        "insert" => "Insert".into(),
        "backspace" => "Backspace".into(),
        "space" => "Space".into(),
        "tab" => "Tab".into(),
        _ if key.len() == 1
            || key.starts_with('f') && key[1..].chars().all(|c| c.is_ascii_digit()) =>
        {
            key.to_ascii_uppercase()
        }
        _ if suffix.is_empty() => parsed.key.clone(),
        _ => parsed.key[..parsed.key.len() - 3].to_string(),
    };
    parts.push(format!("{key}{suffix}"));
    let flags: String = parsed.flags.into_iter().collect();
    format!("{flags}{}", parts.join("+"))
}
