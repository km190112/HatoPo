use crate::{Error, Result};
use chrono::{NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};

/// 通知ファイルの書式の版。互換の無い変更をしたら上げる。
pub const SCHEMA: u32 = 1;
/// リンクは要件どおり 3 つまで。
pub const MAX_URLS: usize = 3;

/// 宛先の種類。並び順がそのまま一覧の表示順 (一斉 → 端末 → ユーザー)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Broadcast,
    Device,
    User,
}

impl Kind {
    pub fn parse(s: &str) -> Result<Kind> {
        match s.trim().to_ascii_lowercase().as_str() {
            "broadcast" | "all" | "一斉" => Ok(Kind::Broadcast),
            "device" | "pc" | "端末" => Ok(Kind::Device),
            "user" | "ユーザー" => Ok(Kind::User),
            other => Err(Error::Invalid(format!(
                "宛先の種類「{other}」が分かりません。broadcast / device / user のどれかにしてください。"
            ))),
        }
    }

    pub fn label_ja(self) -> &'static str {
        match self {
            Kind::Broadcast => "一斉",
            Kind::Device => "この PC",
            Kind::User => "あなた",
        }
    }
}

/// 宛先。`name` はユーザー名かコンピューター名で、小文字に正規化して持つ
/// (Windows はどちらも大文字小文字を区別しないため)。broadcast では空。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

impl Target {
    pub fn broadcast() -> Target {
        Target {
            kind: Kind::Broadcast,
            name: String::new(),
        }
    }

    pub fn new(kind: Kind, name: &str) -> Result<Target> {
        if kind == Kind::Broadcast {
            return Ok(Target::broadcast());
        }
        Ok(Target {
            kind,
            name: normalize_name(name)?,
        })
    }

    /// `user:taro` / `device:PC-0123` / `broadcast` の形を読む (CLI 用)。
    pub fn parse(s: &str) -> Result<Target> {
        match s.split_once(':') {
            Some((k, n)) => Target::new(Kind::parse(k)?, n),
            None => Target::new(Kind::parse(s)?, ""),
        }
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            Kind::Broadcast => write!(f, "broadcast"),
            Kind::Device => write!(f, "device:{}", self.name),
            Kind::User => write!(f, "user:{}", self.name),
        }
    }
}

/// フォルダ名に使うので、パスを壊す文字を通さない。日本語名は通す。
pub(crate) fn normalize_name(name: &str) -> Result<String> {
    let n = name.trim().to_lowercase();
    let bad = n.is_empty()
        || n == "."
        || n == ".."
        || n.chars().count() > 64
        || n.chars()
            .any(|c| c.is_control() || r#"\/:*?"<>|"#.contains(c));
    if bad {
        return Err(Error::Invalid(format!(
            "宛先名「{name}」は使えません。ユーザー名かコンピューター名を入れてください (\\ / : * ? \" < > | は不可)。"
        )));
    }
    Ok(n)
}

/// 誰が・どの PC から操作したか。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    pub user: String,
    pub host: String,
}

impl Origin {
    /// `%USERNAME%` と `%COMPUTERNAME%`。
    pub fn current() -> Origin {
        let get = |k: &str| std::env::var(k).unwrap_or_default().to_lowercase();
        Origin {
            user: get("USERNAME"),
            host: get("COMPUTERNAME"),
        }
    }
}

/// 通知ファイル 1 つの中身。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    pub schema: u32,
    pub id: String,
    /// 変更のたびに 1 増える。同時更新の検出に使う。
    pub revision: u64,
    pub target: Target,
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// 依頼期限。表示するだけで、通知の出し入れには使わない。
    #[serde(default)]
    pub due: Option<NaiveDate>,
    #[serde(default)]
    pub urls: Vec<String>,
    pub created_at: NaiveDateTime,
    pub created_by: Origin,
    /// この時刻より前は出さない。
    pub start_at: NaiveDateTime,
    /// この時刻を過ぎたら出さずに archive へ移す。`None` は完了まで出し続ける。
    #[serde(default)]
    pub end_at: Option<NaiveDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<NaiveDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_by: Option<Origin>,
    /// 複数の宛先へまとめて送ったときの組の ID。同じ組の完了状況をまとめて見るのに使う
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

impl Notice {
    pub fn is_started(&self, now: NaiveDateTime) -> bool {
        self.start_at <= now
    }

    pub fn is_expired(&self, now: NaiveDateTime) -> bool {
        self.end_at.is_some_and(|e| e < now)
    }

    /// 利用者が完了を押せるか。一斉送信は管理者が消すまで残す。
    pub fn can_complete(&self) -> bool {
        self.target.kind != Kind::Broadcast
    }
}

/// 新しく作る通知の入力。
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct NewNotice {
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub due: Option<NaiveDate>,
    #[serde(default)]
    pub urls: Vec<String>,
    /// `None` は今すぐ。
    #[serde(default)]
    pub start_at: Option<NaiveDateTime>,
    #[serde(default)]
    pub end_at: Option<NaiveDateTime>,
}

/// 変更できる項目。宛先は変えられない (変えるなら削除して作り直す)。
pub type NoticeEdit = NewNotice;

pub(crate) fn validate(n: &NewNotice) -> Result<(String, Vec<String>)> {
    let title = n.title.trim().to_string();
    if title.is_empty() {
        return Err(Error::Invalid("通知内容 (件名) が空です。".into()));
    }
    let urls: Vec<String> = n
        .urls
        .iter()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
        .collect();
    if urls.len() > MAX_URLS {
        return Err(Error::Invalid(format!(
            "リンクは {MAX_URLS} つまでです ({} 個渡されました)。",
            urls.len()
        )));
    }
    if let (Some(s), Some(e)) = (n.start_at, n.end_at) {
        if e < s {
            return Err(Error::Invalid(format!(
                "通知終了日 ({e}) が通知開始日 ({s}) より前です。"
            )));
        }
    }
    Ok((title, urls))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CloseReason {
    /// 利用者が完了を押した。
    Done,
    /// 通知終了日を過ぎた。
    Expired,
    /// 管理者が削除した。
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Closed {
    pub reason: CloseReason,
    pub by: Origin,
    pub at: NaiveDateTime,
}

/// archive に置く形。通知の中身に「いつ誰がなぜ閉じたか」を足したもの。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Archived {
    #[serde(flatten)]
    pub notice: Notice,
    pub closed: Closed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_lowercased_and_japanese_is_allowed() {
        assert_eq!(normalize_name("  Taro ").unwrap(), "taro");
        assert_eq!(normalize_name("PC-0123").unwrap(), "pc-0123");
        assert_eq!(normalize_name("山田").unwrap(), "山田");
    }

    #[test]
    fn names_that_break_paths_are_rejected() {
        for bad in [
            "", " ", ".", "..", "a\\b", "a/b", "c:", "a*b", "a?b", "a|b", "a\"b", "a<b", "x\ty",
        ] {
            assert!(normalize_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn target_parse() {
        assert_eq!(
            Target::parse("user:Taro").unwrap(),
            Target {
                kind: Kind::User,
                name: "taro".into()
            }
        );
        assert_eq!(
            Target::parse("device:PC-1").unwrap().to_string(),
            "device:pc-1"
        );
        assert_eq!(Target::parse("broadcast").unwrap(), Target::broadcast());
        assert!(Target::parse("user:").is_err());
        assert!(Target::parse("group:x").is_err());
    }

    #[test]
    fn kind_order_is_display_priority() {
        let mut v = vec![Kind::User, Kind::Broadcast, Kind::Device];
        v.sort();
        assert_eq!(v, [Kind::Broadcast, Kind::Device, Kind::User]);
    }

    #[test]
    fn validate_rejects_empty_title_too_many_urls_and_reversed_dates() {
        let ok = NewNotice {
            title: " 工数入力 ".into(),
            urls: vec!["a".into(), " ".into()],
            ..Default::default()
        };
        assert_eq!(
            validate(&ok).unwrap(),
            ("工数入力".to_string(), vec!["a".to_string()])
        );

        assert!(
            validate(&NewNotice {
                title: "  ".into(),
                ..Default::default()
            })
            .is_err()
        );
        let many = NewNotice {
            title: "x".into(),
            urls: vec!["1".into(), "2".into(), "3".into(), "4".into()],
            ..Default::default()
        };
        assert!(validate(&many).is_err());
        let d = |day| {
            chrono::NaiveDate::from_ymd_opt(2026, 11, day)
                .unwrap()
                .and_hms_opt(0, 0, 0)
        };
        let rev = NewNotice {
            title: "x".into(),
            start_at: d(5),
            end_at: d(4),
            ..Default::default()
        };
        assert!(validate(&rev).is_err());
    }
}
