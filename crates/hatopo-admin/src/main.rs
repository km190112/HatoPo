//! HatoPoAdmin.exe — HatoPo の管理画面。
//!
//! 鳩の巣の場所は `--loft <フォルダ>`、無ければ exe のあるフォルダ (鳩の巣に置いた場合)、
//! それも違えば前回開いた場所。どれでもなければ画面の上で選ぶ。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod data;
mod form;

use std::path::PathBuf;

fn looks_like_loft(dir: &std::path::Path) -> bool {
    ["settings.json", "broadcast", "PigeonHole"]
        .iter()
        .any(|n| dir.join(n).exists())
}

fn main() -> eframe::Result {
    let mut args = std::env::args_os().skip(1);
    let mut loft: Option<PathBuf> = None;
    while let Some(a) = args.next() {
        if a == "--loft" {
            loft = args.next().map(PathBuf::from);
        }
    }
    let loft = loft
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(PathBuf::from))
                .filter(|d| looks_like_loft(d))
        })
        .or_else(|| {
            std::fs::read_to_string(app::admin_loft_file())
                .ok()
                .map(|s| PathBuf::from(s.trim()))
                .filter(|p| !p.as_os_str().is_empty())
        })
        .unwrap_or_default();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("HatoPo 管理")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([900.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "HatoPo 管理",
        options,
        Box::new(move |cc| Ok(Box::new(app::AdminApp::new(cc, loft)))),
    )
}

/// 日本語を表示するため Windows に入っているフォントを読む (exe を軽く保つため埋め込まない)。
pub fn setup_fonts(ctx: &eframe::egui::Context) {
    use eframe::egui;
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
