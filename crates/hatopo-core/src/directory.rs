//! 名簿 (`directory.json`)。通知先にするユーザーと端末の一覧。管理画面だけが書く。
//!
//! 利用者の HatoPo は名簿を読まない (名簿に無い人・PC にも通知は届く)。
//! 名簿は管理者が宛先を選ぶためと、最終アクセスなどを一覧にするためのもの。

use crate::loft::{Lock, read_json, to_json, write_atomic};
use crate::model::normalize_name;
use crate::{Error, Origin, Result};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const DIRECTORY_FILE: &str = "directory.json";

/// 名簿の 1 行。`id` はログオン名かコンピューター名 (小文字に正規化して持つ)。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Member {
    pub id: String,
    /// 氏名や設置場所など、人が見て分かる名前
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Directory {
    /// 保存のたびに 1 増える。管理者 2 人が同時に直したときの検出に使う
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub users: Vec<Member>,
    #[serde(default)]
    pub devices: Vec<Member>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<NaiveDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_by: Option<Origin>,
}

fn path(root: &Path) -> PathBuf {
    root.join(DIRECTORY_FILE)
}

impl Directory {
    /// 無ければ空の名簿 (版 0)。壊れていれば `Broken` (黙って空にすると、保存した瞬間に名簿が消える)。
    pub fn load(root: &Path) -> Result<Directory> {
        let p = path(root);
        if !p.exists() {
            return Ok(Directory::default());
        }
        read_json(&p)
    }

    /// 保存する。読んだときから他の人が保存していたら `Conflict`。成功したら保存後の名簿を返す。
    pub fn save(&self, root: &Path, by: &Origin, now: NaiveDateTime) -> Result<Directory> {
        let mut next = self.clone();
        next.normalize()?;
        let p = path(root);
        let _lock = Lock::acquire(&p, "directory")?;
        let current = Directory::load(root)?;
        if current.revision != self.revision {
            return Err(Error::Conflict {
                id: DIRECTORY_FILE.into(),
                expected: self.revision,
                actual: current.revision,
            });
        }
        next.revision = current.revision + 1;
        next.updated_at = Some(now);
        next.updated_by = Some(by.clone());
        write_atomic(&p, &to_json(&next)?)?;
        Ok(next)
    }

    /// ID を小文字にそろえ、空・パスを壊す文字・重複を弾く。並びは ID 順にする。
    pub fn normalize(&mut self) -> Result<()> {
        for (label, list) in [("ユーザー", &mut self.users), ("端末", &mut self.devices)] {
            let mut seen = HashSet::new();
            for m in list.iter_mut() {
                m.id = normalize_name(&m.id)?;
                m.name = m.name.trim().to_string();
                m.note = m.note.trim().to_string();
                if !seen.insert(m.id.clone()) {
                    return Err(Error::Invalid(format!(
                        "{label}「{}」が名簿に 2 回あります。",
                        m.id
                    )));
                }
            }
            list.sort_by(|a, b| a.id.cmp(&b.id));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 23)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap()
    }

    fn admin() -> Origin {
        Origin {
            user: "hanako".into(),
            host: "pc-admin".into(),
        }
    }

    fn member(id: &str, name: &str) -> Member {
        Member {
            id: id.into(),
            name: name.into(),
            note: String::new(),
        }
    }

    #[test]
    fn missing_file_is_empty_revision_zero() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(Directory::load(d.path()).unwrap(), Directory::default());
    }

    #[test]
    fn save_round_trips_normalized_and_sorted() {
        let d = tempfile::tempdir().unwrap();
        let dir = Directory {
            users: vec![member(" Taro ", "山田 太郎"), member("Hanako", "佐藤 花子")],
            devices: vec![member("PC-0123", "組立 2 班")],
            ..Default::default()
        };
        let saved = dir.save(d.path(), &admin(), now()).unwrap();
        assert_eq!(saved.revision, 1);
        let back = Directory::load(d.path()).unwrap();
        assert_eq!(back, saved);
        let ids: Vec<_> = back.users.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["hanako", "taro"]);
        assert_eq!(back.devices[0].id, "pc-0123");
    }

    #[test]
    fn concurrent_admins_are_detected() {
        let d = tempfile::tempdir().unwrap();
        let a = Directory::load(d.path()).unwrap();
        let b = a.clone();
        let mut a2 = a.clone();
        a2.users.push(member("taro", ""));
        a2.save(d.path(), &admin(), now()).unwrap();
        let mut b2 = b;
        b2.users.push(member("jiro", ""));
        let err = b2.save(d.path(), &admin(), now()).unwrap_err();
        assert!(
            matches!(
                err,
                Error::Conflict {
                    expected: 0,
                    actual: 1,
                    ..
                }
            ),
            "{err}"
        );
        assert_eq!(Directory::load(d.path()).unwrap().users.len(), 1);
    }

    #[test]
    fn duplicates_and_bad_ids_are_rejected() {
        let d = tempfile::tempdir().unwrap();
        let dup = Directory {
            users: vec![member("taro", ""), member("TARO", "")],
            ..Default::default()
        };
        assert!(dup.save(d.path(), &admin(), now()).is_err());
        let bad = Directory {
            devices: vec![member("a\\b", "")],
            ..Default::default()
        };
        assert!(bad.save(d.path(), &admin(), now()).is_err());
        assert!(
            !d.path().join(DIRECTORY_FILE).exists(),
            "失敗したら何も書かない"
        );
    }

    #[test]
    fn broken_file_is_an_error_not_an_empty_list() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(DIRECTORY_FILE), "{ broken").unwrap();
        assert!(matches!(
            Directory::load(d.path()),
            Err(Error::Broken { .. })
        ));
    }
}
