//! この端末だけの記憶 (`%LOCALAPPDATA%\HatoPo\state.json`)。
//!
//! - 開いた通知 (マスコットの「新着」表示を消すため。目に入っただけなので共有には書かない)
//! - 「既読にする」を押した通知。正は鳩の巣の既読の印 (`read\`) で、ここはその写しと、
//!   まだ鳩の巣へ書けていない操作 (繋がらなかったときに次の確認で書く)
//! - マスコットの位置と表示/非表示、大きさ、選んだキャラクター

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalState {
    pub seen: BTreeSet<String>,
    /// 既読にした通知 (一斉送信は件数とトーストから外す。個別は完了するまで数える)
    pub read: BTreeSet<String>,
    /// 既読・未読を切り替えたが、まだ鳩の巣へ書けていない通知
    pub read_dirty: BTreeSet<String>,
    pub mascot_pos: Option<[f32; 2]>,
    pub mascot_hidden: bool,
    /// 自分で選んだマスコットの大きさ (%)。None なら settings.json の既定
    pub mascot_scale: Option<u32>,
    /// 自分で選んだキャラクター (`character_icon\` の下のフォルダ名)。None なら settings.json の既定
    pub character: Option<String>,
}

impl LocalState {
    pub fn path() -> PathBuf {
        crate::boot::local_dir().join("state.json")
    }

    /// 無い・壊れているときは空から始める (困るのは「新着」表示が 1 回余計に出る程度)。
    pub fn load() -> LocalState {
        std::fs::read(Self::path())
            .ok()
            .map(|b| Self::from_bytes(&b))
            .unwrap_or_default()
    }

    /// v0.3 までの `read_broadcasts` (既読の一斉送信) は `read` へ移し、鳩の巣へ一度書く。
    fn from_bytes(b: &[u8]) -> LocalState {
        let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(b) else {
            return LocalState::default();
        };
        let old = v
            .as_object_mut()
            .and_then(|o| o.remove("read_broadcasts"))
            .and_then(|x| serde_json::from_value::<BTreeSet<String>>(x).ok());
        let mut s: LocalState = serde_json::from_value(v).unwrap_or_default();
        if let Some(old) = old {
            s.read_dirty.extend(old.iter().cloned());
            s.read.extend(old);
        }
        s
    }

    /// 既読・未読を切り替える (鳩の巣へは裏方が書く)。
    pub fn set_read(&mut self, id: &str, read: bool) {
        if read {
            self.read.insert(id.to_string());
        } else {
            self.read.remove(id);
        }
        self.read_dirty.insert(id.to_string());
    }

    /// 鳩の巣と照らし合わせた結果を取り込む。変わったら `true`。
    /// 照らし合わせている間に利用者がまた切り替えたものは、次の確認に回す。
    pub fn apply_sync(&mut self, sync: &ReadSync) -> bool {
        let before = (self.read.clone(), self.read_dirty.clone());
        for (id, state) in &sync.pushed {
            if self.read.contains(id) == *state {
                self.read_dirty.remove(id);
            }
        }
        for (id, state) in &sync.remote {
            if self.read_dirty.contains(id) {
                continue;
            }
            if *state {
                self.read.insert(id.clone());
            } else {
                self.read.remove(id);
            }
        }
        before != (self.read.clone(), self.read_dirty.clone())
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
        let count = |s: &Self| (s.seen.len(), s.read.len(), s.read_dirty.len());
        let before = count(self);
        self.seen.retain(|id| alive.contains(id));
        self.read.retain(|id| alive.contains(id));
        // 消えた通知へはもう書けない (書いても NotFound になる)
        self.read_dirty.retain(|id| alive.contains(id));
        before != count(self)
    }
}

/// 鳩の巣と照らし合わせた結果 (裏方が共有を読み書きして作り、`LocalState::apply_sync` で取り込む)。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReadSync {
    /// 鳩の巣へ書けた操作 (ID, 既読にしたか)
    pub pushed: Vec<(String, bool)>,
    /// 鳩の巣の今の状態 (ID, 既読か)。読めなかったものは入れない
    pub remote: Vec<(String, bool)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_keeps_only_alive_ids() {
        let mut s = LocalState::default();
        s.seen.extend(["a".to_string(), "b".to_string()]);
        s.set_read("b", true);
        s.set_read("a", true);
        let alive: BTreeSet<String> = ["b".to_string()].into();
        assert!(s.prune(&alive));
        assert_eq!(s.seen, alive);
        assert_eq!(s.read, alive);
        assert_eq!(s.read_dirty, alive);
        assert!(!s.prune(&alive));
    }

    fn ids(v: &[&str]) -> BTreeSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn old_read_broadcasts_move_to_read_and_are_pushed_once() {
        let s = LocalState::from_bytes(
            r#"{"seen":["x"],"read_broadcasts":["b1","b2"],"mascot_scale":125}"#.as_bytes(),
        );
        assert_eq!(s.read, ids(&["b1", "b2"]));
        assert_eq!(s.read_dirty, ids(&["b1", "b2"]), "鳩の巣へ書くまで dirty");
        assert_eq!(s.mascot_scale, Some(125));
        let saved = serde_json::to_string(&s).unwrap();
        assert!(!saved.contains("read_broadcasts"), "新しい形で保存する");
        assert_eq!(LocalState::from_bytes(b"{ broken"), LocalState::default());
    }

    #[test]
    fn pushed_ops_clear_dirty_and_remote_state_is_adopted() {
        let mut s = LocalState::default();
        s.set_read("a", true); // 押した → 書けた
        s.set_read("b", true); // 押した → 書けなかった (オフライン)
        let sync = ReadSync {
            pushed: vec![("a".into(), true)],
            // c は別の PC で既読にした、a も鳩の巣では既読
            remote: vec![("a".into(), true), ("b".into(), false), ("c".into(), true)],
        };
        assert!(s.apply_sync(&sync));
        assert_eq!(
            s.read,
            ids(&["a", "b", "c"]),
            "dirty の b は鳩の巣の未読で上書きしない"
        );
        assert_eq!(s.read_dirty, ids(&["b"]), "書けなかった b は次に再試行");
        assert!(!s.apply_sync(&sync), "2 回目は何も変わらない");

        // 未読に戻された (別の PC で) ものは未読になる
        let back = ReadSync {
            pushed: vec![],
            remote: vec![("c".into(), false)],
        };
        assert!(s.apply_sync(&back));
        assert!(!s.read.contains("c"));
    }

    #[test]
    fn toggled_again_while_syncing_stays_dirty() {
        let mut s = LocalState::default();
        s.set_read("a", true);
        // 裏方が「既読」を書いている間に、利用者が未読に戻した
        s.set_read("a", false);
        let sync = ReadSync {
            pushed: vec![("a".into(), true)],
            remote: vec![("a".into(), true)],
        };
        s.apply_sync(&sync);
        assert!(!s.read.contains("a"), "利用者の最後の操作が勝つ");
        assert!(s.read_dirty.contains("a"), "次の確認で未読を書く");
    }

    #[test]
    fn unknown_fields_and_missing_fields_are_tolerated() {
        let s: LocalState = serde_json::from_str(r#"{"seen":["x"],"future_field":1}"#).unwrap();
        assert!(s.seen.contains("x"));
        assert!(!s.mascot_hidden);
    }

    #[test]
    fn chosen_character_survives_save_and_old_files_mean_no_choice() {
        let s = LocalState {
            character: Some("HatoPoPink".into()),
            ..Default::default()
        };
        let back: LocalState =
            serde_json::from_slice(&serde_json::to_vec_pretty(&s).unwrap()).unwrap();
        assert_eq!(back.character.as_deref(), Some("HatoPoPink"));
        // この項目が無い古い state.json は「選んでいない」
        let old: LocalState = serde_json::from_str(r#"{"mascot_scale":125}"#).unwrap();
        assert_eq!(old.character, None);
        assert_eq!(old.mascot_scale, Some(125));
    }
}
