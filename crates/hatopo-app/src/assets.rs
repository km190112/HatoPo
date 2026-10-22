//! キャラクター画像。鳩の巣の `character_icon\<名前>\` から読み、無いものは exe に埋め込んだ HatoPoBlue を使う。
//! (共有に繋がらない状態で起動しても絵が出るように)

use std::path::Path;

/// マスコットの絵の種類。ファイル名は `poses\<name>.png`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pose {
    /// 通知なし
    Idle,
    /// まだ開いていない通知がある
    Letter,
    /// 未完了はあるが全部目は通した
    LetterCalm,
    /// 1 件完了した直後
    Happy,
    /// 全部完了した直後
    Celebrate,
    /// 鳩の巣に繋がらない
    Puzzled,
    /// マスコットを休ませている (今は未使用。設定で選べるようにする余地)
    Sleep,
}

impl Pose {
    pub const ALL: [Pose; 7] = [
        Pose::Idle,
        Pose::Letter,
        Pose::LetterCalm,
        Pose::Happy,
        Pose::Celebrate,
        Pose::Puzzled,
        Pose::Sleep,
    ];

    pub fn file_stem(self) -> &'static str {
        match self {
            Pose::Idle => "idle",
            Pose::Letter => "letter",
            Pose::LetterCalm => "letter_calm",
            Pose::Happy => "happy",
            Pose::Celebrate => "celebrate",
            Pose::Puzzled => "puzzled",
            Pose::Sleep => "sleep",
        }
    }

    fn embedded(self) -> &'static [u8] {
        macro_rules! pose {
            ($n:literal) => {
                include_bytes!(concat!(
                    "../../../loft-template/character_icon/HatoPoBlue/poses/",
                    $n,
                    ".png"
                ))
            };
        }
        match self {
            Pose::Idle => pose!("idle"),
            Pose::Letter => pose!("letter"),
            Pose::LetterCalm => pose!("letter_calm"),
            Pose::Happy => pose!("happy"),
            Pose::Celebrate => pose!("celebrate"),
            Pose::Puzzled => pose!("puzzled"),
            Pose::Sleep => pose!("sleep"),
        }
    }
}

/// トレイアイコンの種類。ファイル名は `tray-icon\<name>.png`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tray {
    Normal,
    Unread,
    Inactive,
    Error,
}

impl Tray {
    pub const ALL: [Tray; 4] = [Tray::Normal, Tray::Unread, Tray::Inactive, Tray::Error];

    fn file_stem(self) -> &'static str {
        match self {
            Tray::Normal => "normal",
            Tray::Unread => "unread",
            Tray::Inactive => "inactive",
            Tray::Error => "error",
        }
    }

    fn embedded(self) -> &'static [u8] {
        macro_rules! tray {
            ($n:literal) => {
                include_bytes!(concat!(
                    "../../../loft-template/character_icon/HatoPoBlue/tray-icon/",
                    $n,
                    ".png"
                ))
            };
        }
        match self {
            Tray::Normal => tray!("normal"),
            Tray::Unread => tray!("unread"),
            Tray::Inactive => tray!("inactive"),
            Tray::Error => tray!("error"),
        }
    }
}

pub const HERO_PNG: &[u8] = include_bytes!("../../../assets/hero.png");

/// 復号済みの RGBA 画像。
#[derive(Clone)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub fn decode(bytes: &[u8]) -> Option<Rgba> {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .ok()?
        .into_rgba8();
    Some(Rgba {
        width: img.width(),
        height: img.height(),
        pixels: img.into_raw(),
    })
}

/// 鳩の巣の絵を優先し、無い・壊れているものは埋め込みを使う。
fn load(dir: Option<&Path>, sub: &str, stem: &str, fallback: &[u8]) -> Rgba {
    dir.and_then(|d| std::fs::read(d.join(sub).join(format!("{stem}.png"))).ok())
        .and_then(|b| decode(&b))
        .unwrap_or_else(|| decode(fallback).expect("埋め込み画像は必ず PNG として読める"))
}

pub fn load_pose(character_dir: Option<&Path>, p: Pose) -> Rgba {
    load(character_dir, "poses", p.file_stem(), p.embedded())
}

pub fn load_tray(character_dir: Option<&Path>, t: Tray) -> Rgba {
    load(character_dir, "tray-icon", t.file_stem(), t.embedded())
}

/// トレイアイコンの右上に赤い丸と件数を描く。数字は 3x5 の点字フォントで描くので外部フォントが要らない。
pub fn with_badge(base: &Rgba, count: usize) -> Rgba {
    let mut img = base.clone();
    if count == 0 {
        return img;
    }
    let w = img.width as i32;
    let h = img.height as i32;
    let r = (w.min(h) as f32 * 0.30) as i32;
    let (cx, cy) = (w - r - 1, r + 1);
    let mut put = |x: i32, y: i32, c: [u8; 4]| {
        if x >= 0 && y >= 0 && x < w && y < h {
            let i = ((y * w + x) * 4) as usize;
            img.pixels[i..i + 4].copy_from_slice(&c);
        }
    };
    for y in cy - r..=cy + r {
        for x in cx - r..=cx + r {
            let d2 = (x - cx).pow(2) + (y - cy).pow(2);
            if d2 <= r * r {
                put(x, y, [0xE5, 0x39, 0x35, 0xFF]);
            }
        }
    }
    let text = if count > 9 {
        "9+".to_string()
    } else {
        count.to_string()
    };
    let glyphs: Vec<[u8; 5]> = text.chars().map(glyph).collect();
    let scale = (r / 4).max(1);
    let gw = 3 * scale;
    let gap = scale;
    let total = glyphs.len() as i32 * gw + (glyphs.len() as i32 - 1) * gap;
    let mut x0 = cx - total / 2;
    let y0 = cy - (5 * scale) / 2;
    for g in glyphs {
        for (row, bits) in g.iter().enumerate() {
            for col in 0..3 {
                if bits & (0b100 >> col) != 0 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            put(
                                x0 + col * scale + dx,
                                y0 + row as i32 * scale + dy,
                                [0xFF, 0xFF, 0xFF, 0xFF],
                            );
                        }
                    }
                }
            }
        }
        x0 += gw + gap;
    }
    img
}

fn glyph(c: char) -> [u8; 5] {
    match c {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b010, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        '+' => [0b000, 0b010, 0b111, 0b010, 0b000],
        _ => [0; 5],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_image_decodes() {
        for p in Pose::ALL {
            let img = load_pose(None, p);
            assert!(img.width > 50 && img.height > 50, "{p:?}");
        }
        for t in Tray::ALL {
            assert_eq!(load_tray(None, t).width, 64, "{t:?}");
        }
        assert!(decode(HERO_PNG).is_some());
    }

    #[test]
    fn loft_images_override_and_broken_ones_fall_back() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("poses")).unwrap();
        // 2x2 の PNG を置いた絵は差し替わる
        let mut png = Vec::new();
        image::RgbaImage::new(2, 2)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        std::fs::write(d.path().join("poses/idle.png"), &png).unwrap();
        std::fs::write(d.path().join("poses/happy.png"), b"not a png").unwrap();
        assert_eq!(load_pose(Some(d.path()), Pose::Idle).width, 2);
        assert!(load_pose(Some(d.path()), Pose::Happy).width > 50);
        assert!(load_pose(Some(d.path()), Pose::Sleep).width > 50);
    }

    #[test]
    fn badge_draws_red_in_top_right_and_keeps_size() {
        let base = load_tray(None, Tray::Unread);
        for n in [1, 3, 9, 10, 120] {
            let b = with_badge(&base, n);
            assert_eq!((b.width, b.height), (64, 64));
            // 赤丸の左端付近 (数字の外) は赤
            let r = (64.0 * 0.30) as u32;
            let (x, y) = (64 - 2 * r + 1, r + 1);
            let i = ((y * 64 + x) * 4) as usize;
            assert_eq!(&b.pixels[i..i + 3], &[0xE5, 0x39, 0x35], "{n}");
        }
        assert!(with_badge(&base, 0).pixels == base.pixels);
    }
}
