use crate::loft::{to_json, write_atomic};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// 鳩の巣の `settings.json`。管理者が直接編集する。書かれていない項目は既定値。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
/// 確認の間隔の上限 (これより長いと通知が届くのが遅すぎる)。
pub const MAX_POLL_SECONDS: u64 = 3600;
/// 催促の間隔の上限 (1 日)。
pub const MAX_REMIND_MINUTES: u64 = 1440;

impl Settings {
    /// 管理画面から保存する。範囲外の値やキャラクターの名前の誤りは書く前に弾く。
    pub fn save(&self, root: &Path) -> Result<()> {
        self.validate()?;
        write_atomic(&root.join("settings.json"), &to_json(self)?)
    }

    pub fn validate(&self) -> Result<()> {
        if !(MIN_POLL_SECONDS..=MAX_POLL_SECONDS).contains(&self.poll_seconds) {
            return Err(Error::Invalid(format!(
                "確認の間隔は {MIN_POLL_SECONDS}〜{MAX_POLL_SECONDS} 秒にしてください。"
            )));
        }
        if self.remind_minutes > MAX_REMIND_MINUTES {
            return Err(Error::Invalid(format!(
                "催促の間隔は 0〜{MAX_REMIND_MINUTES} 分にしてください (0 は催促しない)。"
            )));
        }
        crate::model::normalize_name(&self.character).map_err(|_| {
            Error::Invalid(format!(
                "キャラクター「{}」の名前が正しくありません。",
                self.character
            ))
        })?;
        Ok(())
    }

    /// `character_icon\` の下にあるキャラクターのフォルダ名。
    pub fn characters(root: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(root.join("character_icon"))
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .filter_map(|e| e.file_name().to_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

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
    fn save_round_trips_and_rejects_out_of_range() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings {
            poll_seconds: 30,
            remind_minutes: 0,
            ..Default::default()
        };
        s.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()), (s, None));
        for bad in [
            Settings {
                poll_seconds: 5,
                ..Default::default()
            },
            Settings {
                poll_seconds: 7200,
                ..Default::default()
            },
            Settings {
                remind_minutes: 2000,
                ..Default::default()
            },
            Settings {
                character: r"a\b".into(),
                ..Default::default()
            },
        ] {
            assert!(bad.save(dir.path()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn characters_are_listed_from_folders() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Settings::characters(dir.path()).is_empty());
        std::fs::create_dir_all(dir.path().join("character_icon/HatoPoBlue")).unwrap();
        std::fs::create_dir_all(dir.path().join("character_icon/HatoPoPink")).unwrap();
        std::fs::write(dir.path().join("character_icon/readme.txt"), "x").unwrap();
        assert_eq!(
            Settings::characters(dir.path()),
            ["HatoPoBlue", "HatoPoPink"]
        );
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
