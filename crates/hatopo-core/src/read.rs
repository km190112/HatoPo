//! 既読の印 (`read\{id}\<user>.json`)。利用者が「既読にする」を押したときに書き、
//! 「未読に戻す」で消す。管理画面はこれを読んで、誰がまだ読んでいないかを出す。
//!
//! 1 通知 × 1 人 = 1 ファイルなので、何人が同時に押してもぶつからない (ロックは要らない)。
//! 通知が archive へ移るとき (`Loft::close`) に読んだ人を archive へ写し、印のフォルダは消す。

use crate::error::io;
use crate::id::is_valid_id;
use crate::loft::{json_files, read_json, to_json, write_atomic};
use crate::model::{ReadMark, normalize_name};
use crate::{Error, Loft, Origin, Result};
use chrono::NaiveDateTime;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

impl Loft {
    fn read_dir_of(&self, id: &str) -> Result<PathBuf> {
        if !is_valid_id(id) {
            return Err(Error::Invalid(format!(
                "通知 ID「{id}」の形が正しくありません。"
            )));
        }
        Ok(self.root().join("read").join(id))
    }

    /// 既読にする。通知がもう無ければ (完了・削除・期限切れ) `NotFound`。
    pub fn mark_read(&self, id: &str, by: &Origin, now: NaiveDateTime) -> Result<ReadMark> {
        let user = normalize_name(&by.user)?;
        let dir = self.read_dir_of(id)?;
        self.locate(id)?;
        let mark = ReadMark {
            user: user.clone(),
            host: by.host.clone(),
            at: now,
        };
        fs::create_dir_all(&dir).map_err(io(&dir))?;
        write_atomic(&dir.join(format!("{user}.json")), &to_json(&mark)?)?;
        // 書いている間に他の人が完了させると、片付けた後のフォルダを作り直してしまう。残さない
        if let Err(Error::NotFound { .. }) = self.locate(id) {
            self.clear_reads(id);
            return Err(Error::NotFound { id: id.into() });
        }
        Ok(mark)
    }

    /// 未読に戻す。印が無ければ何もしない。
    pub fn unmark_read(&self, id: &str, user: &str) -> Result<()> {
        let dir = self.read_dir_of(id)?;
        let p = dir.join(format!("{}.json", normalize_name(user)?));
        match fs::remove_file(&p) {
            Ok(()) => {
                // 空になったフォルダは消す (中身が残っていれば失敗するだけ)
                let _ = fs::remove_dir(&dir);
                Ok(())
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Io { path: p, source: e }),
        }
    }

    /// この人が既読にしているか。共有に届かないときは `Err` (未読と取り違えない)。
    pub fn has_read(&self, id: &str, user: &str) -> Result<bool> {
        let p = self
            .read_dir_of(id)?
            .join(format!("{}.json", normalize_name(user)?));
        match fs::metadata(&p) {
            Ok(m) => Ok(m.is_file()),
            Err(e) if e.kind() == ErrorKind::NotFound => {
                self.check_online()?;
                Ok(false)
            }
            Err(e) => Err(Error::Io { path: p, source: e }),
        }
    }

    /// 既読にした人。古い順。壊れた印は飛ばす。
    pub fn readers(&self, id: &str) -> Result<Vec<ReadMark>> {
        let mut v: Vec<ReadMark> = json_files(&self.read_dir_of(id)?)?
            .iter()
            .filter_map(|p| read_json(p).ok())
            .collect();
        v.sort_by(|a, b| a.at.cmp(&b.at).then(a.user.cmp(&b.user)));
        Ok(v)
    }

    /// 印のフォルダごと消す。失敗しても困らない (次に閉じる人はいないので残るだけ)。
    pub(crate) fn clear_reads(&self, id: &str) {
        if let Ok(dir) = self.read_dir_of(id) {
            let _ = fs::remove_dir_all(dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{CloseReason, Error, Kind, Loft, NewNotice, Origin, Target};
    use chrono::{NaiveDate, NaiveDateTime};

    fn at(h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 2)
            .unwrap()
            .and_hms_opt(h, 0, 0)
            .unwrap()
    }

    fn who(user: &str, host: &str) -> Origin {
        Origin {
            user: user.into(),
            host: host.into(),
        }
    }

    fn broadcast(loft: &Loft) -> String {
        let n = NewNotice {
            title: "お知らせ".into(),
            ..Default::default()
        };
        loft.add(&Target::broadcast(), &n, &who("admin", "pc-admin"), at(8))
            .unwrap()
            .id
    }

    #[test]
    fn nobody_one_and_many_readers() {
        let d = tempfile::tempdir().unwrap();
        let loft = Loft::new(d.path());
        let id = broadcast(&loft);
        assert!(loft.readers(&id).unwrap().is_empty(), "0 人");
        assert!(!loft.has_read(&id, "taro").unwrap());

        loft.mark_read(&id, &who("Taro", "PC-1"), at(10)).unwrap();
        let one = loft.readers(&id).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!((one[0].user.as_str(), one[0].at), ("taro", at(10)));
        assert!(
            loft.has_read(&id, "TARO").unwrap(),
            "大文字小文字を問わない"
        );

        loft.mark_read(&id, &who("山田", "pc-2"), at(9)).unwrap();
        loft.mark_read(&id, &who("hanako", "pc-3"), at(11)).unwrap();
        let users: Vec<_> = loft
            .readers(&id)
            .unwrap()
            .into_iter()
            .map(|m| m.user)
            .collect();
        assert_eq!(users, ["山田", "taro", "hanako"], "古い順");
    }

    #[test]
    fn unmark_removes_only_that_user_and_is_quiet_when_absent() {
        let d = tempfile::tempdir().unwrap();
        let loft = Loft::new(d.path());
        let id = broadcast(&loft);
        loft.unmark_read(&id, "taro").unwrap();
        loft.mark_read(&id, &who("taro", "pc-1"), at(10)).unwrap();
        loft.mark_read(&id, &who("jiro", "pc-2"), at(10)).unwrap();
        loft.unmark_read(&id, "taro").unwrap();
        assert!(!loft.has_read(&id, "taro").unwrap());
        assert!(loft.has_read(&id, "jiro").unwrap());
        loft.unmark_read(&id, "jiro").unwrap();
        assert!(
            !d.path().join("read").join(&id).exists(),
            "空になったら片付ける"
        );
    }

    #[test]
    fn marking_a_notice_that_is_gone_is_not_found_and_leaves_nothing() {
        let d = tempfile::tempdir().unwrap();
        let loft = Loft::new(d.path());
        let id = broadcast(&loft);
        loft.close(&id, CloseReason::Deleted, &who("admin", "pc-admin"), at(9))
            .unwrap();
        assert!(matches!(
            loft.mark_read(&id, &who("taro", "pc-1"), at(10)),
            Err(Error::NotFound { .. })
        ));
        assert!(!d.path().join("read").join(&id).exists());
    }

    #[test]
    fn closing_copies_readers_to_archive_and_clears_marks() {
        let d = tempfile::tempdir().unwrap();
        let loft = Loft::new(d.path());
        let n = NewNotice {
            title: "図面".into(),
            ..Default::default()
        };
        let id = loft
            .add(
                &Target::new(Kind::User, "taro").unwrap(),
                &n,
                &who("admin", "pc-admin"),
                at(8),
            )
            .unwrap()
            .id;
        loft.mark_read(&id, &who("taro", "pc-1"), at(9)).unwrap();
        let a = loft
            .close(&id, CloseReason::Done, &who("taro", "pc-1"), at(10))
            .unwrap();
        assert_eq!(a.read_by.len(), 1);
        assert_eq!(a.read_by[0].user, "taro");
        assert!(!d.path().join("read").join(&id).exists());
        let back = loft.archived("2026-11").unwrap();
        assert_eq!(back[0].read_by, a.read_by, "archive に残る");
    }

    #[test]
    fn japanese_root_path_works() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("鳩の巣");
        std::fs::create_dir_all(&root).unwrap();
        let loft = Loft::new(&root);
        let id = broadcast(&loft);
        loft.mark_read(&id, &who("佐藤", "事務所"), at(10)).unwrap();
        assert!(loft.has_read(&id, "佐藤").unwrap());
        assert_eq!(loft.readers(&id).unwrap()[0].host, "事務所");
    }

    #[test]
    fn offline_is_an_error_not_unread() {
        let d = tempfile::tempdir().unwrap();
        let loft = Loft::new(d.path().join("無い"));
        assert!(loft.has_read("20261102-080000-0123456789", "taro").is_err());
    }

    #[test]
    fn bad_ids_and_names_are_rejected() {
        let d = tempfile::tempdir().unwrap();
        let loft = Loft::new(d.path());
        let id = broadcast(&loft);
        assert!(loft.mark_read("..\\x", &who("taro", "pc"), at(9)).is_err());
        assert!(loft.mark_read(&id, &who("a\\b", "pc"), at(9)).is_err());
        assert!(loft.has_read("..", "taro").is_err());
    }

    #[test]
    fn old_archive_without_read_by_is_readable() {
        let d = tempfile::tempdir().unwrap();
        let loft = Loft::new(d.path());
        let id = broadcast(&loft);
        let a = loft
            .close(&id, CloseReason::Deleted, &who("admin", "pc-admin"), at(9))
            .unwrap();
        assert!(a.read_by.is_empty());
        let text =
            std::fs::read_to_string(d.path().join("archive/2026-11").join(format!("{id}.json")))
                .unwrap();
        assert!(!text.contains("read_by"), "誰も読んでいなければ書かない");
        assert_eq!(loft.archived("2026-11").unwrap().len(), 1);
    }
}
