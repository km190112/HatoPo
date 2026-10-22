//! 画面: デスクトップのマスコット (透過・最前面) と、通知の一覧の窓と、タスクトレイ。

use crate::assets::{self, Pose, Rgba, Tray};
use crate::worker::{Cmd, Flash, SharedRef, UiMsg};
use eframe::egui::{
    self, Align, Align2, Color32, ColorImage, CornerRadius, FontId, Layout, Margin, Pos2, Rect,
    RichText, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Vec2, ViewportBuilder,
    ViewportCommand, ViewportId,
};
use hatopo_core::{Kind, Notice};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

pub const MASCOT_SIZE: [f32; 2] = [220.0, 210.0];
const LIST_SIZE: [f32; 2] = [560.0, 640.0];
/// 一覧の窓を画面の右端からどれだけ離すか
const LIST_MARGIN: f32 = 24.0;
/// マスコットを隠すときは画面の外へ出す。窓を消すと画面の更新が止まり、トレイも止まるため
const OFFSCREEN: [f32; 2] = [-32000.0, -32000.0];
const FLASH_FOR: Duration = Duration::from_secs(5);
const BUBBLE_FOR: Duration = Duration::from_secs(12);

const BLUE: Color32 = Color32::from_rgb(0x1E, 0x63, 0xD0);
const BLUE_SOFT: Color32 = Color32::from_rgb(0xE6, 0xEF, 0xFC);
const INK: Color32 = Color32::from_rgb(0x1A, 0x24, 0x36);
const MUTED: Color32 = Color32::from_rgb(0x5A, 0x67, 0x7D);
const RED: Color32 = Color32::from_rgb(0xD9, 0x3A, 0x36);
const AMBER_BG: Color32 = Color32::from_rgb(0xFF, 0xF3, 0xD6);
const RED_BG: Color32 = Color32::from_rgb(0xFD, 0xE7, 0xE6);

pub struct App {
    shared: SharedRef,
    cmd_tx: Sender<Cmd>,
    ui_rx: Receiver<UiMsg>,
    loft_root: PathBuf,
    character: String,
    poses: HashMap<Pose, TextureHandle>,
    tray_imgs: HashMap<Tray, Rgba>,
    tray: Option<TrayIcon>,
    autostart_item: Option<CheckMenuItem>,
    tray_key: (Tray, usize, String),
    show_list: bool,
    focus_list: bool,
    open: Option<String>,
    confirm: Option<String>,
    bubble_since: Option<(String, Instant)>,
    last_bubble_key: String,
    last_pos: Option<Pos2>,
    pos_dirty_since: Option<Instant>,
    hidden_applied: Option<bool>,
    window_icon: egui::IconData,
    restart_exe: Option<PathBuf>,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        shared: SharedRef,
        cmd_tx: Sender<Cmd>,
        ui_tx: Sender<UiMsg>,
        ui_rx: Receiver<UiMsg>,
        loft_root: PathBuf,
        restart_exe: Option<PathBuf>,
    ) -> App {
        setup_fonts(&cc.egui_ctx);
        setup_style(&cc.egui_ctx);
        let character = shared.lock().unwrap().snap.settings.character.clone();
        let mut app = App {
            shared,
            cmd_tx: cmd_tx.clone(),
            ui_rx,
            loft_root,
            character: String::new(),
            poses: HashMap::new(),
            tray_imgs: HashMap::new(),
            tray: None,
            autostart_item: None,
            tray_key: (Tray::Inactive, usize::MAX, String::new()),
            show_list: false,
            focus_list: false,
            open: None,
            confirm: None,
            bubble_since: None,
            last_bubble_key: String::new(),
            last_pos: None,
            pos_dirty_since: None,
            hidden_applied: None,
            window_icon: window_icon(),
            restart_exe,
        };
        app.load_character(&cc.egui_ctx, &character);
        let installed = app.restart_exe.is_some();
        if let Some((tray, item)) =
            build_tray(&cc.egui_ctx, ui_tx, cmd_tx, &app.tray_imgs, installed)
        {
            app.tray = Some(tray);
            app.autostart_item = Some(item);
        }
        app
    }

    fn character_dir(&self, name: &str) -> Option<PathBuf> {
        let d = self.loft_root.join("character_icon").join(name);
        d.is_dir().then_some(d)
    }

    fn load_character(&mut self, ctx: &egui::Context, name: &str) {
        let dir = self.character_dir(name);
        for p in Pose::ALL {
            let img = assets::load_pose(dir.as_deref(), p);
            let tex = ctx.load_texture(
                format!("pose-{}", p.file_stem()),
                ColorImage::from_rgba_unmultiplied(
                    [img.width as usize, img.height as usize],
                    &img.pixels,
                ),
                TextureOptions::LINEAR,
            );
            self.poses.insert(p, tex);
        }
        for t in Tray::ALL {
            self.tray_imgs
                .insert(t, assets::load_tray(dir.as_deref(), t));
        }
        self.character = name.to_string();
        self.tray_key.1 = usize::MAX; // トレイを描き直させる
    }

    fn handle_messages(&mut self, ctx: &egui::Context) {
        while let Ok(m) = self.ui_rx.try_recv() {
            match m {
                UiMsg::OpenList => {
                    self.show_list = true;
                    self.focus_list = true;
                }
                UiMsg::ToggleMascot => {
                    let mut s = self.shared.lock().unwrap();
                    s.local.mascot_hidden = !s.local.mascot_hidden;
                    s.local.save();
                }
                UiMsg::ToggleAutostart => self.toggle_autostart(),
                UiMsg::Quit => ctx.send_viewport_cmd(ViewportCommand::Close),
                UiMsg::Restart => {
                    if let Some(exe) = &self.restart_exe
                        && crate::boot::spawn(exe, Some(std::process::id())).is_ok()
                    {
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                }
            }
        }
    }

    /// チェックの状態に合わせてタスクスケジューラへ登録・解除する。失敗したらチェックを戻す。
    fn toggle_autostart(&mut self) {
        let (Some(item), Some(exe)) = (&self.autostart_item, &self.restart_exe) else {
            return;
        };
        let want = item.is_checked();
        let result = if want {
            crate::boot::register_autostart(exe)
        } else {
            crate::boot::unregister_autostart()
        };
        let msg = match &result {
            Ok(()) if want => {
                "ログオン時に HatoPo を自動で起動するように登録しました。".to_string()
            }
            Ok(()) => "自動起動の登録を解除しました。
次回のログオンからは起動しません (今の HatoPo は動き続けます)。
もう一度起動したいときは、トレイのメニューでチェックを付け直してください。"
                .to_string(),
            Err(e) => {
                item.set_checked(!want);
                e.clone()
            }
        };
        // メッセージボックスは閉じるまで戻らないので、画面を止めないよう別スレッドで出す
        let is_err = result.is_err();
        std::thread::spawn(move || crate::win::message_box(&msg, is_err));
    }

    fn update_tray(&mut self, state: Tray, count: usize, tip: String) {
        let key = (state, count, tip);
        if key == self.tray_key {
            return;
        }
        if let Some(tray) = &self.tray {
            let base = &self.tray_imgs[&key.0];
            let img = if key.0 == Tray::Unread {
                assets::with_badge(base, key.1)
            } else {
                base.clone()
            };
            if let Ok(icon) = tray_icon::Icon::from_rgba(img.pixels, img.width, img.height) {
                let _ = tray.set_icon(Some(icon));
            }
            let _ = tray.set_tooltip(Some(&key.2));
        }
        self.tray_key = key;
    }

    /// マスコット窓の位置を覚える。ドラッグ中は OS が窓を動かすので、止まってから 1 秒後に保存する。
    fn remember_position(&mut self, ctx: &egui::Context, hidden: bool) {
        if hidden {
            return;
        }
        let Some(r) = ctx.input(|i| i.viewport().outer_rect) else {
            return;
        };
        if self.last_pos != Some(r.min) {
            if self.last_pos.is_some() {
                self.pos_dirty_since = Some(Instant::now());
            }
            self.last_pos = Some(r.min);
        }
        if let Some(t) = self.pos_dirty_since
            && t.elapsed() > Duration::from_secs(1)
        {
            let mut s = self.shared.lock().unwrap();
            s.local.mascot_pos = Some([r.min.x, r.min.y]);
            s.local.save();
            self.pos_dirty_since = None;
        }
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_messages(ctx);
        // 表示に要るものをまとめて写し取り、ロックを長く持たない
        let (snap, pending, unseen, hidden, saved_pos) = {
            let s = self.shared.lock().unwrap();
            (
                s.snap.clone(),
                s.pending(),
                s.has_unseen(),
                s.local.mascot_hidden,
                s.local.mascot_pos,
            )
        };
        if snap.settings.character != self.character && snap.loaded {
            let name = snap.settings.character.clone();
            self.load_character(ctx, &name);
        }

        // --- マスコットの表示/非表示 ---
        let hidden = hidden || !snap.settings.show_mascot;
        if self.hidden_applied != Some(hidden) {
            let pos = if hidden {
                OFFSCREEN
            } else {
                saved_pos.unwrap_or_else(|| default_position(ctx))
            };
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos.into()));
            ctx.send_viewport_cmd(ViewportCommand::MousePassthrough(hidden));
            self.hidden_applied = Some(hidden);
            self.last_pos = None;
        }
        self.remember_position(ctx, hidden);

        // --- 状態の決定 ---
        let flash = snap
            .flash
            .filter(|(_, t)| t.elapsed() < FLASH_FOR)
            .map(|(f, _)| f);
        let pose = match (snap.loaded, flash) {
            (false, _) => Pose::Idle,
            (_, Some(Flash::AllDone)) => Pose::Celebrate,
            (_, Some(Flash::Done)) => Pose::Happy,
            _ if !snap.online => Pose::Puzzled,
            _ if pending > 0 && unseen => Pose::Letter,
            _ if pending > 0 => Pose::LetterCalm,
            _ => Pose::Idle,
        };
        let bubble = match flash {
            Some(Flash::AllDone) => Some("ぜんぶ完了！おつかれさま"),
            Some(Flash::Done) => Some("完了しました！"),
            None if snap.loaded && !snap.online => Some("鳩の巣に繋がらないよ…"),
            None if pending > 0 && unseen => Some("HatoPoからの便りです！"),
            None => None,
        };
        let key = bubble.unwrap_or_default().to_string();
        if key != self.last_bubble_key {
            self.last_bubble_key = key.clone();
            self.bubble_since = (!key.is_empty()).then(|| (key, Instant::now()));
        }

        let (tray_state, tip) = if !snap.loaded {
            (Tray::Inactive, "HatoPo — 確認しています".to_string())
        } else if !snap.online {
            (Tray::Error, "HatoPo — 鳩の巣に繋がりません".to_string())
        } else if pending > 0 {
            (Tray::Unread, format!("HatoPo — 未完了の便り {pending} 件"))
        } else {
            (Tray::Normal, "HatoPo — 便りはありません".to_string())
        };
        self.update_tray(tray_state, pending, tip);

        // --- マスコット ---
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| self.mascot(ui, pose, pending));

        // --- 一覧の窓 ---
        if self.show_list {
            self.list_window(ctx, &snap);
        }

        // 吹き出しの消灯や完了の喜びを時間で戻すため、ゆっくり描き直す
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

impl App {
    fn mascot(&mut self, ui: &mut egui::Ui, pose: Pose, pending: usize) {
        let full = ui.max_rect();
        let tex = &self.poses[&pose];
        let size = tex.size_vec2();
        let scale = (130.0 / size.y).min(150.0 / size.x).min(1.0);
        let img_size = size * scale;
        let img_rect = Rect::from_min_size(
            Pos2::new(
                full.right() - img_size.x - 8.0,
                full.bottom() - img_size.y - 4.0,
            ),
            img_size,
        );
        let painter = ui.painter();
        painter.image(
            tex.id(),
            img_rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );

        if pending > 0 {
            let c = Pos2::new(img_rect.right() - 10.0, img_rect.top() + 12.0);
            painter.circle(c, 13.0, RED, Stroke::new(2.0_f32, Color32::WHITE));
            let label = if pending > 99 {
                "99+".to_string()
            } else {
                pending.to_string()
            };
            painter.text(
                c,
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(14.0),
                Color32::WHITE,
            );
        }

        let resp = ui.interact(img_rect, ui.id().with("mascot"), Sense::click_and_drag());
        if resp.drag_started_by(egui::PointerButton::Primary) {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if resp.clicked() {
            self.show_list = !self.show_list;
            self.focus_list = self.show_list;
        }
        let hovered = resp.hovered();
        resp.context_menu(|ui| {
            if ui.button("一覧を開く").clicked() {
                self.show_list = true;
                self.focus_list = true;
                ui.close();
            }
            if ui.button("今すぐ確認").clicked() {
                let _ = self.cmd_tx.send(Cmd::Refresh);
                ui.close();
            }
            if ui.button("マスコットを隠す").clicked() {
                let mut s = self.shared.lock().unwrap();
                s.local.mascot_hidden = true;
                s.local.save();
                ui.close();
            }
        });

        let text = match &self.bubble_since {
            Some((t, since)) if since.elapsed() < BUBBLE_FOR => Some(t.clone()),
            _ if hovered && pending > 0 => Some(format!("未完了の便りが {pending} 件")),
            _ if hovered => Some("クリックで一覧".to_string()),
            _ => None,
        };
        if let Some(t) = text {
            draw_bubble(ui, full, img_rect, &t);
        }
    }

    fn list_window(&mut self, ctx: &egui::Context, snap: &crate::worker::Snapshot) {
        let id = ViewportId::from_hash_of("hatopo-list");
        let monitor = ctx
            .input(|i| i.viewport().monitor_size)
            .unwrap_or(Vec2::new(1366.0, 768.0));
        let pos = list_position(monitor);
        let builder = ViewportBuilder::default()
            .with_title("HatoPo — 便りの一覧")
            .with_position(pos)
            .with_inner_size(LIST_SIZE)
            .with_min_inner_size([380.0, 320.0])
            .with_icon(self.window_icon.clone());
        if self.focus_list {
            // 開くたびに画面の右端・縦の中央へ出す (既に開いていて後ろに隠れていても戻ってくる)
            ctx.send_viewport_cmd_to(id, ViewportCommand::OuterPosition(pos.into()));
            ctx.send_viewport_cmd_to(id, ViewportCommand::Visible(true));
            ctx.send_viewport_cmd_to(id, ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd_to(id, ViewportCommand::Focus);
            self.focus_list = false;
        }
        ctx.show_viewport_immediate(id, builder, |ctx, _class| {
            if ctx.input(|i| i.viewport().close_requested()) {
                self.show_list = false;
                return;
            }
            egui::TopBottomPanel::top("status")
                .frame(
                    egui::Frame::new()
                        .fill(BLUE_SOFT)
                        .inner_margin(Margin::symmetric(14, 10)),
                )
                .show(ctx, |ui| self.status_bar(ui, snap));
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(Color32::from_rgb(0xF7, 0xF9, 0xFD))
                        .inner_margin(Margin::symmetric(14, 10)),
                )
                .show(ctx, |ui| self.list_body(ui, snap));
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, snap: &crate::worker::Snapshot) {
        ui.horizontal(|ui| {
            let (dot, text) = if !snap.loaded {
                (MUTED, "確認しています…".to_string())
            } else if snap.online {
                (
                    Color32::from_rgb(0x2E, 0x9E, 0x5B),
                    format!(
                        "鳩の巣に接続中 ・ 最終確認 {}",
                        snap.checked_at
                            .map(|t| t.format("%H:%M").to_string())
                            .unwrap_or_default()
                    ),
                )
            } else {
                (RED, "鳩の巣に繋がりません".to_string())
            };
            let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 5.0, dot);
            ui.label(RichText::new(text).color(INK));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("今すぐ確認").clicked() {
                    let _ = self.cmd_tx.send(Cmd::Refresh);
                }
            });
        });
    }

    fn list_body(&mut self, ui: &mut egui::Ui, snap: &crate::worker::Snapshot) {
        if let Some(reason) = snap.offline_reason.as_ref().filter(|_| !snap.online) {
            banner(
                ui,
                RED_BG,
                &format!("鳩の巣に繋がりません。最後に読めた一覧を表示しています。\n{reason}"),
            );
        }
        if let Some(e) = &snap.action_error {
            banner(ui, RED_BG, e);
        }
        if let Some(w) = &snap.settings_warning {
            banner(ui, AMBER_BG, &format!("{w}\n既定の設定で動いています。"));
        }
        if !snap.broken.is_empty() {
            banner(
                ui,
                AMBER_BG,
                &format!(
                    "読めない通知ファイルが {} 件あります。管理者に知らせてください。\n{}",
                    snap.broken.len(),
                    snap.broken.join("\n")
                ),
            );
        }

        if snap.items.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                let msg = if snap.loaded {
                    "届いている便りはありません"
                } else {
                    "鳩の巣を確認しています…"
                };
                ui.label(RichText::new(msg).size(16.0).color(MUTED));
                if snap.waiting > 0 {
                    ui.label(
                        RichText::new(format!(
                            "これから届く予定の便りが {} 件あります",
                            snap.waiting
                        ))
                        .color(MUTED),
                    );
                }
            });
            return;
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut last_kind = None;
                for n in &snap.items {
                    if last_kind != Some(n.target.kind) {
                        last_kind = Some(n.target.kind);
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(section_title(n.target.kind))
                                .size(13.0)
                                .strong()
                                .color(MUTED),
                        );
                        ui.add_space(2.0);
                    }
                    self.card(ui, n, snap);
                    ui.add_space(6.0);
                }
                if snap.waiting > 0 {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!(
                            "このほかに、これから届く予定の便りが {} 件あります",
                            snap.waiting
                        ))
                        .color(MUTED),
                    );
                }
            });
    }

    fn card(&mut self, ui: &mut egui::Ui, n: &Notice, snap: &crate::worker::Snapshot) {
        let (read, seen) = {
            let s = self.shared.lock().unwrap();
            (
                s.local.read_broadcasts.contains(&n.id),
                s.local.seen.contains(&n.id),
            )
        };
        let expanded = self.open.as_deref() == Some(n.id.as_str());
        let today = chrono::Local::now().date_naive();
        let fill = if read {
            Color32::from_rgb(0xF0, 0xF2, 0xF5)
        } else {
            Color32::WHITE
        };
        let stroke = if !seen && !read {
            Stroke::new(1.5_f32, BLUE)
        } else {
            Stroke::new(1.0_f32, Color32::from_rgb(0xD5, 0xDD, 0xEA))
        };
        egui::Frame::new()
            .fill(fill)
            .stroke(stroke)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    // 開閉の印は自前で描く (日本語フォントに ▸ が無く □ に化けたため)
                    let (mark, _) = ui.allocate_exact_size(Vec2::new(12.0, 18.0), Sense::hover());
                    let c = mark.center();
                    let pts = if expanded {
                        vec![
                            c + Vec2::new(-5.0, -3.0),
                            c + Vec2::new(5.0, -3.0),
                            c + Vec2::new(0.0, 4.0),
                        ]
                    } else {
                        vec![
                            c + Vec2::new(-3.0, -5.0),
                            c + Vec2::new(4.0, 0.0),
                            c + Vec2::new(-3.0, 5.0),
                        ]
                    };
                    ui.painter()
                        .add(egui::Shape::convex_polygon(pts, MUTED, Stroke::NONE));
                    let title = RichText::new(&n.title).size(15.0).strong().color(if read {
                        MUTED
                    } else {
                        INK
                    });
                    let clicked = ui
                        .add(egui::Button::new(title).frame(false).wrap())
                        .clicked();
                    if clicked {
                        if expanded {
                            self.open = None;
                        } else {
                            self.open = Some(n.id.clone());
                            let mut s = self.shared.lock().unwrap();
                            if s.local.seen.insert(n.id.clone()) {
                                s.local.save();
                            }
                        }
                        self.confirm = None;
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if !seen && !read {
                        ui.label(RichText::new("新着").small().strong().color(BLUE));
                    }
                    if let Some(d) = n.due {
                        let late = d <= today;
                        ui.label(
                            RichText::new(format!("期限 {}", d.format("%Y/%m/%d")))
                                .small()
                                .color(if late { RED } else { INK }),
                        );
                    }
                    ui.label(
                        RichText::new(format!(
                            "差出人 {} ({}) ・ {}",
                            n.created_by.user,
                            n.created_by.host,
                            n.start_at.format("%m/%d %H:%M")
                        ))
                        .small()
                        .color(MUTED),
                    );
                });
                if !expanded {
                    return;
                }
                ui.add_space(6.0);
                if !n.body.is_empty() {
                    ui.add(egui::Label::new(RichText::new(&n.body).color(INK)).selectable(true));
                    ui.add_space(4.0);
                }
                for url in &n.urls {
                    let resp = ui
                        .button(RichText::new(format!("開く: {}", link_label(url))).color(BLUE))
                        .on_hover_text(url);
                    if resp.clicked()
                        && let Err(e) = crate::win::shell_open(url)
                    {
                        self.shared.lock().unwrap().snap.action_error = Some(e);
                    }
                }
                if let Some(end) = n.end_at {
                    ui.label(
                        RichText::new(format!("{} まで表示されます", end.format("%Y/%m/%d %H:%M")))
                            .small()
                            .color(MUTED),
                    );
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if n.can_complete() {
                        let busy = snap.busy.contains(&n.id);
                        if self.confirm.as_deref() == Some(n.id.as_str()) {
                            ui.label("完了にすると一覧から消えます。");
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("完了にする").color(Color32::WHITE),
                                    )
                                    .fill(BLUE),
                                )
                                .clicked()
                            {
                                self.shared.lock().unwrap().snap.busy.insert(n.id.clone());
                                let _ = self.cmd_tx.send(Cmd::Complete(n.id.clone()));
                                self.confirm = None;
                                self.open = None;
                            }
                            if ui.button("やめる").clicked() {
                                self.confirm = None;
                            }
                        } else if busy {
                            ui.label(RichText::new("完了を書き込んでいます…").color(MUTED));
                        } else if ui.button("完了").clicked() {
                            self.confirm = Some(n.id.clone());
                        }
                    } else {
                        let label = if read {
                            "未読に戻す"
                        } else {
                            "既読にする"
                        };
                        if ui.button(label).clicked() {
                            let mut s = self.shared.lock().unwrap();
                            if read {
                                s.local.read_broadcasts.remove(&n.id);
                            } else {
                                s.local.read_broadcasts.insert(n.id.clone());
                            }
                            s.local.save();
                        }
                        ui.label(
                            RichText::new("全員へのお知らせです。管理者が片付けるまで残ります。")
                                .small()
                                .color(MUTED),
                        );
                    }
                });
            });
    }
}

fn section_title(k: Kind) -> &'static str {
    match k {
        Kind::Broadcast => "全員へのお知らせ",
        Kind::Device => "この PC への便り",
        Kind::User => "あなたへの便り",
    }
}

/// ボタンに出す短い名前。パスならファイル名、URL ならそのまま。
pub fn link_label(url: &str) -> String {
    let t = url.trim().trim_end_matches(['\\', '/']);
    if t.contains("://") {
        return t.to_string();
    }
    t.rsplit(['\\', '/'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(t)
        .to_string()
}

fn banner(ui: &mut egui::Ui, fill: Color32, text: &str) {
    egui::Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).color(INK));
        });
    ui.add_space(6.0);
}

fn draw_bubble(ui: &egui::Ui, full: Rect, bird: Rect, text: &str) {
    let width = full.width() - 16.0;
    let galley = ui.ctx().fonts(|f| {
        f.layout(
            text.to_string(),
            FontId::proportional(14.0),
            INK,
            width - 20.0,
        )
    });
    let size = galley.size() + Vec2::new(20.0, 14.0);
    let bottom = (bird.top() + 6.0).max(full.top() + size.y + 8.0);
    let rect = Rect::from_min_size(Pos2::new(full.left() + 8.0, bottom - size.y - 8.0), size);
    let p = ui.painter();
    let tail_x = (bird.center().x - 10.0).clamp(rect.left() + 14.0, rect.right() - 14.0);
    p.add(egui::Shape::convex_polygon(
        vec![
            Pos2::new(tail_x - 7.0, rect.bottom() - 1.0),
            Pos2::new(tail_x + 7.0, rect.bottom() - 1.0),
            Pos2::new(tail_x + 2.0, rect.bottom() + 9.0),
        ],
        Color32::WHITE,
        Stroke::new(1.5_f32, BLUE),
    ));
    p.rect(
        rect,
        CornerRadius::same(10),
        Color32::WHITE,
        Stroke::new(1.5_f32, BLUE),
        StrokeKind::Inside,
    );
    // 尾の付け根の線を白で消す
    p.line_segment(
        [
            Pos2::new(tail_x - 5.5, rect.bottom() - 1.0),
            Pos2::new(tail_x + 5.5, rect.bottom() - 1.0),
        ],
        Stroke::new(2.0_f32, Color32::WHITE),
    );
    p.galley(rect.min + Vec2::new(10.0, 7.0), galley, INK);
}

/// 一覧の窓の左上の位置。画面の右端に寄せ、縦は中央。画面が窓より小さいときは左上に揃える。
fn list_position(monitor: Vec2) -> [f32; 2] {
    [
        (monitor.x - LIST_SIZE[0] - LIST_MARGIN).max(0.0),
        ((monitor.y - LIST_SIZE[1]) / 2.0).max(0.0),
    ]
}

fn default_position(ctx: &egui::Context) -> [f32; 2] {
    // 画面右下 (タスクバーの少し上)。モニタの大きさが取れなければ控えめな位置
    let monitor = ctx
        .input(|i| i.viewport().monitor_size)
        .unwrap_or(Vec2::new(1366.0, 768.0));
    [
        monitor.x - MASCOT_SIZE[0] - 24.0,
        monitor.y - MASCOT_SIZE[1] - 64.0,
    ]
}

pub fn window_icon() -> egui::IconData {
    let img = assets::load_tray(None, Tray::Unread);
    egui::IconData {
        rgba: img.pixels,
        width: img.width,
        height: img.height,
    }
}

fn build_tray(
    ctx: &egui::Context,
    ui_tx: Sender<UiMsg>,
    cmd_tx: Sender<Cmd>,
    imgs: &HashMap<Tray, Rgba>,
    installed: bool,
) -> Option<(TrayIcon, CheckMenuItem)> {
    let menu = Menu::new();
    let items = [
        MenuItem::with_id("open", "一覧を開く", true, None),
        MenuItem::with_id("refresh", "今すぐ確認", true, None),
        MenuItem::with_id("mascot", "マスコットの表示/非表示", true, None),
    ];
    for i in &items {
        let _ = menu.append(i);
    }
    let _ = menu.append(&PredefinedMenuItem::separator());
    // 導入していない (--loft で起動した) ときは登録先の exe が無いので押せなくする
    let autostart = if installed {
        CheckMenuItem::with_id(
            "autostart",
            "ログオン時に自動で起動する",
            true,
            crate::boot::is_autostart_registered(),
            None,
        )
    } else {
        CheckMenuItem::with_id(
            "autostart",
            "ログオン時に自動で起動する (導入後に使えます)",
            false,
            false,
            None,
        )
    };
    let _ = menu.append(&autostart);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&MenuItem::with_id("quit", "HatoPo を終了", true, None));

    let (c1, tx1) = (ctx.clone(), ui_tx.clone());
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let msg = match e.id.0.as_str() {
            "open" => Some(UiMsg::OpenList),
            "mascot" => Some(UiMsg::ToggleMascot),
            "autostart" => Some(UiMsg::ToggleAutostart),
            "quit" => Some(UiMsg::Quit),
            "refresh" => {
                let _ = cmd_tx.send(Cmd::Refresh);
                None
            }
            _ => None,
        };
        if let Some(m) = msg {
            let _ = tx1.send(m);
        }
        c1.request_repaint();
    }));
    let c2 = ctx.clone();
    TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = e
        {
            let _ = ui_tx.send(UiMsg::OpenList);
            c2.request_repaint();
        }
    }));

    let img = &imgs[&Tray::Inactive];
    let icon = tray_icon::Icon::from_rgba(img.pixels.clone(), img.width, img.height).ok()?;
    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("HatoPo")
        .with_icon(icon)
        .build()
        .ok()
        .map(|t| (t, autostart))
}

/// 日本語を表示するため Windows に入っているフォントを読む (exe を軽く保つため埋め込まない)。
fn setup_fonts(ctx: &egui::Context) {
    let candidates = [
        r"C:\Windows\Fonts\YuGothM.ttc",
        r"C:\Windows\Fonts\meiryo.ttc",
        r"C:\Windows\Fonts\msgothic.ttc",
    ];
    let Some(bytes) = candidates.iter().find_map(|p| std::fs::read(p).ok()) else {
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "jp".into(),
        std::sync::Arc::new(egui::FontData::from_owned(bytes)),
    );
    for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(fam)
            .or_default()
            .insert(0, "jp".into());
    }
    ctx.set_fonts(fonts);
}

fn setup_style(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::light());
    ctx.style_mut(|s| {
        s.spacing.item_spacing = Vec2::new(8.0, 6.0);
        s.spacing.button_padding = Vec2::new(10.0, 5.0);
        for f in s.text_styles.values_mut() {
            f.size = f.size.max(13.0);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{LIST_SIZE, Vec2, link_label, list_position};

    #[test]
    fn list_opens_at_right_middle_of_screen() {
        // 1920x1200: 右端から 24px、縦の中央
        assert_eq!(
            list_position(Vec2::new(1920.0, 1200.0)),
            [1920.0 - LIST_SIZE[0] - 24.0, (1200.0 - LIST_SIZE[1]) / 2.0]
        );
        // 画面より窓が大きいときははみ出さず 0 に揃える
        assert_eq!(list_position(Vec2::new(500.0, 600.0)), [0.0, 0.0]);
    }

    #[test]
    fn link_labels_are_short_and_readable() {
        assert_eq!(link_label(r"\\server\共有\工数表.xlsm"), "工数表.xlsm");
        assert_eq!(link_label(r"C:\tools\更新\"), "更新");
        assert_eq!(link_label("https://example.com/a"), "https://example.com/a");
        assert_eq!(link_label("notes.txt"), "notes.txt");
    }
}
