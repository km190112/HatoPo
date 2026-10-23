//! 各 PC の HatoPo が書く「生存印」(`status\user\<名前>.json` / `status\device\<PC名>.json`)。
//!
//! 管理画面はこれを読んで、ユーザー・端末ごとの最終アクセス・最後に完了した日・HatoPo の版を出す。
//! 共有を叩きすぎないよう、利用者側は起動時・完了時・一定間隔ごとにだけ書く。

use crate::loft::{read_json, to_json, write_atomic};
use crate::model::normalize_name;
use crate::{Error, Kind, Origin, Result};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClientStatus {
    /// `user` か `device`
    pub kind: Kind,
    pub id: String,
    /// ユーザーなら最後に使った PC、端末なら最後にログオンしていたユーザー
    #[serde(default)]
    pub counterpart: String,
    /// その PC で動いている HatoPo の版
    #[serde(default)]
    pub version: String,
    pub last_seen: NaiveDateTime,
    /// 最後に通知を完了した (一覧から消した) 日時
    #[serde(default)]
    pub last_done_at: Option<NaiveDateTime>,
    #[serde(default)]
    pub last_done_title: Option<String>,
}

fn dir(root: &Path, kind: Kind) -> PathBuf {
    let k = if kind == Kind::Device {
        "device"
    } else {
        "user"
    };
    root.join("status").join(k)
}

fn file(root: &Path, kind: Kind, id: &str) -> Result<PathBuf> {
    Ok(dir(root, kind).join(format!("{}.json", normalize_name(id)?)))
}

pub fn read_status(root: &Path, kind: Kind, id: &str) -> Option<ClientStatus> {
    read_json(&file(root, kind, id).ok()?).ok()
}

/// 自分 (ユーザー) と自分の PC (端末) の生存印を書く。
/// `done` に完了した通知の件名を渡すと「最後に完了した日」も更新する。渡さなければ前の値を保つ。
pub fn touch(
    root: &Path,
    me: &Origin,
    version: &str,
    now: NaiveDateTime,
    done: Option<&str>,
) -> Result<()> {
    for (kind, id, other) in [
        (Kind::User, &me.user, &me.host),
        (Kind::Device, &me.host, &me.user),
    ] {
        // 名前が取れない環境では書かない
        let Ok(path) = file(root, kind, id) else {
            continue;
        };
        let prev = read_status(root, kind, id);
        let (last_done_at, last_done_title) = match done {
            Some(t) => (Some(now), Some(t.to_string())),
            None => prev
                .map(|p| (p.last_done_at, p.last_done_title))
                .unwrap_or((None, None)),
        };
        let s = ClientStatus {
            kind,
            id: normalize_name(id)?,
            counterpart: other.clone(),
            version: version.into(),
            last_seen: now,
            last_done_at,
            last_done_title,
        };
        let d = dir(root, kind);
        std::fs::create_dir_all(&d).map_err(|e| Error::Io { path: d, source: e })?;
        write_atomic(&path, &to_json(&s)?)?;
    }
    Ok(())
}

/// 全員分。読めないファイルは飛ばす (生存印は消えても困らない)。
pub fn read_all_status(root: &Path) -> Result<Vec<ClientStatus>> {
    let mut out = Vec::new();
    for kind in [Kind::User, Kind::Device] {
        let d = dir(root, kind);
        let rd = match std::fs::read_dir(&d) {
            Ok(rd) => rd,
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(e) => return Err(Error::Io { path: d, source: e }),
        };
        for e in rd.flatten() {
            let p = e.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || !name.ends_with(".json") {
                continue;
            }
            if let Ok(s) = read_json::<ClientStatus>(&p) {
                out.push(s);
            }
        }
    }
    out.sort_by(|a, b| (a.kind, &a.id).cmp(&(b.kind, &b.id)));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 23)
            .unwrap()
            .and_hms_opt(h, 0, 0)
            .unwrap()
    }

    fn me() -> Origin {
        Origin {
            user: "Taro".into(),
            host: "PC-0123".into(),
        }
    }

    #[test]
    fn touch_writes_user_and_device_and_keeps_last_done() {
        let d = tempfile::tempdir().unwrap();
        touch(d.path(), &me(), "0.2.0", at(9), Some("工数表の入力")).unwrap();
        touch(d.path(), &me(), "0.2.0", at(10), None).unwrap();

        let u = read_status(d.path(), Kind::User, "taro").unwrap();
        assert_eq!(u.last_seen, at(10));
        assert_eq!(
            u.last_done_at,
            Some(at(9)),
            "完了でない更新は前の完了日を保つ"
        );
        assert_eq!(u.last_done_title.as_deref(), Some("工数表の入力"));
        assert_eq!(u.counterpart, "PC-0123");

        let dv = read_status(d.path(), Kind::Device, "pc-0123").unwrap();
        assert_eq!(
            (dv.counterpart.as_str(), dv.version.as_str()),
            ("Taro", "0.2.0")
        );

        let all = read_all_status(d.path()).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].kind, Kind::Device, "端末 → ユーザーの順");
    }

    #[test]
    fn empty_names_are_skipped_and_garbage_is_ignored() {
        let d = tempfile::tempdir().unwrap();
        let nobody = Origin {
            user: String::new(),
            host: "pc-1".into(),
        };
        touch(d.path(), &nobody, "0.2.0", at(9), None).unwrap();
        std::fs::write(d.path().join("status/device/bad.json"), "{").unwrap();
        let all = read_all_status(d.path()).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "pc-1");
    }

    #[test]
    fn no_status_folder_is_empty() {
        let d = tempfile::tempdir().unwrap();
        assert!(read_all_status(d.path()).unwrap().is_empty());
    }
}
