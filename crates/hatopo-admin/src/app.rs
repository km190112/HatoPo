//! 管理画面。通知 / 新しく送る / 完了の記録 / ユーザー / 端末 / 設定 の 6 つの画面。
//!
//! 共有フォルダへの読み書きは画面のスレッドで行う (管理者が 1〜2 人で使う道具なので、
//! 遅い共有で一瞬固まるのは許す)。読み直しは［再読込］と、書き込んだ直後に行う。

use crate::data::{Data, MemberRow, group_by_key, is_older, member_rows, read_rows, read_summary};
use crate::form::NoticeForm;
use chrono::{Local, NaiveDateTime};
use eframe::egui::{self, Color32, RichText};
use egui_extras::{Column, TableBuilder};
use hatopo_core::{
    Archived, CloseReason, Directory, Error, ImportResult, Kind, Loft, MASCOT_SCALES,
    MAX_POLL_SECONDS, MAX_REMIND_MINUTES, MIN_POLL_SECONDS, Member, Notice, Origin, Settings,
    Target, import_members_csv, merge_members,
};
use std::collections::BTreeSet;
use std::path::PathBuf;

const OK: Color32 = Color32::from_rgb(0x1F, 0x8A, 0x4C);
const WARN: Color32 = Color32::from_rgb(0xB0, 0x6A, 0x00);
const BAD: Color32 = Color32::from_rgb(0xC6, 0x2F, 0x2F);
const MUTED: Color32 = Color32::from_rgb(0x5A, 0x67, 0x7D);
/// 最終アクセスがこれより古い PC・人は黄色で出す (HatoPo が止まっているかもしれない)
const STALE_DAYS: i64 = 7;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Notices,
    Compose,
    Archive,
    Users,
    Devices,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StateFilter {
    All,
    Showing,
    Waiting,
    Expired,
}

/// 名簿の編集中の 1 行。`original` が None なら新しく足す行。
struct MemberEdit {
    kind: Kind,
    member: Member,
    original: Option<String>,
    confirm_delete: bool,
}

/// 直前に CSV から取り込んだ結果 (エラー行を見せている間だけ保持する)。
struct MemberImport {
    kind: Kind,
    result: ImportResult,
}

/// 完了の記録の詳細。同じ送信 (組) なら全員分と、まだ出ている (未完了の) 通知も並べる。
struct ArchiveDetail {
    members: Vec<Archived>,
    active: Vec<Notice>,
}

pub struct AdminApp {
    loft: Loft,
    loft_input: String,
    me: Origin,
    data: Data,
    loaded_at: Option<NaiveDateTime>,
    tab: Tab,
    message: Option<(bool, String)>,

    f_kind: Option<Kind>,
    f_state: StateFilter,
    f_keyword: String,
    selected: Option<String>,
    edit: Option<NoticeForm>,
    edit_revision: u64,
    apply_to_group: bool,
    confirm_delete: bool,
    group_view: Option<(String, Vec<Notice>, Vec<Archived>)>,

    c_kind: Kind,
    c_selected: BTreeSet<String>,
    c_extra: String,
    c_search: String,
    c_form: NoticeForm,

    a_month: String,
    a_rows: Vec<Archived>,
    a_reason: Option<CloseReason>,
    a_keyword: String,
    a_selected: Option<String>,
    a_detail: Option<ArchiveDetail>,

    m_edit: Option<MemberEdit>,
    m_search: String,
    m_import: Option<MemberImport>,

    s_edit: Settings,
    show_gallery: bool,
    /// キャラクター一覧の絵 (読めなかったものは None)。一覧を開くたびに読み直す
    textures: std::collections::HashMap<PathBuf, Option<egui::TextureHandle>>,
}

/// キャラクターが持つ絵 (`poses\<名前>.png`) と、画面に出す説明。
const POSE_LABELS: [(&str, &str); 7] = [
    ("idle", "通知なし"),
    ("letter", "未開封あり"),
    ("letter_calm", "目は通した"),
    ("happy", "1 件完了"),
    ("celebrate", "全部完了"),
    ("puzzled", "接続できない"),
    ("sleep", "休み中"),
];

fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

fn fmt(dt: NaiveDateTime) -> String {
    dt.format("%Y/%m/%d %H:%M").to_string()
}

fn fmt_opt(dt: Option<NaiveDateTime>) -> String {
    dt.map(fmt).unwrap_or_else(|| "—".into())
}

fn kind_label(k: Kind) -> &'static str {
    match k {
        Kind::Broadcast => "一斉",
        Kind::Device => "端末",
        Kind::User => "ユーザー",
    }
}

fn reason_label(r: CloseReason) -> &'static str {
    match r {
        CloseReason::Done => "完了",
        CloseReason::Expired => "期限切れ",
        CloseReason::Deleted => "削除",
    }
}

fn state_of(n: &Notice, t: NaiveDateTime) -> (&'static str, Color32) {
    if n.is_expired(t) {
        ("期限切れ", MUTED)
    } else if !n.is_started(t) {
        ("開始前", WARN)
    } else {
        ("表示中", OK)
    }
}

pub fn admin_loft_file() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("HatoPo")
        .join("admin-loft.txt")
}

impl AdminApp {
    pub fn new(cc: &eframe::CreationContext<'_>, loft: PathBuf) -> AdminApp {
        crate::setup_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::light());
        cc.egui_ctx.style_mut(|s| {
            // 文字はドラッグで選んでコピーできる。表の行を開くクリックは、選択できない左端の列で受ける
            s.interaction.selectable_labels = true;
            s.spacing.item_spacing = egui::vec2(8.0, 6.0);
            s.spacing.button_padding = egui::vec2(10.0, 4.0);
        });
        let today = now().date();
        let mut app = AdminApp {
            loft_input: loft.display().to_string(),
            loft: Loft::new(loft),
            me: Origin::current(),
            data: Data::default(),
            loaded_at: None,
            tab: Tab::Notices,
            message: None,
            f_kind: None,
            f_state: StateFilter::All,
            f_keyword: String::new(),
            selected: None,
            edit: None,
            edit_revision: 0,
            apply_to_group: false,
            confirm_delete: false,
            group_view: None,
            c_kind: Kind::User,
            c_selected: BTreeSet::new(),
            c_extra: String::new(),
            c_search: String::new(),
            c_form: NoticeForm::new(today),
            a_month: String::new(),
            a_rows: Vec::new(),
            a_reason: None,
            a_keyword: String::new(),
            a_selected: None,
            a_detail: None,
            m_edit: None,
            m_search: String::new(),
            m_import: None,
            s_edit: Settings::default(),
            show_gallery: false,
            textures: Default::default(),
        };
        app.reload();
        app
    }

    fn reload(&mut self) {
        self.data = Data::load(&self.loft);
        self.loaded_at = Some(now());
        self.s_edit = self.data.settings.clone();
        if !self.data.months.contains(&self.a_month) {
            self.a_month = self.data.months.first().cloned().unwrap_or_default();
        }
        self.load_archive();
        // 選んでいた通知が消えていたら (完了・削除) 編集を閉じる
        if let Some(id) = &self.selected
            && !self.data.notices.iter().any(|n| &n.id == id)
        {
            self.close_editor();
        }
    }

    fn load_archive(&mut self) {
        self.a_selected = None;
        self.a_detail = None;
        self.a_rows = if self.a_month.is_empty() {
            Vec::new()
        } else {
            match self.loft.archived(&self.a_month) {
                Ok(mut v) => {
                    v.reverse(); // 新しい順
                    v
                }
                Err(e) => {
                    self.fail(e.to_string());
                    Vec::new()
                }
            }
        };
    }

    fn ok(&mut self, msg: impl Into<String>) {
        self.message = Some((false, msg.into()));
    }

    fn fail(&mut self, msg: impl Into<String>) {
        self.message = Some((true, msg.into()));
    }

    fn close_editor(&mut self) {
        self.selected = None;
        self.edit = None;
        self.confirm_delete = false;
        self.apply_to_group = false;
        self.group_view = None;
    }

    fn select(&mut self, n: &Notice) {
        self.selected = Some(n.id.clone());
        self.edit = Some(NoticeForm::from_notice(n, now().date()));
        self.edit_revision = n.revision;
        self.confirm_delete = false;
        self.apply_to_group = false;
        self.group_view = n.group.as_ref().and_then(|g| {
            self.loft
                .group_status(g, n.created_at, now())
                .ok()
                .map(|(a, c)| (g.clone(), a, c))
        });
    }

    fn change_loft(&mut self, path: PathBuf) {
        self.loft = Loft::new(&path);
        self.loft_input = path.display().to_string();
        let f = admin_loft_file();
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&f, path.display().to_string());
        self.close_editor();
        self.m_edit = None;
        self.m_import = None;
        self.reload();
        match &self.data.error {
            None => self.ok(format!("{} を開きました。", path.display())),
            Some(e) => self.fail(e.clone()),
        }
    }
}

impl eframe::App for AdminApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(0xE6, 0xEF, 0xFC))
                    .inner_margin(egui::Margin::symmetric(14, 8)),
            )
            .show(ctx, |ui| self.header(ui));

        egui::SidePanel::left("nav")
            .resizable(false)
            .exact_width(150.0)
            .show(ctx, |ui| self.nav(ui));

        if self.tab == Tab::Notices && self.edit.is_some() {
            egui::SidePanel::right("editor")
                .resizable(true)
                .default_width(470.0)
                .min_width(380.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.editor(ui));
                });
        }

        if self.tab == Tab::Archive && self.a_detail.is_some() {
            egui::SidePanel::right("archive-detail")
                .resizable(true)
                .default_width(470.0)
                .min_width(380.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.archive_detail(ui));
                });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some((is_err, msg)) = self.message.clone() {
                let fill = if is_err {
                    Color32::from_rgb(0xFD, 0xE7, 0xE6)
                } else {
                    Color32::from_rgb(0xE2, 0xF4, 0xEA)
                };
                egui::Frame::new()
                    .fill(fill)
                    .corner_radius(6)
                    .inner_margin(8)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal_wrapped(|ui| {
                            ui.label(msg);
                            if ui.small_button("閉じる").clicked() {
                                self.message = None;
                            }
                        });
                    });
                ui.add_space(6.0);
            }
            if let Some(e) = self.data.error.clone() {
                ui.colored_label(BAD, e);
                ui.label(
                    "［鳩の巣の場所］を確かめるか、［鳩の巣を作る］(設定の画面) で作ってください。",
                );
                ui.add_space(8.0);
            }
            match self.tab {
                Tab::Notices => self.notices(ui),
                Tab::Compose => self.compose(ui),
                Tab::Archive => self.archive(ui),
                Tab::Users => self.members(ui, Kind::User),
                Tab::Devices => self.members(ui, Kind::Device),
                Tab::Settings => self.settings(ui),
            }
        });
    }
}

impl AdminApp {
    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("HatoPo 管理").size(18.0).strong());
            ui.add_space(16.0);
            ui.label("鳩の巣の場所");
            ui.add(egui::TextEdit::singleline(&mut self.loft_input).desired_width(360.0));
            if ui.button("開く").clicked() {
                let p = PathBuf::from(self.loft_input.trim());
                self.change_loft(p);
            }
            if ui.button("参照…").clicked()
                && let Some(p) = rfd::FileDialog::new()
                    .set_title("鳩の巣 (HatoLoft) のフォルダを選ぶ")
                    .pick_folder()
            {
                self.change_loft(p);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("再読込").clicked() {
                    self.reload();
                    self.message = None;
                }
                let (c, t) = if self.data.error.is_none() {
                    (OK, "接続中")
                } else {
                    (BAD, "繋がりません")
                };
                ui.label(
                    RichText::new(format!(
                        "● {t}  {}",
                        self.loaded_at
                            .map(|t| t.format("%H:%M:%S 読込").to_string())
                            .unwrap_or_default()
                    ))
                    .color(c),
                );
            });
        });
    }

    fn nav(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        let pending = self.data.notices.len();
        let items = [
            (Tab::Notices, format!("通知 ({pending})")),
            (Tab::Compose, "新しく送る".to_string()),
            (Tab::Archive, "完了の記録".to_string()),
            (
                Tab::Users,
                format!("ユーザー ({})", self.data.directory.users.len()),
            ),
            (
                Tab::Devices,
                format!("端末 ({})", self.data.directory.devices.len()),
            ),
            (Tab::Settings, "設定".to_string()),
        ];
        for (t, label) in items {
            let sel = self.tab == t;
            if ui
                .add_sized(
                    [ui.available_width(), 32.0],
                    egui::Button::selectable(sel, RichText::new(label).size(14.0)),
                )
                .clicked()
            {
                self.tab = t;
            }
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.label(
                RichText::new(format!(
                    "操作する人: {}@{}\nHatoPo 管理 {}",
                    self.me.user,
                    self.me.host,
                    env!("CARGO_PKG_VERSION")
                ))
                .small()
                .color(MUTED),
            );
        });
    }

    // ------------------------------------------------------------------ 通知
    fn notices(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("種類");
            egui::ComboBox::from_id_salt("f-kind")
                .selected_text(self.f_kind.map(kind_label).unwrap_or("すべて"))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.f_kind, None, "すべて");
                    for k in [Kind::Broadcast, Kind::Device, Kind::User] {
                        ui.selectable_value(&mut self.f_kind, Some(k), kind_label(k));
                    }
                });
            ui.label("状態");
            egui::ComboBox::from_id_salt("f-state")
                .selected_text(match self.f_state {
                    StateFilter::All => "すべて",
                    StateFilter::Showing => "表示中",
                    StateFilter::Waiting => "開始前",
                    StateFilter::Expired => "期限切れ",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.f_state, StateFilter::All, "すべて");
                    ui.selectable_value(&mut self.f_state, StateFilter::Showing, "表示中");
                    ui.selectable_value(&mut self.f_state, StateFilter::Waiting, "開始前");
                    ui.selectable_value(&mut self.f_state, StateFilter::Expired, "期限切れ");
                });
            ui.label("検索");
            ui.add(
                egui::TextEdit::singleline(&mut self.f_keyword)
                    .hint_text("件名・本文・宛先")
                    .desired_width(200.0),
            );
            if ui.button("＋ 新しく送る").clicked() {
                self.tab = Tab::Compose;
            }
        });
        if !self.data.broken.is_empty() {
            ui.colored_label(
                WARN,
                format!(
                    "読めない通知ファイルが {} 件あります: {}",
                    self.data.broken.len(),
                    self.data
                        .broken
                        .iter()
                        .map(|b| b.path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
        }
        ui.add_space(4.0);

        let t = now();
        let kw = self.f_keyword.trim().to_lowercase();
        let rows: Vec<Notice> = self
            .data
            .notices
            .iter()
            .filter(|n| self.f_kind.is_none_or(|k| n.target.kind == k))
            .filter(|n| match self.f_state {
                StateFilter::All => true,
                StateFilter::Showing => n.is_started(t) && !n.is_expired(t),
                StateFilter::Waiting => !n.is_started(t),
                StateFilter::Expired => n.is_expired(t),
            })
            .filter(|n| {
                kw.is_empty()
                    || n.title.to_lowercase().contains(&kw)
                    || n.body.to_lowercase().contains(&kw)
                    || n.target.name.contains(&kw)
            })
            .cloned()
            .collect();
        if rows.is_empty() {
            ui.add_space(20.0);
            ui.label(RichText::new("条件に合う通知はありません。").color(MUTED));
            return;
        }
        let groups = group_by_key(&rows, |n| n.group.as_deref());
        ui.label(
            RichText::new(
                "左端の「種類」をクリックすると詳細が開きます。文字はドラッグで選んでコピーできます。複数の宛先へ送ったものは 1 行にまとめています。",
            )
            .small()
            .color(MUTED),
        );
        let mut clicked: Option<Vec<Notice>> = None;
        egui::ScrollArea::horizontal().show(ui, |ui| {
            TableBuilder::new(ui)
                .striped(true)
                .resizable(true)
                .sense(egui::Sense::click())
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::exact(64.0))
                .column(Column::initial(150.0).at_least(80.0))
                .column(Column::initial(260.0).at_least(120.0))
                .column(Column::exact(124.0))
                .column(Column::exact(124.0))
                .column(Column::exact(84.0))
                .column(Column::exact(70.0))
                .column(Column::exact(190.0))
                .column(Column::remainder().at_least(60.0))
                .header(24.0, |mut h| {
                    for c in [
                        "種類",
                        "宛先",
                        "件名",
                        "開始",
                        "終了",
                        "依頼期限",
                        "状態",
                        "既読",
                        "組",
                    ] {
                        h.col(|ui| {
                            ui.strong(c);
                        });
                    }
                })
                .body(|mut body| {
                    for g in &groups {
                        let n = &g[0];
                        body.row(26.0, |mut row| {
                            row.set_selected(
                                self.selected
                                    .as_deref()
                                    .is_some_and(|s| g.iter().any(|m| m.id == s)),
                            );
                            row.col(|ui| {
                                // 選択できない文字にして、行のクリックを受ける
                                ui.add(
                                    egui::Label::new(kind_label(n.target.kind)).selectable(false),
                                );
                            });
                            row.col(|ui| {
                                if n.group.is_some() {
                                    ui.label(self.data.group_target_label(g));
                                } else {
                                    ui.label(self.data.label(n.target.kind, &n.target.name));
                                }
                            });
                            row.col(|ui| {
                                ui.label(&n.title);
                            });
                            row.col(|ui| {
                                ui.label(fmt(n.start_at));
                            });
                            row.col(|ui| {
                                ui.label(fmt_opt(n.end_at));
                            });
                            row.col(|ui| {
                                ui.label(
                                    n.due
                                        .map(|d| d.format("%Y/%m/%d").to_string())
                                        .unwrap_or_else(|| "—".into()),
                                );
                            });
                            row.col(|ui| {
                                let (s, c) = state_of(n, t);
                                ui.colored_label(c, s);
                            });
                            row.col(|ui| {
                                if n.group.is_some() {
                                    ui.label(self.data.group_summary(g));
                                } else {
                                    ui.label(read_summary(n.target.kind, &self.data.read_rows(n)));
                                }
                            });
                            row.col(|ui| {
                                if n.group.is_some() {
                                    ui.label("まとめて送信");
                                }
                            });
                            if row.response().clicked() {
                                clicked = Some(g.clone());
                            }
                        });
                    }
                });
        });
        if let Some(g) = clicked {
            self.select(&g[0]);
            // 複数の宛先へ送ったものは、編集も削除も全員分が初期値
            if g.len() > 1 {
                self.apply_to_group = true;
            }
        }
    }

    fn editor(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        let Some(n) = self.data.notices.iter().find(|n| n.id == id).cloned() else {
            return;
        };
        ui.horizontal(|ui| {
            ui.heading("通知を直す");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("閉じる").clicked() {
                    self.close_editor();
                }
            });
        });
        if self.edit.is_none() {
            return;
        }
        ui.label(
            RichText::new(format!(
                "{} ・ {}\n作成 {} ({}@{}) ・ 版 {}",
                kind_label(n.target.kind),
                self.data.label(n.target.kind, &n.target.name),
                fmt(n.created_at),
                n.created_by.user,
                n.created_by.host,
                n.revision
            ))
            .color(MUTED),
        );
        ui.add_space(6.0);
        if let Some(form) = self.edit.as_mut() {
            form.ui(ui, "edit");
        }
        ui.add_space(8.0);
        let group_others: Vec<Notice> = self
            .group_view
            .as_ref()
            .map(|(_, a, _)| a.iter().filter(|x| x.id != id).cloned().collect())
            .unwrap_or_default();
        if !group_others.is_empty() {
            ui.checkbox(
                &mut self.apply_to_group,
                format!(
                    "同じ送信のほかの {} 件にも同じ変更・削除をする",
                    group_others.len()
                ),
            );
        }
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new(RichText::new("保存").color(Color32::WHITE)).fill(OK))
                .clicked()
            {
                self.save_edit(&n, &group_others);
            }
            if self.confirm_delete {
                ui.label("archive へ移します。");
                if ui
                    .add(
                        egui::Button::new(RichText::new("削除する").color(Color32::WHITE))
                            .fill(BAD),
                    )
                    .clicked()
                {
                    self.delete_selected(&n, &group_others);
                }
                if ui.button("やめる").clicked() {
                    self.confirm_delete = false;
                }
            } else if ui.button("削除…").clicked() {
                self.confirm_delete = true;
            }
        });

        // まとめ送信は下の「同じ送信の状況」に既読も出すので、ここは 1 件ずつ送ったものだけ
        if self.group_view.is_none() {
            self.readers_table(ui, &n);
        }

        if let Some((_, active, closed)) = &self.group_view {
            ui.add_space(12.0);
            ui.separator();
            let done = closed
                .iter()
                .filter(|a| a.closed.reason == CloseReason::Done)
                .count();
            ui.label(
                RichText::new(format!(
                    "同じ送信の状況 (既読・未読・完了): {} 件中 {} 件完了",
                    active.len() + closed.len(),
                    done
                ))
                .strong(),
            );
            egui::Grid::new("group-status")
                .striped(true)
                .num_columns(3)
                .show(ui, |ui| {
                    for a in active {
                        ui.label(self.data.label(a.target.kind, &a.target.name));
                        let rows = self.data.read_rows(a);
                        match rows.iter().find_map(|r| r.mark.as_ref()) {
                            Some(m) => {
                                ui.colored_label(WARN, "既読・未完了");
                                ui.label(format!("{} {}@{}", fmt(m.at), m.user, m.host));
                            }
                            None => {
                                ui.colored_label(BAD, "未読");
                                ui.label("");
                            }
                        }
                        ui.end_row();
                    }
                    for a in closed {
                        ui.label(self.data.label(a.notice.target.kind, &a.notice.target.name));
                        let c = if a.closed.reason == CloseReason::Done {
                            OK
                        } else {
                            MUTED
                        };
                        ui.colored_label(c, reason_label(a.closed.reason));
                        ui.label(format!(
                            "{} {}@{}",
                            fmt(a.closed.at),
                            a.closed.by.user,
                            a.closed.by.host
                        ));
                        ui.end_row();
                    }
                });
        }
    }

    /// 「既読にする」を押した人と、まだの人 (未読が上)。
    fn readers_table(&self, ui: &mut egui::Ui, n: &Notice) {
        let rows = self.data.read_rows(n);
        let read = rows.iter().filter(|r| r.mark.is_some()).count();
        ui.add_space(12.0);
        ui.separator();
        let head = match n.target.kind {
            Kind::Device if rows.is_empty() => {
                "既読: この PC でまだ誰も既読にしていません".to_string()
            }
            Kind::Device => format!("既読: この PC で {read} 人が既読にしました"),
            _ => {
                let done = rows
                    .iter()
                    .filter(|r| r.mark.as_ref().is_some_and(|m| m.done))
                    .count();
                if done > 0 {
                    format!("既読: {} 人中 {read} 人 (うち完了 {done} 人)", rows.len())
                } else {
                    format!("既読: {} 人中 {read} 人", rows.len())
                }
            }
        };
        ui.label(RichText::new(head).strong());
        if n.target.kind == Kind::Broadcast && rows.is_empty() {
            ui.label(
                RichText::new("名簿にユーザーがおらず、HatoPo を起動した人もまだいません。")
                    .color(MUTED),
            );
        }
        egui::Grid::new("readers")
            .striped(true)
            .num_columns(3)
            .show(ui, |ui| {
                for r in &rows {
                    ui.label(self.data.label(Kind::User, &r.user));
                    match &r.mark {
                        Some(m) => {
                            ui.colored_label(OK, if m.done { "完了" } else { "既読" });
                            ui.label(format!("{} ({})", fmt(m.at), m.host));
                        }
                        None => {
                            ui.colored_label(BAD, "未読");
                            ui.label("");
                        }
                    }
                    ui.end_row();
                }
            });
    }

    fn save_edit(&mut self, n: &Notice, others: &[Notice]) {
        let Some(form) = &self.edit else { return };
        let edit = form.to_new();
        let t = now();
        match self
            .loft
            .update(&n.id, self.edit_revision, &edit, &self.me, t)
        {
            Ok(_) => {
                let mut extra = 0;
                let mut errors = Vec::new();
                if self.apply_to_group {
                    for o in others {
                        match self.loft.update(&o.id, o.revision, &edit, &self.me, t) {
                            Ok(_) => extra += 1,
                            Err(e) => errors.push(e.to_string()),
                        }
                    }
                }
                let id = n.id.clone();
                self.reload();
                if let Some(fresh) = self.data.notices.iter().find(|x| x.id == id).cloned() {
                    self.select(&fresh);
                }
                if errors.is_empty() {
                    self.ok(if extra > 0 {
                        format!("保存しました (同じ送信の {extra} 件も)。")
                    } else {
                        "保存しました。".into()
                    });
                } else {
                    self.fail(format!(
                        "保存しましたが、同じ送信のうち {} 件は直せませんでした: {}",
                        errors.len(),
                        errors.join(" / ")
                    ));
                }
            }
            Err(e @ Error::Conflict { .. }) => {
                self.fail(format!(
                    "{e}\n［再読込］すると最新の内容が出ます (入力した内容は消えます)。"
                ));
            }
            Err(e) => self.fail(e.to_string()),
        }
    }

    fn delete_selected(&mut self, n: &Notice, others: &[Notice]) {
        let t = now();
        let mut targets = vec![n.id.clone()];
        if self.apply_to_group {
            targets.extend(others.iter().map(|o| o.id.clone()));
        }
        let mut done = 0;
        let mut errors = Vec::new();
        for id in &targets {
            match self.loft.close(id, CloseReason::Deleted, &self.me, t) {
                Ok(_) | Err(Error::NotFound { .. }) => done += 1,
                Err(e) => errors.push(e.to_string()),
            }
        }
        self.close_editor();
        self.reload();
        if errors.is_empty() {
            self.ok(format!("{done} 件を削除しました (archive へ移しました)。"));
        } else {
            self.fail(format!(
                "{done} 件を削除しました。失敗: {}",
                errors.join(" / ")
            ));
        }
    }

    // ------------------------------------------------------------------ 新しく送る
    fn compose(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("新しく送る");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("宛先").strong());
                for k in [Kind::User, Kind::Device, Kind::Broadcast] {
                    let label = match k {
                        Kind::User => "ユーザー",
                        Kind::Device => "端末 (PC)",
                        Kind::Broadcast => "全員 (一斉)",
                    };
                    if ui.radio_value(&mut self.c_kind, k, label).changed() {
                        self.c_selected.clear();
                    }
                }
            });
            if self.c_kind != Kind::Broadcast {
                self.target_picker(ui);
            } else {
                ui.label(
                    RichText::new("全員の一覧のいちばん上に出ます。［完了］ボタンを出すと、押した人の画面だけ消えます。出さなければ、管理者が削除するか終了日で片付きます。")
                        .color(MUTED),
                );
            }
            ui.add_space(8.0);
            // 完了ボタンは、触るまで宛先の種類の既定 (全員宛ては出さない) に従う
            if !self.c_form.complete_touched {
                self.c_form.complete_button = self.c_kind != Kind::Broadcast;
            }
            self.c_form.ui(ui, "compose");
            ui.add_space(10.0);
            let count = if self.c_kind == Kind::Broadcast {
                1
            } else {
                self.c_selected.len()
            };
            ui.horizontal(|ui| {
                let label = if self.c_kind == Kind::Broadcast {
                    "全員に送る".to_string()
                } else {
                    format!("{count} 件送る")
                };
                let btn = ui.add_enabled(
                    count > 0,
                    egui::Button::new(RichText::new(label).color(Color32::WHITE).size(15.0))
                        .fill(OK),
                );
                if btn.clicked() {
                    self.send();
                }
                if count == 0 {
                    ui.label(RichText::new("宛先を 1 つ以上選んでください。").color(WARN));
                }
            });
        });
    }

    fn target_picker(&mut self, ui: &mut egui::Ui) {
        let kind = self.c_kind;
        let rows = member_rows(
            kind,
            &self.data.directory,
            &self.data.statuses,
            &self.data.notices,
        );
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.c_search)
                    .hint_text("名前・ID で絞り込む")
                    .desired_width(200.0),
            );
            if ui.button("表示中を全部選ぶ").clicked() {
                let q = self.c_search.to_lowercase();
                for r in rows.iter().filter(|r| matches(r, &q)) {
                    self.c_selected.insert(r.member.id.clone());
                }
            }
            if ui.button("選択を外す").clicked() {
                self.c_selected.clear();
            }
        });
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .id_salt("picker")
                .max_height(180.0)
                .show(ui, |ui| {
                    let q = self.c_search.to_lowercase();
                    if rows.is_empty() {
                        ui.label(
                            RichText::new("名簿が空です。下の欄に ID を入れて足すか、［ユーザー］［端末］の画面で名簿を作ってください。")
                                .color(MUTED),
                        );
                    }
                    ui.horizontal_wrapped(|ui| {
                        for r in rows.iter().filter(|r| matches(r, &q)) {
                            let mut on = self.c_selected.contains(&r.member.id);
                            let label = if r.member.name.is_empty() {
                                r.member.id.clone()
                            } else {
                                format!("{} ({})", r.member.id, r.member.name)
                            };
                            if ui.checkbox(&mut on, label).changed() {
                                if on {
                                    self.c_selected.insert(r.member.id.clone());
                                } else {
                                    self.c_selected.remove(&r.member.id);
                                }
                            }
                        }
                    });
                });
        });
        ui.horizontal(|ui| {
            ui.label("名簿に無い宛先");
            ui.add(
                egui::TextEdit::singleline(&mut self.c_extra)
                    .hint_text(if kind == Kind::User {
                        "ログオン名 (カンマ区切りで複数)"
                    } else {
                        "コンピューター名 (カンマ区切りで複数)"
                    })
                    .desired_width(280.0),
            );
            if ui.button("足す").clicked() {
                for id in self.c_extra.split([',', '、', ' ']) {
                    let id = id.trim().to_lowercase();
                    if !id.is_empty() {
                        self.c_selected.insert(id);
                    }
                }
                self.c_extra.clear();
            }
        });
        if !self.c_selected.is_empty() {
            ui.label(
                RichText::new(format!(
                    "選んだ宛先 ({}): {}",
                    self.c_selected.len(),
                    self.c_selected
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
                .color(MUTED),
            );
        }
    }

    fn send(&mut self) {
        let targets: Result<Vec<Target>, Error> = if self.c_kind == Kind::Broadcast {
            Ok(vec![Target::broadcast()])
        } else {
            self.c_selected
                .iter()
                .map(|id| Target::new(self.c_kind, id))
                .collect()
        };
        let result = targets.and_then(|t| {
            self.loft
                .add_many(&t, &self.c_form.to_new(), &self.me, now())
        });
        match result {
            Ok(made) => {
                let n = made.len();
                self.c_form = NoticeForm::new(now().date());
                self.c_selected.clear();
                self.reload();
                self.ok(format!(
                    "{n} 件送りました。［通知］の画面で確かめられます。"
                ));
            }
            Err(e) => self.fail(e.to_string()),
        }
    }

    // ------------------------------------------------------------------ 完了の記録
    fn archive(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("月");
            let before = self.a_month.clone();
            egui::ComboBox::from_id_salt("a-month")
                .selected_text(if self.a_month.is_empty() {
                    "(記録なし)".to_string()
                } else {
                    self.a_month.clone()
                })
                .show_ui(ui, |ui| {
                    for m in self.data.months.clone() {
                        ui.selectable_value(&mut self.a_month, m.clone(), m);
                    }
                });
            if before != self.a_month {
                self.load_archive();
            }
            ui.label("理由");
            egui::ComboBox::from_id_salt("a-reason")
                .selected_text(self.a_reason.map(reason_label).unwrap_or("すべて"))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.a_reason, None, "すべて");
                    for r in [
                        CloseReason::Done,
                        CloseReason::Expired,
                        CloseReason::Deleted,
                    ] {
                        ui.selectable_value(&mut self.a_reason, Some(r), reason_label(r));
                    }
                });
            ui.label("検索");
            ui.add(
                egui::TextEdit::singleline(&mut self.a_keyword)
                    .hint_text("件名・宛先・完了した人")
                    .desired_width(200.0),
            );
        });
        let kw = self.a_keyword.trim().to_lowercase();
        let rows: Vec<&Archived> = self
            .a_rows
            .iter()
            .filter(|a| self.a_reason.is_none_or(|r| a.closed.reason == r))
            .filter(|a| {
                kw.is_empty()
                    || a.notice.title.to_lowercase().contains(&kw)
                    || a.notice.target.name.contains(&kw)
                    || a.closed.by.user.contains(&kw)
                    || a.closed.by.host.contains(&kw)
            })
            .collect();
        let done = rows
            .iter()
            .filter(|a| a.closed.reason == CloseReason::Done)
            .count();
        ui.label(
            RichText::new(format!(
                "{} 件 (完了 {} / 期限切れ {} / 削除 {})",
                rows.len(),
                done,
                rows.iter()
                    .filter(|a| a.closed.reason == CloseReason::Expired)
                    .count(),
                rows.iter()
                    .filter(|a| a.closed.reason == CloseReason::Deleted)
                    .count()
            ))
            .color(MUTED),
        );
        if rows.is_empty() {
            ui.add_space(20.0);
            ui.label(RichText::new("この月の記録はありません。").color(MUTED));
            return;
        }
        let groups = group_by_key(&rows, |a| a.notice.group.as_deref());
        ui.label(
            RichText::new(
                "左端の「閉じた日時」をクリックすると、誰がいつ完了したか・未完了の人の一覧が開きます。文字はドラッグで選んでコピーできます。",
            )
            .small()
            .color(MUTED),
        );
        let mut clicked: Option<Vec<Archived>> = None;
        egui::ScrollArea::horizontal().show(ui, |ui| {
            TableBuilder::new(ui)
                .striped(true)
                .resizable(true)
                .sense(egui::Sense::click())
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::exact(124.0))
                .column(Column::exact(100.0))
                .column(Column::initial(200.0).at_least(100.0))
                .column(Column::exact(64.0))
                .column(Column::initial(170.0).at_least(80.0))
                .column(Column::remainder().at_least(160.0))
                .header(24.0, |mut h| {
                    for c in [
                        "閉じた日時",
                        "理由",
                        "誰が (ユーザー@PC)",
                        "種類",
                        "宛先",
                        "件名",
                    ] {
                        h.col(|ui| {
                            ui.strong(c);
                        });
                    }
                })
                .body(|mut body| {
                    for g in &groups {
                        let a = g[0];
                        let multi = g.len() > 1;
                        body.row(24.0, |mut row| {
                            row.set_selected(
                                self.a_selected
                                    .as_deref()
                                    .is_some_and(|s| g.iter().any(|m| m.notice.id == s)),
                            );
                            row.col(|ui| {
                                let at = g.iter().map(|m| m.closed.at).max().unwrap_or(a.closed.at);
                                // 選択できない文字にして、行のクリックを受ける
                                ui.add(egui::Label::new(fmt(at)).selectable(false));
                            });
                            row.col(|ui| {
                                if multi {
                                    let done = g
                                        .iter()
                                        .filter(|m| m.closed.reason == CloseReason::Done)
                                        .count();
                                    ui.label(format!("完了 {done}/{}", g.len()));
                                } else {
                                    let c = match a.closed.reason {
                                        CloseReason::Done => OK,
                                        CloseReason::Expired => MUTED,
                                        CloseReason::Deleted => BAD,
                                    };
                                    ui.colored_label(c, reason_label(a.closed.reason));
                                }
                            });
                            row.col(|ui| {
                                let who: Vec<String> = g
                                    .iter()
                                    .map(|m| {
                                        format!(
                                            "{} ({})",
                                            self.data.label(Kind::User, &m.closed.by.user),
                                            m.closed.by.host
                                        )
                                    })
                                    .collect();
                                ui.label(who.join(" / "));
                            });
                            row.col(|ui| {
                                ui.label(kind_label(a.notice.target.kind));
                            });
                            row.col(|ui| {
                                if multi {
                                    ui.label(self.data.target_summary(
                                        a.notice.target.kind,
                                        &a.notice.target.name,
                                        g.len(),
                                    ));
                                } else {
                                    ui.label(
                                        self.data
                                            .label(a.notice.target.kind, &a.notice.target.name),
                                    );
                                }
                            });
                            row.col(|ui| {
                                ui.label(&a.notice.title);
                            });
                            if row.response().clicked() {
                                clicked = Some(g.iter().map(|m| (*m).clone()).collect());
                            }
                        });
                    }
                });
        });
        if let Some(g) = clicked {
            self.select_archived(&g);
        }
    }

    /// 完了の記録の行を開く。同じ送信 (組) なら、全員分と、まだ未完了の通知も集める。
    fn select_archived(&mut self, g: &[Archived]) {
        let first = &g[0];
        self.a_selected = Some(first.notice.id.clone());
        let found = first.notice.group.as_deref().and_then(|gid| {
            self.loft
                .group_status(gid, first.notice.created_at, now())
                .ok()
        });
        let (mut members, active) = match found {
            Some((active, closed)) if !closed.is_empty() => (closed, active),
            _ => (g.to_vec(), Vec::new()),
        };
        members.sort_by_key(|a| a.closed.at);
        self.a_detail = Some(ArchiveDetail { members, active });
    }

    /// 完了の記録の詳細。誰がいつ完了したか、まだの人は誰かを、通知の画面と同じ作りで見せる。
    fn archive_detail(&mut self, ui: &mut egui::Ui) {
        let Some(d) = &self.a_detail else {
            return;
        };
        let Some(first) = d.members.first() else {
            return;
        };
        let n = &first.notice;
        let mut close = false;
        let mut copy = false;
        ui.horizontal(|ui| {
            ui.heading("完了の記録");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("閉じる").clicked() {
                    close = true;
                }
                if ui.button("全文をコピー").clicked() {
                    copy = true;
                }
            });
        });

        // 表示とコピーで同じ行を使う: (宛先, 状態, 色, 日時, 誰が)
        let mut rows: Vec<(String, String, Color32, String, String)> = Vec::new();
        for a in &d.members {
            let c = match a.closed.reason {
                CloseReason::Done => OK,
                CloseReason::Expired => MUTED,
                CloseReason::Deleted => BAD,
            };
            rows.push((
                self.data.label(a.notice.target.kind, &a.notice.target.name),
                reason_label(a.closed.reason).to_string(),
                c,
                fmt(a.closed.at),
                format!(
                    "{}@{}",
                    self.data.label(Kind::User, &a.closed.by.user),
                    a.closed.by.host
                ),
            ));
        }
        for a in &d.active {
            let mark = self.data.read_rows(a).into_iter().find_map(|r| r.mark);
            let who = mark
                .as_ref()
                .map(|m| format!("{}@{}", m.user, m.host))
                .unwrap_or_default();
            let at = mark.as_ref().map(|m| fmt(m.at)).unwrap_or_default();
            let (state, c) = if mark.is_some() {
                ("未完了 (既読)", WARN)
            } else {
                ("未完了 (未読)", BAD)
            };
            rows.push((
                self.data.label(a.target.kind, &a.target.name),
                state.to_string(),
                c,
                at,
                who,
            ));
        }
        let count = |r: CloseReason| d.members.iter().filter(|a| a.closed.reason == r).count();
        let summary = format!(
            "全 {} 件: 完了 {} / 未完了 {} / 期限切れ {} / 削除 {}",
            d.members.len() + d.active.len(),
            count(CloseReason::Done),
            d.active.len(),
            count(CloseReason::Expired),
            count(CloseReason::Deleted),
        );
        // 一斉・端末宛ては宛先が人ではないので、読んだ人の一覧も出す
        let readers = if d.members.len() == 1 && n.target.kind != Kind::User {
            read_rows(n, &first.read_by, &self.data.directory, &self.data.statuses)
        } else {
            Vec::new()
        };

        ui.label(
            RichText::new(format!(
                "{} ・ {}\n作成 {} ({}@{})",
                kind_label(n.target.kind),
                self.data.label(n.target.kind, &n.target.name),
                fmt(n.created_at),
                n.created_by.user,
                n.created_by.host,
            ))
            .color(MUTED),
        );
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(&n.title).strong().size(16.0)).selectable(true));
        if !n.body.is_empty() {
            ui.add(egui::Label::new(&n.body).selectable(true));
        }
        for u in &n.urls {
            ui.add(egui::Label::new(u).selectable(true));
        }
        if let Some(due) = n.due {
            ui.label(format!("依頼期限 {}", due.format("%Y/%m/%d")));
        }
        ui.add_space(8.0);
        ui.separator();
        ui.label(RichText::new(&summary).strong());
        egui::Grid::new("archive-status")
            .striped(true)
            .num_columns(4)
            .show(ui, |ui| {
                for (target, state, c, at, who) in &rows {
                    ui.label(target);
                    ui.colored_label(*c, state);
                    ui.label(at);
                    ui.label(who);
                    ui.end_row();
                }
            });
        if !readers.is_empty() {
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!(
                    "既読: {} 人中 {} 人",
                    readers.len(),
                    readers.iter().filter(|r| r.mark.is_some()).count()
                ))
                .strong(),
            );
            egui::Grid::new("archive-readers")
                .striped(true)
                .num_columns(3)
                .show(ui, |ui| {
                    for r in &readers {
                        ui.label(self.data.label(Kind::User, &r.user));
                        match &r.mark {
                            Some(m) => {
                                ui.colored_label(OK, if m.done { "完了" } else { "既読" });
                                ui.label(format!("{} ({})", fmt(m.at), m.host));
                            }
                            None => {
                                ui.colored_label(BAD, "未読");
                                ui.label("");
                            }
                        }
                        ui.end_row();
                    }
                });
        }

        if copy {
            let mut text = format!(
                "{}\n{} ・ {}\n{}\n",
                n.title,
                kind_label(n.target.kind),
                self.data.label(n.target.kind, &n.target.name),
                summary
            );
            for (target, state, _, at, who) in &rows {
                text.push_str(&format!("{target}\t{state}\t{at}\t{who}\n"));
            }
            for r in &readers {
                let (st, at) = match &r.mark {
                    Some(m) => (if m.done { "完了" } else { "既読" }, fmt(m.at)),
                    None => ("未読", String::new()),
                };
                text.push_str(&format!("{}\t{st}\t{at}\n", r.user));
            }
            ui.ctx().copy_text(text);
            self.ok("内容をコピーしました。");
        }
        if close {
            self.a_selected = None;
            self.a_detail = None;
        }
    }

    // ------------------------------------------------------------------ 名簿
    fn members(&mut self, ui: &mut egui::Ui, kind: Kind) {
        let (title, id_label, other_label) = if kind == Kind::User {
            ("ユーザー", "ログオン名", "最後に使った PC")
        } else {
            ("端末", "コンピューター名", "最後のユーザー")
        };
        if let Some(e) = self.data.directory_error.clone() {
            ui.colored_label(
                BAD,
                format!("名簿 (directory.json) が読めないので編集を止めています: {e}"),
            );
            return;
        }
        let rows = member_rows(
            kind,
            &self.data.directory,
            &self.data.statuses,
            &self.data.notices,
        );
        let unregistered = rows.iter().filter(|r| !r.registered).count();
        ui.horizontal_wrapped(|ui| {
            ui.heading(format!("{title}の名簿"));
            if ui.button(format!("＋ {title}を足す")).clicked() {
                self.m_edit = Some(MemberEdit {
                    kind,
                    member: Member::default(),
                    original: None,
                    confirm_delete: false,
                });
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.m_search)
                    .hint_text("絞り込む")
                    .desired_width(180.0),
            );
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button(format!("CSV から{title}を取り込む…")).clicked() {
                self.import_members_from_csv(kind, id_label);
            }
            ui.label(
                RichText::new(format!(
                    "{id_label},名前,メモ の順に入れた CSV (UTF-8 または Shift-JIS) を選びます。\
1 行目が見出しなら自動で読み飛ばします。既に名簿にある{id_label}は上書きされます。"
                ))
                .small()
                .color(MUTED),
            );
        });
        if let Some(errors) = self
            .m_import
            .as_ref()
            .filter(|i| i.kind == kind)
            .map(|i| i.result.errors.clone())
        {
            self.show_import_errors(ui, &errors);
        }
        if unregistered > 0 {
            ui.colored_label(
                WARN,
                format!(
                    "HatoPo は動いているが名簿に無い{title}が {unregistered} 件あります (薄い行)。行をクリックすると名簿に足せます。"
                ),
            );
        }
        if self.m_edit.as_ref().is_some_and(|e| e.kind == kind) {
            self.member_editor(ui, id_label);
        }
        let t = now();
        let mine = env!("CARGO_PKG_VERSION");
        let q = self.m_search.to_lowercase();
        let mut action: Option<MemberRow> = None;
        ui.label(
            RichText::new(
                "左端の ID をクリックすると直せます。文字はドラッグで選んでコピーできます。",
            )
            .small()
            .color(MUTED),
        );
        egui::ScrollArea::horizontal().show(ui, |ui| {
            TableBuilder::new(ui)
                .striped(true)
                .resizable(true)
                .sense(egui::Sense::click())
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::initial(130.0).at_least(90.0))
                .column(Column::initial(130.0).at_least(80.0))
                .column(Column::exact(124.0))
                .column(Column::exact(124.0))
                .column(Column::initial(110.0).at_least(80.0))
                .column(Column::exact(56.0))
                .column(Column::exact(52.0))
                .column(Column::remainder().at_least(80.0))
                .header(24.0, |mut h| {
                    for c in [
                        id_label,
                        "名前",
                        "最終アクセス",
                        "最後に完了",
                        other_label,
                        "版",
                        "未完了",
                        "メモ",
                    ] {
                        h.col(|ui| {
                            ui.strong(c);
                        });
                    }
                })
                .body(|mut body| {
                    for r in rows.iter().filter(|r| matches(r, &q)) {
                        body.row(26.0, |mut row| {
                            let weak = |s: String| {
                                if r.registered {
                                    RichText::new(s)
                                } else {
                                    RichText::new(s).color(MUTED)
                                }
                            };
                            row.col(|ui| {
                                // 選択できない文字にして、行のクリック (編集) を受ける
                                ui.add(egui::Label::new(weak(r.member.id.clone())).selectable(false));
                            });
                            row.col(|ui| {
                                ui.label(weak(r.member.name.clone()));
                            });
                            row.col(|ui| {
                                let stale = r
                                    .last_seen
                                    .is_none_or(|s| (t - s).num_days() >= STALE_DAYS);
                                let text = r
                                    .last_seen
                                    .map(fmt)
                                    .unwrap_or_else(|| "まだ無し".into());
                                if stale {
                                    ui.colored_label(WARN, text)
                                } else {
                                    ui.label(text)
                                }
                                .on_hover_text(format!(
                                    "{STALE_DAYS} 日以上アクセスが無いと黄色になります"
                                ));
                            });
                            row.col(|ui| {
                                let resp = ui.label(fmt_opt(r.last_done_at));
                                if let Some(title) = r.last_done_title.as_deref().filter(|t| !t.is_empty()) {
                                    resp.on_hover_text(format!("完了した通知: {title}"));
                                }
                            });
                            row.col(|ui| {
                                ui.label(&r.counterpart);
                            });
                            row.col(|ui| {
                                if is_older(&r.version, mine) {
                                    ui.colored_label(WARN, &r.version).on_hover_text(format!(
                                        "管理画面 ({mine}) より古い版です。共有の HatoPo.exe を新しくすると、次の確認で入れ替わります"
                                    ));
                                } else {
                                    ui.label(&r.version);
                                }
                            });
                            row.col(|ui| {
                                ui.label(r.pending.to_string());
                            });
                            row.col(|ui| {
                                ui.label(&r.member.note);
                            });
                            if row.response().clicked() {
                                action = Some(r.clone());
                            }
                        });
                    }
                });
        });
        if let Some(r) = action {
            self.m_edit = Some(MemberEdit {
                kind,
                original: r.registered.then(|| r.member.id.clone()),
                member: r.member,
                confirm_delete: false,
            });
        }
    }

    fn member_editor(&mut self, ui: &mut egui::Ui, id_label: &str) {
        let mut save = false;
        let mut delete = false;
        let mut cancel = false;
        let Some(e) = self.m_edit.as_mut() else {
            return;
        };
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(if e.original.is_some() {
                    "名簿の行を直す"
                } else {
                    "名簿に足す"
                })
                .strong(),
            );
            egui::Grid::new("member-edit")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label(format!("{id_label} *"));
                    ui.add(
                        egui::TextEdit::singleline(&mut e.member.id)
                            .hint_text(if e.kind == Kind::User {
                                "%USERNAME% と同じ"
                            } else {
                                "%COMPUTERNAME% と同じ"
                            })
                            .desired_width(240.0),
                    );
                    ui.end_row();
                    ui.label("名前");
                    ui.add(
                        egui::TextEdit::singleline(&mut e.member.name)
                            .hint_text(if e.kind == Kind::User {
                                "氏名"
                            } else {
                                "設置場所など"
                            })
                            .desired_width(240.0),
                    );
                    ui.end_row();
                    ui.label("メモ");
                    ui.add(egui::TextEdit::singleline(&mut e.member.note).desired_width(360.0));
                    ui.end_row();
                });
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Button::new(RichText::new("名簿に保存").color(Color32::WHITE))
                            .fill(OK),
                    )
                    .clicked()
                {
                    save = true;
                }
                if ui.button("やめる").clicked() {
                    cancel = true;
                }
                if e.original.is_some() {
                    if e.confirm_delete {
                        ui.label("名簿から外します (通知や記録は消えません)。");
                        if ui
                            .add(
                                egui::Button::new(RichText::new("外す").color(Color32::WHITE))
                                    .fill(BAD),
                            )
                            .clicked()
                        {
                            delete = true;
                        }
                    } else if ui.button("名簿から外す…").clicked() {
                        e.confirm_delete = true;
                    }
                }
            });
        });
        ui.add_space(6.0);
        if cancel {
            self.m_edit = None;
        } else if save || delete {
            self.save_member(delete);
        }
    }

    fn save_member(&mut self, delete: bool) {
        let Some(e) = &self.m_edit else { return };
        let mut dir: Directory = self.data.directory.clone();
        let list = if e.kind == Kind::Device {
            &mut dir.devices
        } else {
            &mut dir.users
        };
        if let Some(orig) = &e.original {
            list.retain(|m| &m.id != orig);
        }
        if !delete {
            list.push(e.member.clone());
        }
        match dir.save(self.loft.root(), &self.me, now()) {
            Ok(saved) => {
                self.data.directory = saved;
                self.m_edit = None;
                self.ok(if delete {
                    "名簿から外しました。"
                } else {
                    "名簿に保存しました。"
                });
            }
            Err(e @ Error::Conflict { .. }) => self.fail(format!(
                "{e}\nほかの管理者が名簿を変えています。［再読込］してからやり直してください。"
            )),
            Err(e) => self.fail(e.to_string()),
        }
    }

    fn import_members_from_csv(&mut self, kind: Kind, id_label: &str) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(format!("{id_label}・名前・メモの順の CSV を選ぶ"))
            .add_filter("CSV", &["csv"])
            .pick_file()
        else {
            return;
        };
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                self.fail(format!("CSV ({}) が読めません: {e}", path.display()));
                return;
            }
        };
        let result = import_members_csv(&bytes);

        let mut dir: Directory = self.data.directory.clone();
        let list = if kind == Kind::Device {
            &mut dir.devices
        } else {
            &mut dir.users
        };
        let (added, updated) = merge_members(list, &result.members);

        if added == 0 && updated == 0 {
            if result.errors.is_empty() {
                self.fail("CSV から取り込める行がありませんでした。");
            } else {
                self.fail(format!(
                    "CSV の {} 件の行すべてで取り込みに失敗しました。下の一覧を確認してください。",
                    result.errors.len()
                ));
                self.m_import = Some(MemberImport { kind, result });
            }
            return;
        }

        match dir.save(self.loft.root(), &self.me, now()) {
            Ok(saved) => {
                self.data.directory = saved;
                let title = if kind == Kind::User {
                    "ユーザー"
                } else {
                    "端末"
                };
                let mut msg = format!(
                    "CSV から{title}を取り込みました (新規 {added} 件・更新 {updated} 件)。"
                );
                if !result.errors.is_empty() {
                    msg.push_str(&format!(
                        " {} 件はエラーで取り込めませんでした。下の一覧を確認してください。",
                        result.errors.len()
                    ));
                }
                self.ok(msg);
                self.m_import = if result.errors.is_empty() {
                    None
                } else {
                    Some(MemberImport { kind, result })
                };
            }
            Err(e @ Error::Conflict { .. }) => self.fail(format!(
                "{e}\nほかの管理者が名簿を変えています。［再読込］してからやり直してください。"
            )),
            Err(e) => self.fail(e.to_string()),
        }
    }

    fn show_import_errors(&mut self, ui: &mut egui::Ui, errors: &[hatopo_core::ImportError]) {
        if errors.is_empty() {
            return;
        }
        let mut close = false;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.colored_label(WARN, format!("取り込めなかった行 ({} 件):", errors.len()));
                if ui.small_button("閉じる").clicked() {
                    close = true;
                }
            });
            egui::ScrollArea::vertical()
                .max_height(120.0)
                .show(ui, |ui| {
                    for e in errors {
                        ui.label(RichText::new(e.to_string()).small());
                    }
                });
        });
        if close {
            self.m_import = None;
        }
    }

    // ------------------------------------------------------------------ 設定
    /// 鳩の巣の `character_icon\` にあるキャラクターごとに、持っている絵を並べる。
    fn character_gallery(&mut self, ui: &mut egui::Ui, chars: &[String]) {
        if chars.is_empty() {
            ui.label(
                RichText::new("鳩の巣の character_icon フォルダにキャラクターがありません。")
                    .color(WARN),
            );
            return;
        }
        ui.label(
            RichText::new(
                "［みんなの既定にする］を押したあと、下の［保存］で settings.json に書きます。絵が無いポーズは、利用者の HatoPo では同梱の HatoPoBlue の絵で補います。",
            )
            .small()
            .color(MUTED),
        );
        let root = self.loft.root().join("character_icon");
        for name in chars {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(name).strong());
                    if *name == self.s_edit.character {
                        ui.colored_label(OK, "みんなの既定");
                    } else if ui.button("みんなの既定にする").clicked() {
                        self.s_edit.character = name.clone();
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    for (stem, label) in POSE_LABELS {
                        let path = root.join(name).join("poses").join(format!("{stem}.png"));
                        ui.vertical(|ui| {
                            ui.set_width(104.0);
                            match self.pose_texture(ui.ctx(), &path) {
                                Some(t) => {
                                    ui.add(
                                        egui::Image::new(&t)
                                            .fit_to_exact_size(egui::Vec2::splat(96.0)),
                                    );
                                }
                                None => {
                                    let (r, _) = ui.allocate_exact_size(
                                        egui::Vec2::splat(96.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().rect_stroke(
                                        r,
                                        4.0,
                                        egui::Stroke::new(1.0_f32, MUTED),
                                        egui::StrokeKind::Inside,
                                    );
                                    ui.painter().text(
                                        r.center(),
                                        egui::Align2::CENTER_CENTER,
                                        "絵なし",
                                        egui::FontId::proportional(13.0),
                                        MUTED,
                                    );
                                }
                            }
                            ui.label(RichText::new(label).small());
                        });
                    }
                });
            });
        }
    }

    fn pose_texture(
        &mut self,
        ctx: &egui::Context,
        path: &std::path::Path,
    ) -> Option<egui::TextureHandle> {
        if let Some(t) = self.textures.get(path) {
            return t.clone();
        }
        let tex = std::fs::read(path)
            .ok()
            .and_then(|b| image::load_from_memory_with_format(&b, image::ImageFormat::Png).ok())
            .map(|img| {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                ctx.load_texture(
                    path.display().to_string(),
                    egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
                    egui::TextureOptions::LINEAR,
                )
            });
        self.textures.insert(path.to_path_buf(), tex.clone());
        tex
    }

    fn settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("設定 (settings.json)");
        ui.label(
            RichText::new("利用者の HatoPo は次の確認のときに読み直します。手で編集せず、この画面から保存してください (書く前に値を確かめます)。")
                .color(MUTED),
        );
        if let Some(w) = &self.data.settings_warning {
            ui.colored_label(
                WARN,
                format!("今の settings.json: {w} (利用者は既定値で動いています)"),
            );
        }
        ui.add_space(8.0);
        let chars = Settings::characters(self.loft.root());
        let label = if self.show_gallery {
            "キャラクターの一覧を閉じる"
        } else {
            "キャラクターの絵を一覧で見る"
        };
        if ui.button(label).clicked() {
            self.show_gallery = !self.show_gallery;
            self.textures.clear(); // 開くたびに読み直す (絵を足した直後でも見える)
        }
        if self.show_gallery {
            self.character_gallery(ui, &chars);
        }
        ui.add_space(8.0);
        egui::Grid::new("settings")
            .num_columns(3)
            .spacing([16.0, 10.0])
            .show(ui, |ui| {
                ui.label("キャラクター");
                egui::ComboBox::from_id_salt("character")
                    .selected_text(&self.s_edit.character)
                    .show_ui(ui, |ui| {
                        for c in &chars {
                            ui.selectable_value(&mut self.s_edit.character, c.clone(), c);
                        }
                    });
                ui.label(
                    RichText::new(
                        "みんなの既定。利用者が自分で選んでいればそちらが先。絵が足りなければ同梱の HatoPoBlue を使う",
                    )
                    .color(MUTED),
                );
                ui.end_row();

                ui.label("確認の間隔");
                ui.add(
                    egui::DragValue::new(&mut self.s_edit.poll_seconds)
                        .range(MIN_POLL_SECONDS..=MAX_POLL_SECONDS)
                        .suffix(" 秒"),
                );
                ui.label(
                    RichText::new("共有を見に行く間隔。短いほど早く届くが共有への負荷が増える")
                        .color(MUTED),
                );
                ui.end_row();

                ui.label("催促の間隔");
                ui.add(
                    egui::DragValue::new(&mut self.s_edit.remind_minutes)
                        .range(0..=MAX_REMIND_MINUTES)
                        .suffix(" 分"),
                );
                ui.label(
                    RichText::new("未完了の便りをトーストで出し直す間隔。0 で催促しない")
                        .color(MUTED),
                );
                ui.end_row();

                ui.label("マスコット");
                ui.checkbox(&mut self.s_edit.show_mascot, "デスクトップに鳩を出す");
                ui.label(RichText::new("外すとタスクトレイだけになる").color(MUTED));
                ui.end_row();

                ui.label("既定の大きさ");
                egui::ComboBox::from_id_salt("mascot-scale")
                    .selected_text(format!("{}%", self.s_edit.mascot_scale))
                    .show_ui(ui, |ui| {
                        for p in MASCOT_SCALES {
                            ui.selectable_value(&mut self.s_edit.mascot_scale, p, format!("{p}%"));
                        }
                    });
                ui.label(
                    RichText::new("利用者が自分で選んでいなければこの大きさ。各自はトレイ・右クリック・Ctrl + ホイールで変えられる")
                        .color(MUTED),
                );
                ui.end_row();
            });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new(RichText::new("保存").color(Color32::WHITE)).fill(OK))
                .clicked()
            {
                match self.s_edit.save(self.loft.root()) {
                    Ok(()) => {
                        self.reload();
                        self.ok("設定を保存しました。各 PC は次の確認で読み直します。");
                    }
                    Err(e) => self.fail(e.to_string()),
                }
            }
            if ui.button("元に戻す").clicked() {
                self.s_edit = self.data.settings.clone();
            }
            if ui.button("既定値にする").clicked() {
                self.s_edit = Settings::default();
            }
        });

        ui.add_space(20.0);
        ui.separator();
        ui.label(RichText::new("鳩の巣を作る").strong());
        ui.label(
            RichText::new("空のフォルダを鳩の巣にします。broadcast / PigeonHole / archive / status のフォルダと settings.json を作ります (あるものは触りません)。")
                .color(MUTED),
        );
        if ui.button("このフォルダに鳩の巣を作る").clicked() {
            match init_loft(self.loft.root(), &self.s_edit) {
                Ok(()) => {
                    self.reload();
                    self.ok("鳩の巣を用意しました。");
                }
                Err(e) => self.fail(e),
            }
        }
    }
}

fn matches(r: &MemberRow, q: &str) -> bool {
    q.is_empty()
        || r.member.id.contains(q)
        || r.member.name.to_lowercase().contains(q)
        || r.member.note.to_lowercase().contains(q)
}

/// 鳩の巣のフォルダ一式と settings.json を作る。既にあるものは触らない。
fn init_loft(root: &std::path::Path, settings: &Settings) -> Result<(), String> {
    for d in [
        "broadcast",
        "PigeonHole/user",
        "PigeonHole/device",
        "archive",
        "status",
        "read",
        "character_icon",
    ] {
        let p = root.join(d);
        std::fs::create_dir_all(&p).map_err(|e| format!("{} を作れません: {e}", p.display()))?;
    }
    if !root.join("settings.json").exists() {
        settings.save(root).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_creates_folders_and_keeps_existing_settings() {
        let d = tempfile::tempdir().unwrap();
        let custom = Settings {
            poll_seconds: 30,
            ..Default::default()
        };
        custom.save(d.path()).unwrap();
        init_loft(d.path(), &Settings::default()).unwrap();
        for p in [
            "broadcast",
            "PigeonHole/user",
            "PigeonHole/device",
            "archive",
            "status",
            "read",
        ] {
            assert!(d.path().join(p).is_dir(), "{p}");
        }
        assert_eq!(
            Settings::load(d.path()).0.poll_seconds,
            30,
            "既存の設定は上書きしない"
        );
    }
}
