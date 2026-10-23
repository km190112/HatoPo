//! 裏方のスレッド。共有フォルダを定期的に見に行き、完了の書き込みもここで行う。
//! 遅い共有で画面が固まらないように、画面のスレッドからはファイルに触らない。

use crate::local::LocalState;
use chrono::{Local, NaiveDateTime};
use hatopo_core::{CloseReason, Kind, Loft, Notice, Origin, Settings};
use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const AUMID: &str = "HatoPo.Notifier";

/// 画面から裏方への依頼。
pub enum Cmd {
    Refresh,
    Complete(String),
}

/// 裏方から画面への知らせ (トレイ・トーストの操作も同じ口で来る)。
#[derive(Debug, Clone, PartialEq)]
pub enum UiMsg {
    OpenList,
    ToggleMascot,
    /// トレイの「ログオン時に自動で起動する」が押された (チェックは既に切り替わっている)
    ToggleAutostart,
    /// マスコットの大きさ (%) を選んだ
    SetScale(u32),
    /// キャラクターを選んだ (None は管理者の設定に合わせる)
    SetCharacter(Option<String>),
    /// 「HatoPo について」(著作権・ライセンス) を開く
    OpenAbout,
    Quit,
    /// 共有の exe が新しくなったのでローカルへ入れ替えた。起動し直す
    Restart,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Flash {
    /// 1 件完了した
    Done,
    /// 完了して残りが 0 件になった
    AllDone,
}

/// 画面に出すもの一式。
#[derive(Clone, Default)]
pub struct Snapshot {
    /// 一度でも読めたか (起動直後は false)
    pub loaded: bool,
    pub online: bool,
    /// 共有に繋がらない理由
    pub offline_reason: Option<String>,
    pub checked_at: Option<NaiveDateTime>,
    pub items: Vec<Notice>,
    pub waiting: usize,
    pub broken: Vec<String>,
    pub settings: Settings,
    pub settings_warning: Option<String>,
    /// 鳩の巣の `character_icon\` にあるキャラクターの名前 (選ぶ一覧に出す)
    pub characters: Vec<String>,
    /// 完了などの操作に失敗した理由
    pub action_error: Option<String>,
    pub flash: Option<(Flash, Instant)>,
    /// 完了を押して書き込み中の ID
    pub busy: HashSet<String>,
}

pub struct Shared {
    pub snap: Snapshot,
    pub local: LocalState,
}

impl Shared {
    /// 対応が要る件数 (既読にした一斉送信を除く)。
    pub fn pending(&self) -> usize {
        pending(&self.snap.items, &self.local.read_broadcasts).count()
    }

    pub fn has_unseen(&self) -> bool {
        pending(&self.snap.items, &self.local.read_broadcasts)
            .any(|n| !self.local.seen.contains(&n.id))
    }
}

fn pending<'a>(
    items: &'a [Notice],
    read: &'a BTreeSet<String>,
) -> impl Iterator<Item = &'a Notice> + 'a {
    items
        .iter()
        .filter(move |n| !(n.target.kind == Kind::Broadcast && read.contains(&n.id)))
}

pub type SharedRef = Arc<Mutex<Shared>>;

fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

// ---------------------------------------------------------------------------
// トーストを出すかの判断 (純粋な部分。テストする)
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub struct ToastMsg {
    pub title: String,
    pub text: String,
}

#[derive(Default)]
pub struct Reminder {
    known: HashSet<String>,
    last: Option<NaiveDateTime>,
    started: bool,
}

/// 1 回に個別のトーストで出すのはここまで。超えたら 1 枚にまとめる。
const MAX_INDIVIDUAL: usize = 3;

impl Reminder {
    /// 一覧が読めるたびに呼ぶ。出すべきトーストを返す。
    pub fn plan(
        &mut self,
        pending: &[&Notice],
        now: NaiveDateTime,
        remind_minutes: u64,
    ) -> Vec<ToastMsg> {
        let ids: HashSet<String> = pending.iter().map(|n| n.id.clone()).collect();
        let mut out = Vec::new();
        if !self.started {
            // 起動直後は既にあるものを 1 件ずつ鳴らさない。まとめて 1 回知らせる
            self.started = true;
            if !pending.is_empty() {
                out.push(summary(pending.len()));
                self.last = Some(now);
            }
        } else {
            let new: Vec<&&Notice> = pending
                .iter()
                .filter(|n| !self.known.contains(&n.id))
                .collect();
            if new.len() > MAX_INDIVIDUAL {
                out.push(ToastMsg {
                    title: "HatoPoからの便り".into(),
                    text: format!("新しい便りが {} 件届きました", new.len()),
                });
            } else {
                out.extend(new.iter().map(|n| ToastMsg {
                    title: "HatoPoからの便り".into(),
                    text: n.title.clone(),
                }));
            }
            if !out.is_empty() {
                self.last = Some(now);
            } else if remind_minutes > 0 && !pending.is_empty() {
                let due = self
                    .last
                    .is_none_or(|l| now - l >= chrono::Duration::minutes(remind_minutes as i64));
                if due {
                    out.push(summary(pending.len()));
                    self.last = Some(now);
                }
            }
        }
        self.known = ids;
        out
    }
}

fn summary(n: usize) -> ToastMsg {
    ToastMsg {
        title: "HatoPoからの便り".into(),
        text: format!("まだ済んでいない便りが {n} 件あります"),
    }
}

// ---------------------------------------------------------------------------
// スレッド本体
// ---------------------------------------------------------------------------

pub struct Worker {
    pub loft: Loft,
    pub me: Origin,
    pub shared: SharedRef,
    pub ctx: eframe::egui::Context,
    pub ui_tx: Sender<UiMsg>,
    /// ローカルに導入された exe で動いているとき、その場所 (自己更新に使う)
    pub installed_exe: Option<PathBuf>,
    pub toast_icon: Option<PathBuf>,
}

impl Worker {
    pub fn run(self, rx: Receiver<Cmd>) {
        let mut reminder = Reminder::default();
        let mut last_touch: Option<chrono::NaiveDate> = None;
        loop {
            let online = self.poll(&mut reminder);
            // 生存印 (最終アクセス日) は起動直後と、日付が変わって最初の確認だけ書く。
            // 起動時だけにすると、何日もログオンしたままの PC が管理画面で「アクセス無し」に見える
            let today = now().date();
            if online && last_touch != Some(today) {
                self.touch(None);
                last_touch = Some(today);
            }
            let wait = {
                let s = self.shared.lock().unwrap();
                Duration::from_secs(s.snap.settings.poll_seconds.max(10))
            };
            match rx.recv_timeout(wait) {
                Ok(Cmd::Refresh) | Err(RecvTimeoutError::Timeout) => {}
                Ok(Cmd::Complete(id)) => self.complete(&id),
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    /// 管理画面が「最終アクセス」「最後に完了した日」「版」を出すための生存印。失敗しても通知は止めない。
    fn touch(&self, done: Option<&str>) {
        let _ = hatopo_core::touch(
            self.loft.root(),
            &self.me,
            env!("CARGO_PKG_VERSION"),
            now(),
            done,
        );
    }

    fn complete(&self, id: &str) {
        let title = {
            let s = self.shared.lock().unwrap();
            s.snap
                .items
                .iter()
                .find(|n| n.id == id)
                .map(|n| n.title.clone())
        };
        let result = self.loft.close(id, CloseReason::Done, &self.me, now());
        if result.is_ok() {
            self.touch(Some(title.as_deref().unwrap_or("")));
        }
        let mut s = self.shared.lock().unwrap();
        s.snap.busy.remove(id);
        match result {
            Ok(_) | Err(hatopo_core::Error::NotFound { .. }) => {
                // NotFound は他の人 (同じ PC 宛ての通知を別のユーザー) が先に完了させた
                s.snap.items.retain(|n| n.id != id);
                s.snap.action_error = None;
                let left = s.pending();
                s.snap.flash = Some((
                    if left == 0 {
                        Flash::AllDone
                    } else {
                        Flash::Done
                    },
                    Instant::now(),
                ));
            }
            Err(e) => s.snap.action_error = Some(format!("完了にできませんでした: {e}")),
        }
        drop(s);
        self.ctx.request_repaint();
    }

    /// 共有に繋がったら `true`。
    fn poll(&self, reminder: &mut Reminder) -> bool {
        let (settings, warning) = Settings::load(self.loft.root());
        let t = now();
        let result = self.loft.inbox(&self.me.user, &self.me.host, t);
        let mut toasts = Vec::new();
        {
            let mut s = self.shared.lock().unwrap();
            s.snap.loaded = true;
            match result {
                Ok(inbox) => {
                    // 期限切れは見つけた端末が archive へ移す (他の端末と重なっても core が黙って譲る)
                    let errs = self.loft.sweep_expired(&inbox.expired, &self.me, t);
                    s.snap.online = true;
                    s.snap.offline_reason = None;
                    s.snap.checked_at = Some(t);
                    s.snap.items = inbox.active;
                    s.snap.waiting = inbox.waiting;
                    s.snap.broken = inbox
                        .broken
                        .iter()
                        .map(|b| b.path.display().to_string())
                        .chain(errs.iter().map(|e| e.to_string()))
                        .collect();
                    s.snap.settings = settings;
                    s.snap.settings_warning = warning;
                    s.snap.characters = Settings::characters(self.loft.root());
                    let alive: BTreeSet<String> =
                        s.snap.items.iter().map(|n| n.id.clone()).collect();
                    if s.local.prune(&alive) {
                        s.local.save();
                    }
                    let pend: Vec<&Notice> =
                        pending(&s.snap.items, &s.local.read_broadcasts).collect();
                    toasts = reminder.plan(&pend, t, s.snap.settings.remind_minutes);
                }
                Err(e) => {
                    // 最後に読めた一覧は残して出し続ける
                    s.snap.online = false;
                    s.snap.offline_reason = Some(e.to_string());
                }
            }
        }
        for m in toasts {
            self.toast(&m);
        }
        self.ctx.request_repaint();
        self.check_update();
        self.shared.lock().unwrap().snap.online
    }

    fn check_update(&self) {
        let Some(exe) = &self.installed_exe else {
            return;
        };
        match crate::boot::update_if_changed(self.loft.root(), exe) {
            Ok(true) => {
                let _ = self.ui_tx.send(UiMsg::Restart);
                self.ctx.request_repaint();
            }
            Ok(false) => {}
            Err(e) => {
                let mut s = self.shared.lock().unwrap();
                s.snap.action_error = Some(e);
            }
        }
    }

    fn toast(&self, m: &ToastMsg) {
        use tauri_winrt_notification::{IconCrop, Toast};
        let tx = self.ui_tx.clone();
        let ctx = self.ctx.clone();
        let mut t = Toast::new(AUMID).title(&m.title).text1(&m.text);
        if let Some(icon) = &self.toast_icon {
            t = t.icon(icon, IconCrop::Square, "HatoPo");
        }
        let t = t.on_activated(move |_| {
            let _ = tx.send(UiMsg::OpenList);
            ctx.request_repaint();
            Ok(())
        });
        // トーストが出せなくても通知はマスコットとトレイで見えるので、止めない
        let _ = t.show();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use hatopo_core::Target;

    fn at(min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 1)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap()
            + chrono::Duration::minutes(min as i64)
    }

    fn n(id: &str) -> Notice {
        Notice {
            schema: 1,
            id: id.into(),
            revision: 1,
            target: Target::new(Kind::User, "taro").unwrap(),
            title: format!("件名{id}"),
            body: String::new(),
            due: None,
            urls: vec![],
            created_at: at(0),
            created_by: Origin {
                user: "a".into(),
                host: "b".into(),
            },
            start_at: at(0),
            end_at: None,
            updated_at: None,
            updated_by: None,
            group: None,
        }
    }

    fn texts(v: &[ToastMsg]) -> Vec<&str> {
        v.iter().map(|m| m.text.as_str()).collect()
    }

    #[test]
    fn startup_summarizes_existing_then_new_ones_are_individual() {
        let mut r = Reminder::default();
        let (a, b, c) = (n("a"), n("b"), n("c"));
        assert_eq!(
            texts(&r.plan(&[&a, &b], at(0), 60)),
            ["まだ済んでいない便りが 2 件あります"]
        );
        assert!(r.plan(&[&a, &b], at(1), 60).is_empty(), "変化なし・間隔前");
        assert_eq!(texts(&r.plan(&[&a, &b, &c], at(2), 60)), ["件名c"]);
    }

    #[test]
    fn startup_with_nothing_is_silent() {
        let mut r = Reminder::default();
        assert!(r.plan(&[], at(0), 60).is_empty());
        let a = n("a");
        assert_eq!(texts(&r.plan(&[&a], at(1), 60)), ["件名a"]);
    }

    #[test]
    fn many_new_ones_are_merged() {
        let mut r = Reminder::default();
        r.plan(&[], at(0), 60);
        let v: Vec<Notice> = (0..5).map(|i| n(&i.to_string())).collect();
        let refs: Vec<&Notice> = v.iter().collect();
        assert_eq!(
            texts(&r.plan(&refs, at(1), 60)),
            ["新しい便りが 5 件届きました"]
        );
    }

    #[test]
    fn reminds_every_interval_while_pending_and_zero_disables() {
        let mut r = Reminder::default();
        let a = n("a");
        r.plan(&[&a], at(0), 60);
        assert!(r.plan(&[&a], at(59), 60).is_empty());
        assert_eq!(
            texts(&r.plan(&[&a], at(60), 60)),
            ["まだ済んでいない便りが 1 件あります"]
        );
        assert!(r.plan(&[&a], at(61), 60).is_empty());

        let mut off = Reminder::default();
        off.plan(&[&a], at(0), 0);
        assert!(off.plan(&[&a], at(600), 0).is_empty());
    }

    #[test]
    fn completed_then_reissued_id_is_not_new_twice() {
        let mut r = Reminder::default();
        let a = n("a");
        r.plan(&[], at(0), 60);
        assert_eq!(r.plan(&[&a], at(1), 60).len(), 1);
        assert!(r.plan(&[], at(2), 60).is_empty(), "完了して 0 件");
        assert!(r.plan(&[], at(200), 60).is_empty(), "0 件なら催促しない");
    }

    #[test]
    fn read_broadcasts_do_not_count_as_pending() {
        let mut b = n("b");
        b.target = Target::broadcast();
        let u = n("u");
        let shared = Shared {
            snap: Snapshot {
                items: vec![b, u],
                ..Default::default()
            },
            local: LocalState {
                read_broadcasts: ["b".to_string()].into(),
                seen: ["u".to_string()].into(),
                ..Default::default()
            },
        };
        assert_eq!(shared.pending(), 1);
        assert!(!shared.has_unseen());
    }
}
