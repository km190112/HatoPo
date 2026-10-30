//! 名簿 (`directory.json`)。通知先にするユーザーと端末の一覧。管理画面だけが書く。
//!
//! 名簿は管理者が宛先を選ぶためと、最終アクセスなどを一覧にするためのもの。
//! 利用者の HatoPo は差出人の名前を出すために読むだけ (名簿に無い人・PC にも通知は届く)。

use crate::loft::{Lock, read_json, to_json, write_atomic};
use crate::model::normalize_name;
use crate::{Error, Kind, Origin, Result};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

pub const DIRECTORY_FILE: &str = "directory.json";

/// CSV の 1 列目がこれに一致したら、1 行目を見出しとして読み飛ばす。
const HEADER_WORDS: &[&str] = &[
    "ログオン名",
    "ユーザー名",
    "ユーザー",
    "ログイン名",
    "コンピューター名",
    "コンピュータ名",
    "端末名",
    "pc名",
    "id",
];

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

    /// 名簿に載っている名前。載っていない・名前が空なら `None`。ID の大文字小文字は問わない。
    pub fn name_of(&self, kind: Kind, id: &str) -> Option<&str> {
        let list = match kind {
            Kind::User => &self.users,
            Kind::Device => &self.devices,
            Kind::Broadcast => return None,
        };
        let id = id.trim().to_lowercase();
        list.iter()
            .find(|m| m.id == id)
            .map(|m| m.name.as_str())
            .filter(|n| !n.is_empty())
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

/// 名簿を一括登録するために CSV で渡された 1 行が読めなかった理由。
#[derive(Clone, Debug, PartialEq)]
pub struct ImportError {
    /// 1 始まりの物理行番号 (見出し行も数える)。
    pub line: usize,
    pub raw: String,
    pub detail: String,
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}行目「{}」: {}", self.line, self.raw, self.detail)
    }
}

/// CSV からの名簿一括取り込みの結果。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportResult {
    pub members: Vec<Member>,
    pub errors: Vec<ImportError>,
}

/// UTF-8 (BOM 可) / Shift-JIS (CP932) を自動判定してテキストに戻す。
pub fn decode_csv_bytes(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    encoding_rs::SHIFT_JIS.decode(bytes).0.into_owned()
}

/// CSV (`ログオン名,名前,メモ` 列順) を読む。見出し行は自動で読み飛ばし、
/// 1 行ごとに ID を検査する。不正な行は取り込まずエラーに積み、有効な行だけ返す。
pub fn import_members_csv(bytes: &[u8]) -> ImportResult {
    let text = decode_csv_bytes(bytes);
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());

    let mut result = ImportResult::default();
    for (i, rec) in rdr.records().enumerate() {
        let line = i + 1;
        let rec = match rec {
            Ok(rec) => rec,
            Err(e) => {
                result.errors.push(ImportError {
                    line,
                    raw: String::new(),
                    detail: format!("CSV として読めません: {e}"),
                });
                continue;
            }
        };
        if i == 0 && is_header_row(&rec) {
            continue;
        }
        let id_raw = rec.get(0).unwrap_or("").trim();
        let name = rec.get(1).unwrap_or("").trim().to_string();
        let note = rec.get(2).unwrap_or("").trim().to_string();
        if id_raw.is_empty() && name.is_empty() && note.is_empty() {
            continue;
        }
        match normalize_name(id_raw) {
            Ok(id) => result.members.push(Member { id, name, note }),
            Err(e) => result.errors.push(ImportError {
                line,
                raw: rec.iter().collect::<Vec<_>>().join(","),
                detail: e.to_string(),
            }),
        }
    }
    result
}

fn is_header_row(rec: &csv::StringRecord) -> bool {
    let first = rec.get(0).unwrap_or("").trim();
    HEADER_WORDS.iter().any(|w| first.eq_ignore_ascii_case(w))
}

/// CSV から取り込んだ行を既存の名簿にマージする。ID が一致する行は上書き (新しい名前・メモに変える)、
/// 無ければ追加する。戻り値は `(新規件数, 上書き件数)`。
pub fn merge_members(list: &mut Vec<Member>, imported: &[Member]) -> (usize, usize) {
    let mut added = 0;
    let mut updated = 0;
    for m in imported {
        if let Some(existing) = list.iter_mut().find(|x| x.id == m.id) {
            *existing = m.clone();
            updated += 1;
        } else {
            list.push(m.clone());
            added += 1;
        }
    }
    (added, updated)
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

    fn member(id: &str, name: &str, note: &str) -> Member {
        Member {
            id: id.into(),
            name: name.into(),
            note: note.into(),
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
            users: vec![
                member(" Taro ", "山田 太郎", ""),
                member("Hanako", "佐藤 花子", ""),
            ],
            devices: vec![member("PC-0123", "組立 2 班", "")],
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
        a2.users.push(member("taro", "", ""));
        a2.save(d.path(), &admin(), now()).unwrap();
        let mut b2 = b;
        b2.users.push(member("jiro", "", ""));
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
            users: vec![member("taro", "", ""), member("TARO", "", "")],
            ..Default::default()
        };
        assert!(dup.save(d.path(), &admin(), now()).is_err());
        let bad = Directory {
            devices: vec![member("a\\b", "", "")],
            ..Default::default()
        };
        assert!(bad.save(d.path(), &admin(), now()).is_err());
        assert!(
            !d.path().join(DIRECTORY_FILE).exists(),
            "失敗したら何も書かない"
        );
    }

    #[test]
    fn name_of_finds_registered_names_only() {
        let dir = Directory {
            users: vec![member("taro", "山田 太郎", ""), member("jiro", "", "")],
            devices: vec![member("pc-0123", "組立 2 班", "")],
            ..Default::default()
        };
        assert_eq!(dir.name_of(Kind::User, "taro"), Some("山田 太郎"));
        assert_eq!(dir.name_of(Kind::User, "TARO"), Some("山田 太郎"));
        assert_eq!(dir.name_of(Kind::Device, "PC-0123"), Some("組立 2 班"));
        assert_eq!(dir.name_of(Kind::User, "jiro"), None, "名前が空");
        assert_eq!(dir.name_of(Kind::User, "hanako"), None, "名簿に無い");
        assert_eq!(
            dir.name_of(Kind::Device, "taro"),
            None,
            "ユーザーと端末は別"
        );
        assert_eq!(Directory::default().name_of(Kind::User, "taro"), None);
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

    // -------------------------------------------------------- CSV 取り込み

    #[test]
    fn decode_utf8_bom_is_stripped() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("taro,山田太郎,メモ".as_bytes());
        assert_eq!(decode_csv_bytes(&bytes), "taro,山田太郎,メモ");
    }

    #[test]
    fn decode_utf8_without_bom() {
        let bytes = "taro,山田太郎,メモ".as_bytes();
        assert_eq!(decode_csv_bytes(bytes), "taro,山田太郎,メモ");
    }

    #[test]
    fn decode_shift_jis_is_recognized() {
        let (cow, _, had_errors) = encoding_rs::SHIFT_JIS.encode("taro,山田太郎,メモ");
        assert!(!had_errors);
        assert_eq!(decode_csv_bytes(&cow), "taro,山田太郎,メモ");
    }

    #[test]
    fn header_row_is_skipped() {
        let csv = "ログオン名,名前,メモ\ntaro,山田太郎,組立";
        let r = import_members_csv(csv.as_bytes());
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.members, vec![member("taro", "山田太郎", "組立")]);
    }

    #[test]
    fn non_header_first_row_is_treated_as_data() {
        let csv = "taro,山田太郎,組立\njiro,鈴木次郎,";
        let r = import_members_csv(csv.as_bytes());
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(
            r.members,
            vec![
                member("taro", "山田太郎", "組立"),
                member("jiro", "鈴木次郎", ""),
            ]
        );
    }

    #[test]
    fn note_column_is_optional() {
        let csv = "ログオン名,名前,メモ\ntaro,山田太郎";
        let r = import_members_csv(csv.as_bytes());
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.members, vec![member("taro", "山田太郎", "")]);
    }

    #[test]
    fn blank_lines_are_silently_skipped() {
        let csv = "ログオン名,名前,メモ\ntaro,山田太郎,\n\njiro,鈴木次郎,";
        let r = import_members_csv(csv.as_bytes());
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.members.len(), 2);
    }

    #[test]
    fn invalid_id_row_is_skipped_and_reported_with_line_number() {
        let csv = "ログオン名,名前,メモ\ntaro,山田太郎,\na/b,不正,\njiro,鈴木次郎,";
        let r = import_members_csv(csv.as_bytes());
        assert_eq!(
            r.members,
            vec![
                member("taro", "山田太郎", ""),
                member("jiro", "鈴木次郎", ""),
            ]
        );
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.errors[0].line, 3, "見出し行を含む物理行番号");
    }

    #[test]
    fn all_rows_invalid_leaves_members_empty() {
        let csv = "ログオン名,名前,メモ\na/b,不正1,\nc:d,不正2,";
        let r = import_members_csv(csv.as_bytes());
        assert!(r.members.is_empty());
        assert_eq!(r.errors.len(), 2);
    }

    #[test]
    fn merge_adds_new_and_overwrites_existing() {
        let mut list = vec![member("taro", "旧名", "旧メモ")];
        let imported = vec![member("taro", "新名", "新メモ"), member("jiro", "次郎", "")];
        let (added, updated) = merge_members(&mut list, &imported);
        assert_eq!((added, updated), (1, 1));
        assert_eq!(list.len(), 2);
        assert_eq!(
            list.iter().find(|m| m.id == "taro").unwrap(),
            &member("taro", "新名", "新メモ")
        );
    }

    #[test]
    fn merge_duplicate_id_within_import_last_one_wins() {
        let mut list = Vec::new();
        let imported = vec![member("taro", "1回目", ""), member("taro", "2回目", "")];
        merge_members(&mut list, &imported);
        assert_eq!(list, vec![member("taro", "2回目", "")]);
    }
}
