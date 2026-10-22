use serde::Deserialize;
use std::path::Path;

/// 鳩の巣の `settings.json`。管理者が直接編集する。書かれていない項目は既定値。
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// `character_icon\` の下のフォルダ名。
    pub character: String,
    /// 共有フォルダを見に行く間隔 (秒)。
    pub poll_seconds: u64,
    /// 未完了の通知をトーストで出し直す間隔 (分)。0 で出し直さない。
    pub remind_minutes: u64,
    /// マスコットをデスクトップに出すか。false ならトレイだけ。
    pub show_mascot: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            character: "HatoPoBlue".into(),
            poll_seconds: 60,
            remind_minutes: 60,
            show_mascot: true,
        }
    }
}

/// 共有を叩きすぎないための下限。
pub const MIN_POLL_SECONDS: u64 = 10;

impl Settings {
    /// 無い・読めないときは既定値で動かす (通知が止まるよりよい)。読めなかった理由は 2 つ目で返す。
    pub fn load(root: &Path) -> (Settings, Option<String>) {
        let path = root.join("settings.json");
        let text = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return (Settings::default(), None);
            }
            Err(e) => {
                return (
                    Settings::default(),
                    Some(format!("settings.json が読めません: {e}")),
                );
            }
        };
        // メモ帳で保存すると BOM が付くことがある
        let text = text.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&text);
        match serde_json::from_slice::<Settings>(text) {
            Ok(mut s) => {
                s.poll_seconds = s.poll_seconds.max(MIN_POLL_SECONDS);
                (s, None)
            }
            Err(e) => (
                Settings::default(),
                Some(format!("settings.json の書式が誤っています: {e}")),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_text(text: &[u8]) -> (Settings, Option<String>) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), text).unwrap();
        Settings::load(dir.path())
    }

    #[test]
    fn missing_file_gives_defaults_without_warning() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(dir.path()), (Settings::default(), None));
    }

    #[test]
    fn partial_file_fills_defaults_and_bom_is_ok() {
        let (s, w) = load_text("\u{feff}{\"character\": \"HatoPoPink\"}".as_bytes());
        assert_eq!(w, None);
        assert_eq!(s.character, "HatoPoPink");
        assert_eq!(s.poll_seconds, 60);
    }

    #[test]
    fn too_short_poll_is_clamped() {
        let (s, _) = load_text(br#"{"poll_seconds": 1}"#);
        assert_eq!(s.poll_seconds, MIN_POLL_SECONDS);
    }

    #[test]
    fn broken_file_gives_defaults_with_warning() {
        let (s, w) = load_text(br#"{"poll_seconds": "sixty"#);
        assert_eq!(s, Settings::default());
        assert!(w.unwrap().contains("書式"));
    }
}
