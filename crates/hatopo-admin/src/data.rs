//! 鳩の巣から読んだもの一式と、名簿の一覧を組み立てる計算 (画面から切り離してテストする)。

use chrono::NaiveDateTime;
use hatopo_core::{
    Broken, ClientStatus, Directory, Filter, Kind, Loft, Member, Notice, Settings, read_all_status,
};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Data {
    pub notices: Vec<Notice>,
    pub broken: Vec<Broken>,
    pub directory: Directory,
    /// 名簿が壊れていて読めないとき。保存すると消えてしまうので編集を止める
    pub directory_error: Option<String>,
    pub statuses: Vec<ClientStatus>,
    pub settings: Settings,
    pub settings_warning: Option<String>,
    pub months: Vec<String>,
    pub error: Option<String>,
}

impl Data {
    pub fn load(loft: &Loft) -> Data {
        let mut d = Data::default();
        if let Err(e) = loft.check_online() {
            d.error = Some(e.to_string());
            return d;
        }
        match loft.find(&Filter::default()) {
            Ok((n, b)) => {
                d.notices = n;
                d.broken = b;
            }
            Err(e) => d.error = Some(e.to_string()),
        }
        match Directory::load(loft.root()) {
            Ok(dir) => d.directory = dir,
            Err(e) => d.directory_error = Some(e.to_string()),
        }
        d.statuses = read_all_status(loft.root()).unwrap_or_default();
        let (s, w) = Settings::load(loft.root());
        d.settings = s;
        d.settings_warning = w;
        d.months = loft.archive_months().unwrap_or_default();
        d
    }

    /// 宛先の表示名 (`taro (山田 太郎)`)。名簿に無ければ ID だけ。
    pub fn label(&self, kind: Kind, id: &str) -> String {
        if kind == Kind::Broadcast {
            return "全員".into();
        }
        match self.directory.name_of(kind, id) {
            Some(name) => format!("{id} ({name})"),
            None => id.to_string(),
        }
    }
}

/// 名簿の画面の 1 行。
#[derive(Clone, Debug, PartialEq)]
pub struct MemberRow {
    pub member: Member,
    /// 名簿に載っているか (false は HatoPo の生存印だけがある = 名簿に足す候補)
    pub registered: bool,
    pub last_seen: Option<NaiveDateTime>,
    pub last_done_at: Option<NaiveDateTime>,
    pub last_done_title: Option<String>,
    /// ユーザーなら最後に使った PC、端末なら最後のユーザー
    pub counterpart: String,
    pub version: String,
    /// 今出ている (未完了の) 自分宛ての通知の数
    pub pending: usize,
}

/// 名簿と生存印と通知から、ユーザーか端末の一覧を作る。名簿に無いが生存印のある名前も並べる。
pub fn member_rows(
    kind: Kind,
    dir: &Directory,
    statuses: &[ClientStatus],
    notices: &[Notice],
) -> Vec<MemberRow> {
    let list = if kind == Kind::Device {
        &dir.devices
    } else {
        &dir.users
    };
    let mut rows: BTreeMap<String, MemberRow> = BTreeMap::new();
    for m in list {
        rows.insert(m.id.clone(), blank(m.clone(), true));
    }
    for s in statuses.iter().filter(|s| s.kind == kind) {
        let row = rows.entry(s.id.clone()).or_insert_with(|| {
            blank(
                Member {
                    id: s.id.clone(),
                    ..Default::default()
                },
                false,
            )
        });
        row.last_seen = Some(s.last_seen);
        row.last_done_at = s.last_done_at;
        row.last_done_title = s.last_done_title.clone();
        row.counterpart = s.counterpart.clone();
        row.version = s.version.clone();
    }
    for n in notices.iter().filter(|n| n.target.kind == kind) {
        if let Some(r) = rows.get_mut(&n.target.name) {
            r.pending += 1;
        }
    }
    rows.into_values().collect()
}

fn blank(member: Member, registered: bool) -> MemberRow {
    MemberRow {
        member,
        registered,
        last_seen: None,
        last_done_at: None,
        last_done_title: None,
        counterpart: String::new(),
        version: String::new(),
        pending: 0,
    }
}

/// `0.1.10` と `0.1.9` を数として比べる。読めない版は古いとみなさない。
pub fn is_older(version: &str, than: &str) -> bool {
    let parse = |v: &str| -> Option<Vec<u64>> { v.split('.').map(|p| p.parse().ok()).collect() };
    match (parse(version), parse(than)) {
        (Some(a), Some(b)) => a < b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use hatopo_core::{Origin, Target};

    fn at(h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 23)
            .unwrap()
            .and_hms_opt(h, 0, 0)
            .unwrap()
    }

    fn status(kind: Kind, id: &str, h: u32) -> ClientStatus {
        ClientStatus {
            kind,
            id: id.into(),
            counterpart: "pc-1".into(),
            version: "0.2.0".into(),
            last_seen: at(h),
            last_done_at: Some(at(h - 1)),
            last_done_title: Some("工数".into()),
        }
    }

    fn notice(kind: Kind, name: &str) -> Notice {
        Notice {
            schema: 1,
            id: "x".into(),
            revision: 1,
            target: Target::new(kind, name).unwrap(),
            title: "t".into(),
            body: String::new(),
            due: None,
            urls: vec![],
            created_at: at(1),
            created_by: Origin {
                user: "a".into(),
                host: "b".into(),
            },
            start_at: at(1),
            end_at: None,
            updated_at: None,
            updated_by: None,
            group: None,
        }
    }

    #[test]
    fn rows_merge_directory_status_and_pending_counts() {
        let dir = Directory {
            users: vec![
                Member {
                    id: "hanako".into(),
                    name: "佐藤 花子".into(),
                    note: String::new(),
                },
                Member {
                    id: "taro".into(),
                    name: "山田 太郎".into(),
                    note: String::new(),
                },
            ],
            ..Default::default()
        };
        let statuses = [
            status(Kind::User, "taro", 10),
            status(Kind::User, "jiro", 9),
            status(Kind::Device, "pc-1", 10),
        ];
        let notices = [
            notice(Kind::User, "taro"),
            notice(Kind::User, "taro"),
            notice(Kind::Device, "pc-1"),
        ];
        let rows = member_rows(Kind::User, &dir, &statuses, &notices);
        let ids: Vec<_> = rows.iter().map(|r| r.member.id.as_str()).collect();
        assert_eq!(ids, ["hanako", "jiro", "taro"]);
        let taro = &rows[2];
        assert!(taro.registered);
        assert_eq!(
            (taro.last_seen, taro.pending, taro.version.as_str()),
            (Some(at(10)), 2, "0.2.0")
        );
        assert!(!rows[1].registered, "生存印だけの jiro は名簿に足す候補");
        assert_eq!(rows[0].last_seen, None, "一度も起動していない人");

        let devs = member_rows(Kind::Device, &dir, &statuses, &notices);
        assert_eq!(devs.len(), 1);
        assert_eq!(devs[0].pending, 1);
    }

    #[test]
    fn version_comparison_is_numeric() {
        assert!(is_older("0.1.9", "0.1.10"));
        assert!(is_older("0.1.1", "0.2.0"));
        assert!(!is_older("0.2.0", "0.2.0"));
        assert!(
            !is_older("", "0.2.0"),
            "版の無い古い生存印は古いと決めつけない"
        );
    }
}
