//! HatoPo.exe — いつでも、あなたのデスクトップに。小さな伝書鳩。
//!
//! 起動のしかたは 3 通り:
//! - 共有 (鳩の巣) の `HatoPo.exe` をダブルクリック → ローカルへ導入して自動起動を登録し、そちらを起動
//! - ローカルの `%LOCALAPPDATA%\HatoPo\HatoPo.exe` (ログオン時の自動起動) → 常駐
//! - `HatoPo.exe --loft <鳩の巣>` → 導入も自己更新もせずにその場で常駐 (試験・開発用)
//!
//! `--uninstall` で自動起動とトーストの登録を外す。`--show-list` は起動と同時に一覧を開く。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod boot;
mod local;
mod place;
mod ui;
mod win;
mod worker;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};

use hatopo_core::{Origin, Settings};

struct Args {
    loft: Option<PathBuf>,
    wait_pid: Option<u32>,
    uninstall: bool,
    show_list: bool,
}

fn parse_args() -> Args {
    let mut a = Args {
        loft: None,
        wait_pid: None,
        uninstall: false,
        show_list: false,
    };
    let mut it = std::env::args_os().skip(1);
    while let Some(x) = it.next() {
        match x.to_string_lossy().as_ref() {
            "--loft" => a.loft = it.next().map(PathBuf::from),
            "--wait-pid" => a.wait_pid = it.next().and_then(|p| p.to_string_lossy().parse().ok()),
            "--uninstall" => a.uninstall = true,
            "--show-list" => a.show_list = true,
            _ => {}
        }
    }
    a
}

fn main() {
    let args = parse_args();
    if let Some(pid) = args.wait_pid {
        win::wait_for_pid(pid, 15_000);
    }
    if args.uninstall {
        let r = boot::unregister_autostart();
        win::unregister_aumid(worker::AUMID);
        match r {
            Ok(()) => win::message_box("HatoPo の自動起動を解除しました。", false),
            Err(e) => win::message_box(&e, true),
        }
        return;
    }

    let exe = std::env::current_exe().unwrap_or_default();
    let local_exe = boot::local_dir().join(boot::EXE_NAME);
    let running_local = boot::same_path(&exe, &local_exe);

    let (loft, installed_exe) = if let Some(l) = args.loft {
        (l, None)
    } else if running_local {
        match boot::saved_loft() {
            Some(l) => (l, Some(local_exe.clone())),
            None => {
                win::message_box(
                    "鳩の巣 (共有フォルダ) の場所が分かりません。\n共有フォルダにある HatoPo.exe をダブルクリックして導入し直してください。",
                    true,
                );
                return;
            }
        }
    } else if let Some(dir) = exe.parent().filter(|d| boot::looks_like_loft(d)) {
        // 共有から起動された: ローカルへ導入して、そちらを起動する
        if win::SingleInstance::acquire(INSTANCE).is_none() {
            win::message_box(
                "HatoPo は既に動いています。タスクトレイの鳩をクリックしてください。",
                false,
            );
            return;
        }
        match boot::install(dir, &exe) {
            Ok(dest) => {
                win::message_box(
                    "HatoPo を導入しました。\n次回からはログオンすると自動で起動します。",
                    false,
                );
                if let Err(e) = boot::spawn(&dest, Some(std::process::id())) {
                    win::message_box(&e, true);
                }
            }
            Err(e) => win::message_box(&format!("HatoPo を導入できませんでした。\n{e}"), true),
        }
        return;
    } else {
        win::message_box(
            "鳩の巣 (共有フォルダ) が見つかりません。\n共有フォルダにある HatoPo.exe から起動してください。",
            true,
        );
        return;
    };

    let Some(_instance) = win::SingleInstance::acquire(INSTANCE) else {
        if args.wait_pid.is_none() {
            win::message_box(
                "HatoPo は既に動いています。タスクトレイの鳩をクリックしてください。",
                false,
            );
        }
        return;
    };
    boot::cleanup_old();

    // トーストの差出人を「HatoPo」にする (失敗してもマスコットとトレイで知らせられる)
    let icon_png = boot::local_dir().join("hatopo.png");
    let _ = std::fs::create_dir_all(boot::local_dir());
    let _ = std::fs::write(&icon_png, assets::HERO_PNG);
    let _ = win::register_aumid(worker::AUMID, "HatoPo", &icon_png);

    run(loft, installed_exe, icon_png, args.show_list);
}

const INSTANCE: &str = "HatoPo.SingleInstance";

fn run(loft: PathBuf, installed_exe: Option<PathBuf>, icon_png: PathBuf, show_list: bool) {
    let local = local::LocalState::load();
    let start_pos = local.mascot_pos;
    let (settings, _) = Settings::load(&loft);
    // 選んである大きさで窓を作る (最初の描画で大きさが跳ねないように)
    let start_size = ui::mascot_size(ui::clamp_scale(
        local.mascot_scale.unwrap_or(settings.mascot_scale),
    ));
    let shared = Arc::new(Mutex::new(worker::Shared {
        snap: worker::Snapshot {
            settings,
            ..Default::default()
        },
        local,
    }));
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (ui_tx, ui_rx) = mpsc::channel();
    if show_list {
        let _ = ui_tx.send(worker::UiMsg::OpenList);
    }

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("HatoPo")
        .with_inner_size(start_size)
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_taskbar(false)
        .with_resizable(false)
        .with_icon(ui::window_icon());
    if let Some(p) = start_pos {
        viewport = viewport.with_position(p);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    let result = eframe::run_native(
        "HatoPo",
        options,
        Box::new(move |cc| {
            let worker = worker::Worker {
                loft: hatopo_core::Loft::new(&loft),
                me: Origin::current(),
                shared: shared.clone(),
                ctx: cc.egui_ctx.clone(),
                ui_tx: ui_tx.clone(),
                installed_exe: installed_exe.clone(),
                toast_icon: Some(icon_png),
            };
            std::thread::Builder::new()
                .name("hatopo-worker".into())
                .spawn(move || worker.run(cmd_rx))
                .expect("裏方スレッドを起動できない");
            Ok(Box::new(ui::App::new(
                cc,
                shared,
                cmd_tx,
                ui_tx,
                ui_rx,
                loft,
                installed_exe,
            )))
        }),
    );
    if let Err(e) = result {
        win::message_box(&format!("HatoPo の画面を開けませんでした。\n{e}"), true);
    }
}
