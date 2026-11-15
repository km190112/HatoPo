//! 鳩の巣から読んだもの一式と、名簿の一覧を組み立てる計算 (画面から切り離してテストする)。

use chrono::NaiveDateTime;
use hatopo_core::{
    Archived, Broken, ClientStatus, Directory, Filter, Kind, Loft, Member, Notice, ReadMark,
    Settings, read_all_status,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

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
    /// 通知 ID ごとの既読の印
    pub reads: HashMap<String, Vec<ReadMark>>,
    /// 出ている通知の組 (group) ごとの、完了・期限切れ・削除で archive へ移った分
    pub group_closed: HashMap<String, Vec<Archived>>,
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
        d.reads = d
            .notices
            .iter()
            .map(|n| (n.id.clone(), loft.readers(&n.id).unwrap_or_default()))
            .collect();
        d.group_closed = load_group_closed(loft, &d.notices, &d.months);
        d
    }

    /// 組の宛先の表示。`taro ほか 2 人` / `PC-1 ほか 2 台`。組でなければ宛先そのまま。
    pub fn target_summary(&self, kind: Kind, first: &str, total: usize) -> String {
        let head = self.label(kind, first);
        if total <= 1 {
            return head;
        }
        let unit = if kind == Kind::Device { "台" } else { "人" };
        format!("{head} ほか {} {unit}", total - 1)
    }

    /// 出ている通知の組 (1 件だけの送信も可) の宛先の表示。archive へ移った分も人数に入れる。
    pub fn group_target_label(&self, members: &[Notice]) -> String {
        let n = &members[0];
        let closed = n
            .group
            .as_ref()
            .and_then(|g| self.group_closed.get(g))
            .map_or(0, Vec::len);
        self.target_summary(n.target.kind, &n.target.name, members.len() + closed)
    }

    /// 組の進み具合: `完了 2・既読 1・未読 1`。
    pub fn group_summary(&self, members: &[Notice]) -> String {
        let closed: &[Archived] = members[0]
            .group
            .as_ref()
            .and_then(|g| self.group_closed.get(g))
            .map_or(&[], Vec::as_slice);
        let done = closed
            .iter()
            .filter(|a| a.closed.reason == hatopo_core::CloseReason::Done)
            .count();
        let read = members
            .iter()
            .filter(|n| self.reads.get(&n.id).is_some_and(|r| !r.is_empty()))
            .count();
        let unread = members.len() - read;
        let mut s = format!("完了 {done}・既読 {read}・未読 {unread}");
        let other = closed.len() - done;
        if other > 0 {
            s.push_str(&format!("・終了 {other}"));
        }
        s
    }

    /// この通知の宛先ごとの既読 (未読が先)。
    pub fn read_rows(&self, n: &Notice) -> Vec<ReadRow> {
        let none = Vec::new();
        let readers = self.reads.get(&n.id).unwrap_or(&none);
        read_rows(n, readers, &self.directory, &self.statuses)
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

/// 「読んだ人」の表の 1 行。`mark` が無ければ未読。
#[derive(Clone, Debug, PartialEq)]
pub struct ReadRow {
    pub user: String,
    pub mark: Option<ReadMark>,
}

/// 誰が読んで誰が読んでいないか。
/// - 一斉送信: 名簿のユーザーと、HatoPo を起動したことがある (生存印のある) ユーザー全員
/// - ユーザー宛: その人
/// - 端末宛: 宛先が人ではないので、その PC で既読にした人だけ (誰もいなければ空)
///
/// 上の対象に入らない人の印 (名簿から外した人など) も、読んだ事実として並べる。
pub fn read_rows(
    n: &Notice,
    readers: &[ReadMark],
    dir: &Directory,
    statuses: &[ClientStatus],
) -> Vec<ReadRow> {
    let mut users: BTreeSet<String> = match n.target.kind {
        Kind::Broadcast => dir
            .users
            .iter()
            .map(|m| m.id.clone())
            .chain(
                statuses
                    .iter()
                    .filter(|s| s.kind == Kind::User)
                    .map(|s| s.id.clone()),
            )
            .collect(),
        Kind::User => [n.target.name.clone()].into(),
        Kind::Device => BTreeSet::new(),
    };
    users.extend(readers.iter().map(|r| r.user.clone()));
    let mut rows: Vec<ReadRow> = users
        .into_iter()
        .map(|user| ReadRow {
            mark: readers.iter().find(|r| r.user == user).cloned(),
            user,
        })
        .collect();
    rows.sort_by(|a, b| {
        a.mark
            .is_some()
            .cmp(&b.mark.is_some())
            .then(a.user.cmp(&b.user))
    });
    rows
}

/// 出ている通知の組ごとに、archive へ移った分を集める (組を作った月から今月まで)。
fn load_group_closed(
    loft: &Loft,
    notices: &[Notice],
    months: &[String],
) -> HashMap<String, Vec<Archived>> {
    let mut groups: BTreeMap<&str, String> = BTreeMap::new();
    for n in notices {
        if let Some(g) = n.group.as_deref() {
            let m = n.created_at.format("%Y-%m").to_string();
            let e = groups.entry(g).or_insert_with(|| m.clone());
            if m < *e {
                *e = m;
            }
        }
    }
    let mut out: HashMap<String, Vec<Archived>> = HashMap::new();
    let Some(from) = groups.values().min().cloned() else {
        return out;
    };
    for m in months.iter().filter(|m| **m >= from) {
        for a in loft.archived(m).unwrap_or_default() {
            if let Some(g) = a.notice.group.as_deref()
                && groups.contains_key(g)
            {
                out.entry(g.to_string()).or_default().push(a);
            }
        }
    }
    out
}

/// 同じキーの行を 1 組にまとめる。キーの無い行は 1 件ずつ。順序は最初に出た行の位置。
pub fn group_by_key<T: Clone>(rows: &[T], key: impl Fn(&T) -> Option<&str>) -> Vec<Vec<T>> {
    let mut out: Vec<Vec<T>> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for r in rows {
        match key(r) {
            Some(k) => match at.get(k) {
                Some(&i) => out[i].push(r.clone()),
                None => {
                    at.insert(k.to_string(), out.len());
                    out.push(vec![r.clone()]);
                }
            },
            None => out.push(vec![r.clone()]),
        }
    }
    out
}

/// 一覧の「既読」の列。一斉は `3/10 (完了 1)`、ユーザー宛は 完了 / 既読 / 未読、端末宛は読んだ人数。
pub fn read_summary(kind: Kind, rows: &[ReadRow]) -> String {
    let read = rows.iter().filter(|r| r.mark.is_some()).count();
    let done = rows
        .iter()
        .filter(|r| r.mark.as_ref().is_some_and(|m| m.done))
        .count();
    match kind {
        Kind::Broadcast if done > 0 => format!("{read}/{} (完了 {done})", rows.len()),
        Kind::Broadcast => format!("{read}/{}", rows.len()),
        _ if read == 0 => "未読".into(),
        Kind::User => "既読".into(),
        Kind::Device => format!("既読 {read} 人"),
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
            complete_button: None,
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

    fn mark(user: &str, h: u32) -> ReadMark {
        ReadMark {
            user: user.into(),
            host: "pc-1".into(),
            at: at(h),
            done: false,
        }
    }

    fn users(rows: &[ReadRow]) -> Vec<(&str, bool)> {
        rows.iter()
            .map(|r| (r.user.as_str(), r.mark.is_some()))
            .collect()
    }

    #[test]
    fn broadcast_readers_cover_directory_and_status_users_unread_first() {
        let dir = Directory {
            users: vec![Member {
                id: "hanako".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let statuses = [
            status(Kind::User, "taro", 10),
            status(Kind::Device, "pc-1", 10),
        ];
        let b = notice(Kind::Broadcast, "");
        let none = read_rows(&b, &[], &dir, &statuses);
        assert_eq!(users(&none), [("hanako", false), ("taro", false)]);
        assert_eq!(read_summary(Kind::Broadcast, &none), "0/2");

        // 名簿から外した jiro の印も、読んだ事実として出す
        let rows = read_rows(&b, &[mark("taro", 9), mark("jiro", 8)], &dir, &statuses);
        assert_eq!(
            users(&rows),
            [("hanako", false), ("jiro", true), ("taro", true)]
        );
        assert_eq!(read_summary(Kind::Broadcast, &rows), "2/3");
        assert!(read_rows(&b, &[], &Directory::default(), &[]).is_empty());
    }

    #[test]
    fn personal_and_device_readers() {
        let u = notice(Kind::User, "taro");
        let rows = read_rows(&u, &[], &Directory::default(), &[]);
        assert_eq!(users(&rows), [("taro", false)]);
        assert_eq!(read_summary(Kind::User, &rows), "未読");
        let rows = read_rows(&u, &[mark("taro", 9)], &Directory::default(), &[]);
        assert_eq!(read_summary(Kind::User, &rows), "既読");

        let d = notice(Kind::Device, "pc-1");
        let rows = read_rows(&d, &[], &Directory::default(), &[]);
        assert!(rows.is_empty(), "端末宛は読んだ人だけ");
        assert_eq!(read_summary(Kind::Device, &rows), "未読");
        let rows = read_rows(
            &d,
            &[mark("taro", 9), mark("jiro", 10)],
            &Directory::default(),
            &[],
        );
        assert_eq!(read_summary(Kind::Device, &rows), "既読 2 人");
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

    fn grouped(id: &str, group: Option<&str>, user: &str) -> Notice {
        let mut n = notice(Kind::User, user);
        n.id = id.into();
        n.group = group.map(str::to_string);
        n
    }

    #[test]
    fn same_group_becomes_one_row_in_first_seen_order() {
        let rows = vec![
            grouped("a", Some("g1"), "taro"),
            grouped("b", None, "ken"),
            grouped("c", Some("g1"), "hanako"),
            grouped("d", Some("g2"), "jiro"),
        ];
        let g = group_by_key(&rows, |n| n.group.as_deref());
        let ids: Vec<Vec<&str>> = g
            .iter()
            .map(|v| v.iter().map(|n| n.id.as_str()).collect())
            .collect();
        assert_eq!(ids, vec![vec!["a", "c"], vec!["b"], vec!["d"]]);
        // 0 件でも壊れない
        assert!(group_by_key::<Notice>(&[], |n| n.group.as_deref()).is_empty());
    }

    #[test]
    fn group_row_shows_who_and_how_far_it_went() {
        use hatopo_core::{Archived, CloseReason, Closed};
        let mut d = Data::default();
        let a = grouped("a", Some("g1"), "taro");
        let b = grouped("b", Some("g1"), "hanako");
        d.reads.insert("a".into(), vec![mark("taro", 9)]);
        d.group_closed.insert(
            "g1".into(),
            vec![Archived {
                notice: grouped("c", Some("g1"), "jiro"),
                closed: Closed {
                    at: at(10),
                    by: Origin {
                        user: "jiro".into(),
                        host: "pc-3".into(),
                    },
                    reason: CloseReason::Done,
                },
                read_by: vec![],
            }],
        );
        let members = vec![a, b];
        assert_eq!(d.group_summary(&members), "完了 1・既読 1・未読 1");
        // 完了済みの人も人数に入る: 出ている 2 人 + 完了 1 人
        assert_eq!(d.group_target_label(&members), "taro ほか 2 人");
        assert_eq!(d.target_summary(Kind::Device, "pc-1", 3), "pc-1 ほか 2 台");
        assert_eq!(d.target_summary(Kind::User, "taro", 1), "taro");
    }

    #[test]
    fn broadcast_summary_counts_done_separately() {
        let rows = vec![
            ReadRow {
                user: "a".into(),
                mark: Some(ReadMark {
                    done: true,
                    ..mark("a", 9)
                }),
            },
            ReadRow {
                user: "b".into(),
                mark: Some(mark("b", 9)),
            },
            ReadRow {
                user: "c".into(),
                mark: None,
            },
        ];
        assert_eq!(read_summary(Kind::Broadcast, &rows), "2/3 (完了 1)");
    }
}
