//! この端末だけの記憶 (`%LOCALAPPDATA%\HatoPo\state.json`)。共有には書かない。
//!
//! - 開いて読んだ通知 (マスコットの「新着」表示を消すため)
//! - 既読にした一斉送信 (一覧には残すが、件数とトーストから外す)
//! - マスコットの位置と表示/非表示

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalState {
    pub seen: BTreeSet<String>,
    pub read_broadcasts: BTreeSet<String>,
    pub mascot_pos: Option<[f32; 2]>,
    pub mascot_hidden: bool,
    /// 自分で選んだマスコットの大きさ (%)。None なら settings.json の既定
    pub mascot_scale: Option<u32>,
}

impl LocalState {
    pub fn path() -> PathBuf {
        crate::boot::local_dir().join("state.json")
    }

    /// 無い・壊れているときは空から始める (困るのは「新着」表示が 1 回余計に出る程度)。
    pub fn load() -> LocalState {
        std::fs::read(Self::path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let p = Self::path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(b) = serde_json::to_vec_pretty(self) {
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, b).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
    }

    /// 今ある通知の ID だけを残す (完了・期限切れで消えたものを覚え続けない)。
    /// 共有に繋がらず一覧が空のときに呼ぶと全部忘れるので、読めたときだけ呼ぶこと。
    pub fn prune(&mut self, alive: &BTreeSet<String>) -> bool {
        let before = (self.seen.len(), self.read_broadcasts.len());
        self.seen.retain(|id| alive.contains(id));
        self.read_broadcasts.retain(|id| alive.contains(id));
        before != (self.seen.len(), self.read_broadcasts.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_keeps_only_alive_ids() {
        let mut s = LocalState::default();
        s.seen.extend(["a".to_string(), "b".to_string()]);
        s.read_broadcasts.insert("b".into());
        let alive: BTreeSet<String> = ["b".to_string()].into();
        assert!(s.prune(&alive));
        assert_eq!(s.seen, alive);
        assert_eq!(s.read_broadcasts, alive);
        assert!(!s.prune(&alive));
    }

    #[test]
    fn unknown_fields_and_missing_fields_are_tolerated() {
        let s: LocalState = serde_json::from_str(r#"{"seen":["x"],"future_field":1}"#).unwrap();
        assert!(s.seen.contains("x"));
        assert!(!s.mascot_hidden);
    }
}
