//! 鳩の巣 (共有フォルダ) の読み書き。
//!
//! ```text
//! <root>\
//!   broadcast\{id}.json
//!   PigeonHole\device\<host>\{id}.json
//!   PigeonHole\user\<user>\{id}.json
//!   archive\yyyy-MM\{id}.json
//! ```

use crate::error::io;
use crate::id::{is_valid_id, new_id, random_hex};
use crate::model::{
    Archived, CloseReason, Closed, Kind, NewNotice, Notice, NoticeEdit, Origin, SCHEMA, Target,
    normalize_name, validate,
};
use crate::{Error, Result};
use chrono::NaiveDateTime;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// これより古いロックファイルは、書いた端末が落ちた残骸とみなして消す。
const STALE_LOCK: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
pub struct Loft {
    root: PathBuf,
}

/// 読めなかった通知ファイル。一覧には出さず、件数と場所を知らせる。
#[derive(Clone, Debug, PartialEq)]
pub struct Broken {
    pub path: PathBuf,
    pub detail: String,
}

/// ある利用者・端末から見た通知。
#[derive(Clone, Debug, Default)]
pub struct Inbox {
    /// 表示する通知。一斉 → 端末 → ユーザーの順、同じ種類の中は新しい順。
    pub active: Vec<Notice>,
    /// 通知開始日がまだ来ていない件数。
    pub waiting: usize,
    /// 通知終了日を過ぎたもの。`Loft::sweep_expired` で archive へ移す。
    pub expired: Vec<Notice>,
    pub broken: Vec<Broken>,
}

/// 管理者の検索条件。指定しない項目は絞り込まない。
#[derive(Clone, Debug, Default)]
pub struct Filter {
    pub kind: Option<Kind>,
    pub name: Option<String>,
    /// 件名と本文に含まれる文字列 (大文字小文字を区別しない)。
    pub keyword: Option<String>,
}

impl Loft {
    pub fn new(root: impl Into<PathBuf>) -> Loft {
        Loft { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 共有フォルダに届くか。
    pub fn check_online(&self) -> Result<()> {
        match fs::metadata(&self.root) {
            Ok(m) if m.is_dir() => Ok(()),
            Ok(_) => Err(Error::Offline {
                root: self.root.clone(),
                detail: "フォルダではありません".into(),
            }),
            Err(e) => Err(Error::Offline {
                root: self.root.clone(),
                detail: e.to_string(),
            }),
        }
    }

    fn broadcast_dir(&self) -> PathBuf {
        self.root.join("broadcast")
    }

    fn kind_dir(&self, kind: Kind) -> PathBuf {
        match kind {
            Kind::Broadcast => self.broadcast_dir(),
            Kind::Device => self.root.join("PigeonHole").join("device"),
            Kind::User => self.root.join("PigeonHole").join("user"),
        }
    }

    fn dir_for(&self, t: &Target) -> PathBuf {
        match t.kind {
            Kind::Broadcast => self.broadcast_dir(),
            k => self.kind_dir(k).join(&t.name),
        }
    }

    fn archive_dir(&self, now: NaiveDateTime) -> PathBuf {
        self.root
            .join("archive")
            .join(now.format("%Y-%m").to_string())
    }

    /// 通知を 1 件作る。宛先のフォルダが無ければ作る。
    pub fn add(
        &self,
        target: &Target,
        n: &NewNotice,
        by: &Origin,
        now: NaiveDateTime,
    ) -> Result<Notice> {
        self.check_online()?;
        let (title, urls) = validate(n)?;
        let dir = self.dir_for(target);
        fs::create_dir_all(&dir).map_err(io(&dir))?;
        // rename は上書きするので、万一 ID が重なったら作り直す
        let id = loop {
            let id = new_id(now);
            if !dir.join(format!("{id}.json")).exists() {
                break id;
            }
        };
        let notice = Notice {
            schema: SCHEMA,
            id,
            revision: 1,
            target: target.clone(),
            title,
            body: n.body.clone(),
            due: n.due,
            urls,
            created_at: now,
            created_by: by.clone(),
            start_at: n.start_at.unwrap_or(now),
            end_at: n.end_at,
            updated_at: None,
            updated_by: None,
        };
        write_atomic(&dir.join(format!("{}.json", notice.id)), &to_json(&notice)?)?;
        Ok(notice)
    }

    /// ID から通知を探す。
    pub fn get(&self, id: &str) -> Result<Notice> {
        read_notice(&self.locate(id)?)
    }

    /// 件名・本文・期限・リンク・開始・終了を書き換える (宛先は変えられない)。
    ///
    /// `expected_revision` は読んだときの `revision`。他の人が先に変えていたら `Conflict`。
    /// 0 を渡すと版を確かめずに上書きする。`start_at` が `None` なら今の開始日時を保つ。
    pub fn update(
        &self,
        id: &str,
        expected_revision: u64,
        edit: &NoticeEdit,
        by: &Origin,
        now: NaiveDateTime,
    ) -> Result<Notice> {
        let path = self.locate(id)?;
        let _lock = Lock::acquire(&path, id)?;
        let mut n = read_notice(&path)?;
        if expected_revision != 0 && n.revision != expected_revision {
            return Err(Error::Conflict {
                id: id.into(),
                expected: expected_revision,
                actual: n.revision,
            });
        }
        let start_at = edit.start_at.unwrap_or(n.start_at);
        let (title, urls) = validate(&NewNotice {
            start_at: Some(start_at),
            ..edit.clone()
        })?;
        n.title = title;
        n.body = edit.body.clone();
        n.due = edit.due;
        n.urls = urls;
        n.start_at = start_at;
        n.end_at = edit.end_at;
        n.revision += 1;
        n.updated_at = Some(now);
        n.updated_by = Some(by.clone());
        write_atomic(&path, &to_json(&n)?)?;
        Ok(n)
    }

    /// 通知を閉じて `archive\yyyy-MM\` へ移す (完了・期限切れ・削除)。
    ///
    /// 先に元のファイルを archive へ rename して「取った」ことにする。
    /// 複数の端末が同時に閉じようとしても、勝つのは 1 台で、他は `NotFound` か `Locked` になる。
    pub fn close(
        &self,
        id: &str,
        reason: CloseReason,
        by: &Origin,
        now: NaiveDateTime,
    ) -> Result<Archived> {
        let path = self.locate(id)?;
        let _lock = Lock::acquire(&path, id)?;
        let notice = read_notice(&path)?;
        let dir = self.archive_dir(now);
        fs::create_dir_all(&dir).map_err(io(&dir))?;
        let dest = dir.join(format!("{id}.json"));
        fs::rename(&path, &dest).map_err(|e| match e.kind() {
            ErrorKind::NotFound => Error::NotFound { id: id.into() },
            _ => Error::Io {
                path: path.clone(),
                source: e,
            },
        })?;
        let archived = Archived {
            notice,
            closed: Closed {
                reason,
                by: by.clone(),
                at: now,
            },
        };
        write_atomic(&dest, &to_json(&archived)?)?;
        Ok(archived)
    }

    /// この利用者・端末宛ての通知を集める。共有に届かなければ `Offline`。
    pub fn inbox(&self, user: &str, host: &str, now: NaiveDateTime) -> Result<Inbox> {
        self.check_online()?;
        let mut dirs = vec![self.broadcast_dir()];
        // 環境変数が取れない等で名前が不正なら、その宛先は見ない
        if let Ok(h) = normalize_name(host) {
            dirs.push(self.kind_dir(Kind::Device).join(h));
        }
        if let Ok(u) = normalize_name(user) {
            dirs.push(self.kind_dir(Kind::User).join(u));
        }
        let mut inbox = Inbox::default();
        for d in dirs {
            let (notices, broken) = scan_dir(&d)?;
            inbox.broken.extend(broken);
            for n in notices {
                if n.is_expired(now) {
                    inbox.expired.push(n);
                } else if !n.is_started(now) {
                    inbox.waiting += 1;
                } else {
                    inbox.active.push(n);
                }
            }
        }
        sort_for_display(&mut inbox.active);
        Ok(inbox)
    }

    /// 期限切れを archive へ移す。他の端末が先に移した・移している最中のものは黙って飛ばす。
    /// それ以外の失敗だけを返す。
    pub fn sweep_expired(&self, expired: &[Notice], by: &Origin, now: NaiveDateTime) -> Vec<Error> {
        expired
            .iter()
            .filter_map(|n| match self.close(&n.id, CloseReason::Expired, by, now) {
                Ok(_) | Err(Error::NotFound { .. }) | Err(Error::Locked { .. }) => None,
                Err(e) => Some(e),
            })
            .collect()
    }

    /// 全宛先から検索する (管理者用)。開始前・期限切れも含む。
    pub fn find(&self, f: &Filter) -> Result<(Vec<Notice>, Vec<Broken>)> {
        self.check_online()?;
        let name = f.name.as_deref().map(normalize_name).transpose()?;
        let kw = f
            .keyword
            .as_deref()
            .map(str::to_lowercase)
            .filter(|k| !k.is_empty());
        let mut out = Vec::new();
        let mut broken = Vec::new();
        for d in self.all_dirs()? {
            let (notices, b) = scan_dir(&d)?;
            broken.extend(b);
            out.extend(notices.into_iter().filter(|n| {
                f.kind.is_none_or(|k| n.target.kind == k)
                    && name.as_ref().is_none_or(|x| &n.target.name == x)
                    && kw.as_ref().is_none_or(|k| {
                        n.title.to_lowercase().contains(k) || n.body.to_lowercase().contains(k)
                    })
            }));
        }
        sort_for_display(&mut out);
        Ok((out, broken))
    }

    /// `archive\yyyy-MM\` の中身 (管理者が「誰が完了したか」を見る用)。
    pub fn archived(&self, month: &str) -> Result<Vec<Archived>> {
        self.check_online()?;
        if !(month.len() == 7
            && month
                .chars()
                .enumerate()
                .all(|(i, c)| if i == 4 { c == '-' } else { c.is_ascii_digit() }))
        {
            return Err(Error::Invalid(format!(
                "月は 2026-10 の形で指定してください (「{month}」)。"
            )));
        }
        let mut out: Vec<Archived> = Vec::new();
        for p in json_files(&self.root.join("archive").join(month))? {
            if let Ok(a) = read_json::<Archived>(&p) {
                out.push(a);
            }
        }
        out.sort_by_key(|a| a.closed.at);
        Ok(out)
    }

    /// broadcast と PigeonHole の下の宛先フォルダすべて。
    fn all_dirs(&self) -> Result<Vec<PathBuf>> {
        let mut dirs = vec![self.broadcast_dir()];
        for k in [Kind::Device, Kind::User] {
            let base = self.kind_dir(k);
            match fs::read_dir(&base) {
                Ok(rd) => {
                    for e in rd.flatten() {
                        if e.file_type().is_ok_and(|t| t.is_dir()) {
                            dirs.push(e.path());
                        }
                    }
                }
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(Error::Io {
                        path: base,
                        source: e,
                    });
                }
            }
        }
        Ok(dirs)
    }

    fn locate(&self, id: &str) -> Result<PathBuf> {
        if !is_valid_id(id) {
            return Err(Error::Invalid(format!(
                "通知 ID「{id}」の形が正しくありません。"
            )));
        }
        self.check_online()?;
        let file = format!("{id}.json");
        for d in self.all_dirs()? {
            let p = d.join(&file);
            if p.is_file() {
                return Ok(p);
            }
        }
        Err(Error::NotFound { id: id.into() })
    }
}

fn sort_for_display(v: &mut [Notice]) {
    v.sort_by(|a, b| {
        a.target
            .kind
            .cmp(&b.target.kind)
            .then(b.start_at.cmp(&a.start_at))
            .then(b.id.cmp(&a.id))
    });
}

/// `*.json` だけ。`.` で始まる一時ファイル・ロックファイルは除く。フォルダが無ければ空。
fn json_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(Error::Io {
                path: dir.into(),
                source: e,
            });
        }
    };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            !name.starts_with('.') && name.to_ascii_lowercase().ends_with(".json") && p.is_file()
        })
        .collect();
    v.sort();
    Ok(v)
}

fn scan_dir(dir: &Path) -> Result<(Vec<Notice>, Vec<Broken>)> {
    let mut ok = Vec::new();
    let mut broken = Vec::new();
    for p in json_files(dir)? {
        match read_notice(&p) {
            Ok(n) => ok.push(n),
            // 一覧した直後に他の端末が完了させた
            Err(Error::NotFound { .. }) => {}
            Err(e) => broken.push(Broken {
                path: p,
                detail: e.to_string(),
            }),
        }
    }
    Ok((ok, broken))
}

fn read_notice(path: &Path) -> Result<Notice> {
    read_json(path)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).map_err(|e| match e.kind() {
        ErrorKind::NotFound => Error::NotFound {
            id: path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
        },
        _ => Error::Io {
            path: path.into(),
            source: e,
        },
    })?;
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    serde_json::from_slice(bytes).map_err(|e| Error::Broken {
        path: path.into(),
        detail: e.to_string(),
    })
}

fn to_json<T: serde::Serialize>(v: &T) -> Result<Vec<u8>> {
    let mut s = serde_json::to_vec_pretty(v).map_err(|e| Error::Invalid(e.to_string()))?;
    s.push(b'\n');
    Ok(s)
}

/// 同じフォルダの一時ファイルに書いてから rename で置き換える。
/// 読み手は古い中身か新しい中身のどちらかしか見ない (書きかけを見ない)。
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let stem = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{stem}.{}.tmp", random_hex(8)));
    let result = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(Error::Io {
            path: path.into(),
            source: e,
        });
    }
    Ok(())
}

/// `.{id}.lock` を排他作成して持つ。落ちたら Drop で消す。
struct Lock(PathBuf);

impl Lock {
    fn acquire(notice_path: &Path, id: &str) -> Result<Lock> {
        let path = notice_path.with_file_name(format!(".{id}.lock"));
        for attempt in 0..2 {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    let o = Origin::current();
                    let _ = writeln!(f, "{}@{}", o.user, o.host);
                    return Ok(Lock(path));
                }
                Err(e)
                    if e.kind() == ErrorKind::AlreadyExists && attempt == 0 && is_stale(&path) =>
                {
                    let _ = fs::remove_file(&path);
                }
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                    return Err(Error::Locked { id: id.into() });
                }
                // SMB では排他作成の競合が PermissionDenied で返ることがある
                Err(e) if e.kind() == ErrorKind::PermissionDenied && path.exists() => {
                    return Err(Error::Locked { id: id.into() });
                }
                Err(e) => return Err(Error::Io { path, source: e }),
            }
        }
        Err(Error::Locked { id: id.into() })
    }
}

fn is_stale(path: &Path) -> bool {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age > STALE_LOCK)
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(d: u32, h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, d)
            .unwrap()
            .and_hms_opt(h, 0, 0)
            .unwrap()
    }

    fn admin() -> Origin {
        Origin {
            user: "hanako".into(),
            host: "pc-admin".into(),
        }
    }

    fn setup() -> (tempfile::TempDir, Loft) {
        let dir = tempfile::tempdir().unwrap();
        let loft = Loft::new(dir.path());
        (dir, loft)
    }

    fn note(title: &str) -> NewNotice {
        NewNotice {
            title: title.into(),
            ..Default::default()
        }
    }

    fn user(n: &str) -> Target {
        Target::new(Kind::User, n).unwrap()
    }

    fn titles(v: &[Notice]) -> Vec<&str> {
        v.iter().map(|n| n.title.as_str()).collect()
    }

    // --- 件数の地雷: 0 件 / 1 件 / 複数件 ---

    #[test]
    fn empty_loft_gives_empty_inbox() {
        let (_d, loft) = setup();
        let ib = loft.inbox("taro", "pc-1", at(1, 9)).unwrap();
        assert!(ib.active.is_empty() && ib.expired.is_empty() && ib.broken.is_empty());
        assert_eq!(ib.waiting, 0);
    }

    #[test]
    fn one_notice_round_trips_with_japanese() {
        let (d, loft) = setup();
        let n = NewNotice {
            title: "工数表の入力".into(),
            body: "10月分をお願いします。\n絵文字も 🕊️".into(),
            due: NaiveDate::from_ymd_opt(2026, 11, 5),
            urls: vec![r"\\server\共有\工数表.xlsm".into()],
            ..Default::default()
        };
        let made = loft.add(&user("Taro"), &n, &admin(), at(1, 9)).unwrap();
        assert!(
            d.path()
                .join("PigeonHole/user/taro")
                .join(format!("{}.json", made.id))
                .is_file()
        );

        let ib = loft.inbox("TARO", "pc-1", at(1, 9)).unwrap();
        assert_eq!(ib.active, vec![made.clone()]);
        assert_eq!(loft.get(&made.id).unwrap(), made);
    }

    #[test]
    fn many_notices_sorted_broadcast_device_user_then_newest_first() {
        let (_d, loft) = setup();
        let o = admin();
        loft.add(&user("taro"), &note("u-old"), &o, at(1, 9))
            .unwrap();
        loft.add(&user("taro"), &note("u-new"), &o, at(2, 9))
            .unwrap();
        loft.add(
            &Target::new(Kind::Device, "PC-1").unwrap(),
            &note("d"),
            &o,
            at(1, 9),
        )
        .unwrap();
        loft.add(&Target::broadcast(), &note("b"), &o, at(1, 8))
            .unwrap();
        // 他人宛て・他の端末宛ては出ない
        loft.add(&user("jiro"), &note("other"), &o, at(1, 9))
            .unwrap();
        loft.add(
            &Target::new(Kind::Device, "pc-2").unwrap(),
            &note("other-pc"),
            &o,
            at(1, 9),
        )
        .unwrap();

        let ib = loft.inbox("taro", "PC-1", at(3, 9)).unwrap();
        assert_eq!(titles(&ib.active), ["b", "d", "u-new", "u-old"]);
    }

    #[test]
    fn japanese_user_folder_works() {
        let (d, loft) = setup();
        loft.add(&user("山田"), &note("x"), &admin(), at(1, 9))
            .unwrap();
        assert!(d.path().join("PigeonHole/user/山田").is_dir());
        assert_eq!(loft.inbox("山田", "pc", at(1, 9)).unwrap().active.len(), 1);
    }

    #[test]
    fn japanese_root_path_works() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("鳩の巣");
        fs::create_dir(&root).unwrap();
        let loft = Loft::new(&root);
        let n = loft
            .add(&user("taro"), &note("x"), &admin(), at(1, 9))
            .unwrap();
        loft.close(&n.id, CloseReason::Done, &admin(), at(1, 10))
            .unwrap();
        assert_eq!(loft.archived("2026-11").unwrap().len(), 1);
    }

    // --- 開始日・終了日 ---

    #[test]
    fn not_started_is_hidden_and_counted() {
        let (_d, loft) = setup();
        let n = NewNotice {
            start_at: Some(at(5, 0)),
            ..note("later")
        };
        loft.add(&user("taro"), &n, &admin(), at(1, 9)).unwrap();
        let before = loft.inbox("taro", "pc", at(4, 23)).unwrap();
        assert!(before.active.is_empty());
        assert_eq!(before.waiting, 1);
        assert_eq!(loft.inbox("taro", "pc", at(5, 0)).unwrap().active.len(), 1);
    }

    #[test]
    fn expired_goes_to_archive_with_reason() {
        let (d, loft) = setup();
        let n = NewNotice {
            end_at: Some(at(3, 17)),
            ..note("old")
        };
        let made = loft
            .add(&Target::broadcast(), &n, &admin(), at(1, 9))
            .unwrap();
        assert_eq!(
            loft.inbox("taro", "pc", at(3, 17)).unwrap().active.len(),
            1,
            "終了時刻ちょうどはまだ出す"
        );

        let me = Origin {
            user: "taro".into(),
            host: "pc".into(),
        };
        let ib = loft.inbox("taro", "pc", at(3, 18)).unwrap();
        assert!(ib.active.is_empty());
        assert_eq!(ib.expired.len(), 1);
        assert!(loft.sweep_expired(&ib.expired, &me, at(3, 18)).is_empty());

        let arch = loft.archived("2026-11").unwrap();
        assert_eq!(arch.len(), 1);
        assert_eq!(arch[0].notice, made);
        assert_eq!(arch[0].closed.reason, CloseReason::Expired);
        assert!(
            !d.path()
                .join("broadcast")
                .join(format!("{}.json", made.id))
                .exists()
        );
    }

    #[test]
    fn sweeping_twice_from_two_pcs_is_silent() {
        let (_d, loft) = setup();
        let n = NewNotice {
            end_at: Some(at(2, 0)),
            ..note("x")
        };
        loft.add(&Target::broadcast(), &n, &admin(), at(1, 9))
            .unwrap();
        let a = loft.inbox("a", "pc-a", at(3, 0)).unwrap();
        let b = loft.inbox("b", "pc-b", at(3, 0)).unwrap();
        assert!(
            loft.sweep_expired(&a.expired, &admin(), at(3, 0))
                .is_empty()
        );
        assert!(
            loft.sweep_expired(&b.expired, &admin(), at(3, 0))
                .is_empty(),
            "後から来た端末は既に無いので黙る"
        );
        assert_eq!(loft.archived("2026-11").unwrap().len(), 1);
    }

    // --- 完了・変更 ---

    #[test]
    fn complete_moves_to_monthly_archive_with_who_and_when() {
        let (d, loft) = setup();
        let made = loft
            .add(&user("taro"), &note("x"), &admin(), at(1, 9))
            .unwrap();
        let me = Origin {
            user: "taro".into(),
            host: "pc-1".into(),
        };
        let a = loft
            .close(&made.id, CloseReason::Done, &me, at(2, 10))
            .unwrap();
        assert_eq!(
            a.closed,
            Closed {
                reason: CloseReason::Done,
                by: me,
                at: at(2, 10)
            }
        );
        assert!(
            d.path()
                .join("archive/2026-11")
                .join(format!("{}.json", made.id))
                .is_file()
        );
        assert!(
            loft.inbox("taro", "pc-1", at(2, 10))
                .unwrap()
                .active
                .is_empty()
        );
        assert!(matches!(
            loft.close(&made.id, CloseReason::Done, &admin(), at(2, 11)),
            Err(Error::NotFound { .. })
        ));
    }

    #[test]
    fn update_bumps_revision_and_detects_conflict() {
        let (_d, loft) = setup();
        let made = loft
            .add(&user("taro"), &note("v1"), &admin(), at(1, 9))
            .unwrap();
        let v2 = loft
            .update(&made.id, 1, &note("v2"), &admin(), at(1, 10))
            .unwrap();
        assert_eq!(
            (v2.revision, v2.title.as_str(), v2.start_at),
            (2, "v2", made.start_at)
        );
        assert_eq!(v2.updated_by, Some(admin()));

        // 版 1 を読んだまま書こうとした人は弾かれ、ファイルは v2 のまま
        let err = loft
            .update(&made.id, 1, &note("stale"), &admin(), at(1, 11))
            .unwrap_err();
        assert!(
            matches!(
                err,
                Error::Conflict {
                    expected: 1,
                    actual: 2,
                    ..
                }
            ),
            "{err}"
        );
        assert_eq!(loft.get(&made.id).unwrap().title, "v2");

        // 0 は版を確かめない
        assert_eq!(
            loft.update(&made.id, 0, &note("forced"), &admin(), at(1, 12))
                .unwrap()
                .revision,
            3
        );
    }

    #[test]
    fn update_while_locked_is_refused_and_stale_lock_is_cleared() {
        let (d, loft) = setup();
        let made = loft
            .add(&user("taro"), &note("x"), &admin(), at(1, 9))
            .unwrap();
        let lock = d
            .path()
            .join("PigeonHole/user/taro")
            .join(format!(".{}.lock", made.id));
        fs::write(&lock, "jiro@pc-2").unwrap();
        assert!(matches!(
            loft.update(&made.id, 1, &note("y"), &admin(), at(1, 10)),
            Err(Error::Locked { .. })
        ));

        // 落ちた端末の残骸 (古いロック) は消して進む
        let old = std::time::SystemTime::now() - Duration::from_secs(600);
        fs::File::options()
            .write(true)
            .open(&lock)
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(
            loft.update(&made.id, 1, &note("y"), &admin(), at(1, 10))
                .is_ok()
        );
        assert!(!lock.exists(), "使い終わったロックは消える");
    }

    // --- 外部入力の異常 ---

    #[test]
    fn broken_and_foreign_files_do_not_stop_the_inbox() {
        let (d, loft) = setup();
        loft.add(&user("taro"), &note("good"), &admin(), at(1, 9))
            .unwrap();
        let dir = d.path().join("PigeonHole/user/taro");
        fs::write(dir.join("bad.json"), "{ not json").unwrap();
        fs::write(dir.join("memo.txt"), "管理者のメモ").unwrap();
        fs::write(dir.join(".x.json.abcd.tmp"), "{").unwrap();
        fs::create_dir(dir.join("sub.json")).unwrap();

        let ib = loft.inbox("taro", "pc", at(1, 9)).unwrap();
        assert_eq!(titles(&ib.active), ["good"]);
        assert_eq!(ib.broken.len(), 1);
        assert!(ib.broken[0].path.ends_with("bad.json"));
    }

    #[test]
    fn notice_saved_by_notepad_with_bom_is_readable() {
        let (d, loft) = setup();
        let made = loft
            .add(&user("taro"), &note("x"), &admin(), at(1, 9))
            .unwrap();
        let p = d
            .path()
            .join("PigeonHole/user/taro")
            .join(format!("{}.json", made.id));
        let mut bytes = b"\xEF\xBB\xBF".to_vec();
        bytes.extend(fs::read(&p).unwrap());
        fs::write(&p, bytes).unwrap();
        assert_eq!(loft.get(&made.id).unwrap(), made);
    }

    #[test]
    fn offline_root_is_reported_as_offline() {
        let loft = Loft::new(r"\\no-such-server-hatopo\HatoLoft");
        assert!(matches!(
            loft.inbox("taro", "pc", at(1, 9)),
            Err(Error::Offline { .. })
        ));
        assert!(matches!(
            loft.add(&user("taro"), &note("x"), &admin(), at(1, 9)),
            Err(Error::Offline { .. })
        ));
    }

    #[test]
    fn bad_ids_are_rejected_before_touching_files() {
        let (_d, loft) = setup();
        for id in ["..\\..\\x", "a/b", ""] {
            assert!(matches!(loft.get(id), Err(Error::Invalid(_))), "{id}");
        }
        assert!(matches!(
            loft.get("20260101-000000-abcdef"),
            Err(Error::NotFound { .. })
        ));
    }

    #[test]
    fn empty_env_names_skip_that_folder() {
        let (_d, loft) = setup();
        loft.add(&Target::broadcast(), &note("b"), &admin(), at(1, 9))
            .unwrap();
        assert_eq!(titles(&loft.inbox("", "", at(1, 9)).unwrap().active), ["b"]);
    }

    // --- 検索 ---

    #[test]
    fn find_filters_by_kind_name_and_keyword() {
        let (_d, loft) = setup();
        let o = admin();
        loft.add(
            &user("taro"),
            &NewNotice {
                body: "工数を入力".into(),
                ..note("月末")
            },
            &o,
            at(1, 9),
        )
        .unwrap();
        loft.add(&user("jiro"), &note("Update please"), &o, at(1, 9))
            .unwrap();
        loft.add(
            &Target::new(Kind::Device, "pc-9").unwrap(),
            &note("更新"),
            &o,
            at(1, 9),
        )
        .unwrap();

        let all = loft.find(&Filter::default()).unwrap().0;
        assert_eq!(all.len(), 3);
        let f = |kind, name: Option<&str>, kw: Option<&str>| {
            let r = loft
                .find(&Filter {
                    kind,
                    name: name.map(Into::into),
                    keyword: kw.map(Into::into),
                })
                .unwrap()
                .0;
            r.into_iter().map(|n| n.title).collect::<Vec<_>>()
        };
        assert_eq!(f(Some(Kind::Device), None, None), ["更新"]);
        assert_eq!(f(None, Some("TARO"), None), ["月末"]);
        assert_eq!(f(None, None, Some("工数")), ["月末"], "本文も検索する");
        assert_eq!(
            f(None, None, Some("update")),
            ["Update please"],
            "大文字小文字を区別しない"
        );
    }

    #[test]
    fn archived_rejects_bad_month() {
        let (_d, loft) = setup();
        assert!(loft.archived("2026-11").unwrap().is_empty());
        assert!(matches!(loft.archived("../x"), Err(Error::Invalid(_))));
    }
}
