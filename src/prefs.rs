//! Remembers the menu choices between launches, in a small text file under
//! %APPDATA%\BreakpointBilliards (or ~/.config/breakpoint-billiards).

use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub mode: String,
    pub opponent: String,
    pub level: String,
    pub theme: usize,
    pub view: String,
    pub guide: u8,
    pub pocket_hints: bool,
    /// 8-ball: shots must be called.
    pub call_shots: bool,
    pub sound: bool,
    /// 0 = off .. 3 = high.
    pub music: u8,
    pub fullscreen: bool,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            mode: "8".into(),
            opponent: "computer".into(),
            level: "medium".into(),
            theme: 0,
            view: "shooter".into(),
            guide: 0,
            pocket_hints: true,
            call_shots: false,
            sound: true,
            music: 2,
            fullscreen: true,
        }
    }
}

fn path() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("APPDATA") {
        return Some(PathBuf::from(dir).join("BreakpointBilliards").join("settings.cfg"));
    }
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".config").join("breakpoint-billiards").join("settings.cfg"))
}

impl Prefs {
    pub fn parse(text: &str) -> Prefs {
        let map: HashMap<&str, &str> = text
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim(), v.trim()))
            .collect();
        let d = Prefs::default();
        let s = |k: &str, def: &str| map.get(k).map_or(def.to_string(), |v| v.to_string());
        let b = |k: &str, def: bool| map.get(k).map_or(def, |v| *v == "true");
        Prefs {
            mode: s("mode", &d.mode),
            opponent: s("opponent", &d.opponent),
            level: s("level", &d.level),
            theme: map.get("theme").and_then(|v| v.parse().ok()).unwrap_or(d.theme),
            view: s("view", &d.view),
            guide: map.get("guide").and_then(|v| v.parse().ok()).filter(|g| *g <= 2).unwrap_or(d.guide),
            pocket_hints: b("pocket_hints", d.pocket_hints),
            call_shots: b("call_shots", d.call_shots),
            sound: b("sound", d.sound),
            music: map.get("music").and_then(|v| v.parse().ok()).filter(|m| *m <= 3).unwrap_or(d.music),
            fullscreen: b("fullscreen", d.fullscreen),
        }
    }

    pub fn to_text(&self) -> String {
        format!(
            "# Breakpoint Billiards settings\nmode={}\nopponent={}\nlevel={}\ntheme={}\nview={}\nguide={}\npocket_hints={}\ncall_shots={}\nsound={}\nmusic={}\nfullscreen={}\n",
            self.mode, self.opponent, self.level, self.theme, self.view, self.guide, self.pocket_hints, self.call_shots, self.sound, self.music, self.fullscreen
        )
    }

    pub fn load() -> Prefs {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| Prefs::parse(&t))
            .unwrap_or_default()
    }

    /// Best effort: a read-only profile just means settings are not kept.
    pub fn save(&self) {
        if let Some(p) = path() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(p, self.to_text());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_survive_junk() {
        let p = Prefs { theme: 7, guide: 2, call_shots: true, sound: false, music: 1, view: "orbit".into(), ..Prefs::default() };
        assert_eq!(Prefs::parse(&p.to_text()), p);
        let junk = Prefs::parse("theme=banana\nguide=9\nmusic=7\nnonsense\n=\n");
        assert_eq!(junk, Prefs::default());
    }
}
