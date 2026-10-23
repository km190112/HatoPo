//! マスコットと一覧の窓を置く場所の計算。画面には触らない (座標は egui の点)。

use eframe::egui::{Pos2, Rect, Vec2};

/// 画面の端からマスコットの窓までの余白 (右, 下)。下はタスクバーを除いた作業領域から測る
const CORNER_MARGIN: Vec2 = Vec2::new(24.0, 16.0);
/// 一覧をマスコットの窓にこれだけ重ねる。窓の左側は透明な余白なので、見た目はほぼ隣に並ぶ
const LIST_OVERLAP: f32 = 24.0;
/// これ未満しかモニターに載っていないマスコットは「画面から見つからない」とみなす
const MIN_VISIBLE: f32 = 0.8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    pub rect: Rect,
    /// タスクバーを除いた領域
    pub work: Rect,
    pub primary: bool,
}

/// メイン画面。印が無ければ先頭。
pub fn primary(screens: &[Screen]) -> Option<Screen> {
    screens
        .iter()
        .find(|s| s.primary)
        .or(screens.first())
        .copied()
}

/// 窓が一番多く載っているモニター。どれにも載っていなければメイン画面。
pub fn screen_of(win: Rect, screens: &[Screen]) -> Option<Screen> {
    screens
        .iter()
        .map(|s| (overlap_area(win, s.rect), *s))
        .filter(|(a, _)| *a > 0.0)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, s)| s)
        .or_else(|| primary(screens))
}

fn overlap_area(a: Rect, b: Rect) -> f32 {
    let i = a.intersect(b);
    if i.is_positive() { i.area() } else { 0.0 }
}

/// 窓がモニターから見える状態か。ケーブルを抜いたあとの置き去りや、上端が切れた位置を見つける。
/// モニターの情報が取れていないとき (空) は触らない。
pub fn is_reachable(win: Rect, screens: &[Screen]) -> bool {
    if screens.is_empty() || win.area() <= 0.0 {
        return true;
    }
    let best = screens
        .iter()
        .map(|s| overlap_area(win, s.rect))
        .fold(0.0_f32, f32::max);
    best / win.area() >= MIN_VISIBLE
}

/// 作業領域の右下 (マスコットの定位置)。作業領域より窓が大きいときは左上に揃える。
pub fn corner_position(work: Rect, size: Vec2) -> Pos2 {
    Pos2::new(
        (work.right() - size.x - CORNER_MARGIN.x).max(work.left()),
        (work.bottom() - size.y - CORNER_MARGIN.y).max(work.top()),
    )
}

/// 作業領域に収まる一覧の窓の大きさ。
pub fn list_size(work: Rect, want: Vec2) -> Vec2 {
    Vec2::new(want.x.min(work.width()), want.y.min(work.height()))
}

/// 一覧の窓の左上。マスコットのすぐ隣 (左が空いていれば左、無ければ右) に、
/// マスコットが画面の下半分にいれば下をそろえ、上半分にいれば上をそろえて置く。
pub fn list_position(mascot: Rect, work: Rect, size: Vec2) -> Pos2 {
    let left_x = mascot.left() + LIST_OVERLAP - size.x;
    let right_x = mascot.right() - LIST_OVERLAP;
    let x = if left_x >= work.left() {
        left_x
    } else if right_x + size.x <= work.right() {
        right_x
    } else {
        left_x
    };
    let y = if mascot.center().y >= work.center().y {
        mascot.bottom() - size.y
    } else {
        mascot.top()
    };
    Pos2::new(
        x.min(work.right() - size.x).max(work.left()),
        y.min(work.bottom() - size.y).max(work.top()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(l: f32, t: f32, r: f32, b: f32) -> Rect {
        Rect::from_min_max(Pos2::new(l, t), Pos2::new(r, b))
    }

    /// 1920x1080 のモニターで、下に 48 のタスクバー
    fn desk() -> Screen {
        Screen {
            rect: rect(0.0, 0.0, 1920.0, 1080.0),
            work: rect(0.0, 0.0, 1920.0, 1032.0),
            primary: true,
        }
    }

    /// ノート PC (1536x864 相当) の右に外部モニター
    fn laptop_and_external() -> [Screen; 2] {
        [
            Screen {
                rect: rect(0.0, 0.0, 1536.0, 864.0),
                work: rect(0.0, 0.0, 1536.0, 816.0),
                primary: true,
            },
            Screen {
                rect: rect(1536.0, -200.0, 3456.0, 880.0),
                work: rect(1536.0, -200.0, 3456.0, 832.0),
                primary: false,
            },
        ]
    }

    #[test]
    fn corner_is_bottom_right_of_work_area() {
        let size = Vec2::new(220.0, 210.0);
        let p = corner_position(desk().work, size);
        assert_eq!(p, Pos2::new(1920.0 - 220.0 - 24.0, 1032.0 - 210.0 - 16.0));
        // 作業領域より窓が大きいときははみ出さず左上に揃える
        let tiny = rect(0.0, 0.0, 200.0, 150.0);
        assert_eq!(corner_position(tiny, size), Pos2::new(0.0, 0.0));
    }

    #[test]
    fn mascot_left_behind_after_unplugging_is_found() {
        let one = [laptop_and_external()[0]];
        // 外部モニター (x=1536 以降) に置いてあった窓
        let stranded = rect(2800.0, 300.0, 3020.0, 510.0);
        assert!(!is_reachable(stranded, &one));
        // 上の大半が画面の外へ出ている (外部モニターが上にずれていた構成からの戻り)
        let cut_top = rect(1200.0, -150.0, 1420.0, 60.0);
        assert!(!is_reachable(cut_top, &one));
        // ふつうの右下
        let ok = Rect::from_min_size(
            corner_position(one[0].work, Vec2::new(220.0, 210.0)),
            Vec2::new(220.0, 210.0),
        );
        assert!(is_reachable(ok, &one));
        // モニターの情報が取れなかったら触らない
        assert!(is_reachable(stranded, &[]));
    }

    #[test]
    fn mascot_on_second_monitor_is_fine_while_it_is_connected() {
        let both = laptop_and_external();
        assert!(is_reachable(rect(2800.0, 300.0, 3020.0, 510.0), &both));
        // 2 枚にまたがって半々でも、どちらかに 8 割載っていなければ「見えない」扱い
        assert!(!is_reachable(rect(1426.0, 300.0, 1646.0, 510.0), &both));
    }

    #[test]
    fn screen_of_picks_largest_overlap_else_primary() {
        let both = laptop_and_external();
        let on_ext = rect(2800.0, 300.0, 3020.0, 510.0);
        assert!(!screen_of(on_ext, &both).unwrap().primary);
        let nowhere = rect(9000.0, 9000.0, 9100.0, 9100.0);
        assert!(screen_of(nowhere, &both).unwrap().primary);
        assert!(screen_of(nowhere, &[]).is_none());
    }

    #[test]
    fn list_opens_beside_the_mascot() {
        let size = Vec2::new(560.0, 640.0);
        let work = desk().work;
        // 右下のマスコット → 左隣、下をそろえる
        let m = Rect::from_min_size(
            corner_position(work, Vec2::new(220.0, 210.0)),
            Vec2::new(220.0, 210.0),
        );
        let p = list_position(m, work, size);
        assert_eq!(p.x, m.left() + 24.0 - 560.0);
        assert_eq!(p.y, m.bottom() - 640.0);
        // 近い: 一覧の右端はマスコットの窓の中 (左の透明な余白) に入る
        assert!(p.x + size.x > m.left() && p.x + size.x < m.right());
    }

    #[test]
    fn list_goes_to_the_right_when_left_is_full_and_stays_on_screen() {
        let size = Vec2::new(560.0, 640.0);
        let work = desk().work;
        // 左端のマスコット → 右隣
        let m = rect(0.0, 700.0, 220.0, 910.0);
        let p = list_position(m, work, size);
        assert_eq!(p.x, 220.0 - 24.0);
        // 上半分のマスコットは上をそろえる
        let m = rect(1500.0, 40.0, 1720.0, 250.0);
        assert_eq!(list_position(m, work, size).y, 40.0);
        // 小さな画面ではみ出さない
        let small = rect(0.0, 0.0, 700.0, 500.0);
        let sz = list_size(small, size);
        assert_eq!(sz, Vec2::new(560.0, 500.0));
        let p = list_position(rect(480.0, 290.0, 700.0, 500.0), small, sz);
        assert!(p.x >= 0.0 && p.y >= 0.0 && p.x + sz.x <= 700.0 && p.y + sz.y <= 500.0);
    }
}
