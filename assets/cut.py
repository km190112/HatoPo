"""キャラクターシート (assets/src/*.webp) からポーズ・トレイアイコン・exe アイコンを切り出す。

    python assets/cut.py

シートは最初から背景が透過されている。切り出し枠の内側にある「塊」(透明でない画素のつながり) のうち、
枠に完全に収まるものだけを残す。隣の絵や文字ラベルの端が枠にかかっても混ざらない。
出力先: loft-template/character_icon/HatoPoBlue/ と assets/
"""

from collections import deque
from pathlib import Path

import numpy as np
from PIL import Image, ImageFilter

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "assets" / "src"
CHAR = ROOT / "loft-template" / "character_icon" / "HatoPoBlue"

# (シート, 左, 上, 右, 下)。座標はシート上の画素
POSES = {
    "idle": ("sheet2", 40, 364, 161, 514),  # 待機
    "letter": ("sheet1", 800, 626, 1020, 830),  # 手紙を持って驚く (新着)
    "letter_calm": ("sheet2", 33, 729, 145, 819),  # 手紙をくわえる (未完了あり)
    "happy": ("sheet1", 258, 640, 420, 831),  # 喜び (完了した直後)
    "celebrate": ("sheet1", 10, 610, 236, 820),  # 手紙を掲げて喜ぶ (全部完了)
    "puzzled": ("sheet1", 444, 626, 594, 831),  # はてな (共有に繋がらない)
    "sleep": ("sheet1", 614, 648, 796, 830),  # 睡眠
}
TRAY = {
    "normal": ("sheet1", 541, 878, 656, 957),
    "unread": ("sheet1", 701, 869, 820, 956),
    "inactive": ("sheet2", 305, 882, 372, 933),
    "error": ("sheet2", 393, 881, 461, 934),
}
HERO = ("sheet2", 79, 6, 497, 318)  # 鞄を掛けて手紙を運ぶ鳩 (exe アイコン・README)
LOGO = ("sheet1", 46, 844, 448, 1016)

_sheets = {}


def sheet(name):
    if name not in _sheets:
        _sheets[name] = Image.open(SRC / f"{name}.webp").convert("RGBA")
    return _sheets[name]


def cut(spec, pad=2):
    name, x0, y0, x1, y1 = spec
    im = sheet(name).crop((x0, y0, x1 + 1, y1 + 1))
    a = np.array(im)
    solid = a[..., 3] > 20
    # 近い画素同士 (アンチエイリアスの縁など) を 1 つの塊として扱うため少し太らせて数える
    grown = np.array(Image.fromarray((solid * 255).astype("uint8")).filter(ImageFilter.MaxFilter(5))) > 0
    h, w = grown.shape
    seen = np.zeros_like(grown)
    keep = np.zeros_like(grown)
    for y in range(h):
        for x in range(w):
            if grown[y, x] and not seen[y, x]:
                q = deque([(y, x)])
                seen[y, x] = True
                pts = []
                while q:
                    cy, cx = q.popleft()
                    pts.append((cy, cx))
                    for dy, dx in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                        ny, nx = cy + dy, cx + dx
                        if 0 <= ny < h and 0 <= nx < w and grown[ny, nx] and not seen[ny, nx]:
                            seen[ny, nx] = True
                            q.append((ny, nx))
                ys = [p[0] for p in pts]
                xs = [p[1] for p in pts]
                touches_edge = min(ys) == 0 or min(xs) == 0 or max(ys) == h - 1 or max(xs) == w - 1
                if not touches_edge or len(pts) > 0.3 * solid.sum():
                    for p in pts:
                        keep[p] = True
    a[..., 3] = np.where(keep, a[..., 3], 0)
    out = Image.fromarray(a)
    bbox = out.getbbox()
    out = out.crop(bbox)
    canvas = Image.new("RGBA", (out.width + pad * 2, out.height + pad * 2), (0, 0, 0, 0))
    canvas.alpha_composite(out, (pad, pad))
    return canvas


def square(im, size):
    side = max(im.size)
    sq = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    sq.alpha_composite(im, ((side - im.width) // 2, (side - im.height) // 2))
    return sq.resize((size, size), Image.Resampling.LANCZOS)


def main():
    (CHAR / "poses").mkdir(parents=True, exist_ok=True)
    (CHAR / "tray-icon").mkdir(parents=True, exist_ok=True)
    for name, spec in POSES.items():
        im = cut(spec)
        # マスコット窓は 128px 前後で描く。大きい絵は 160px 四方に収める
        im.thumbnail((160, 160), Image.Resampling.LANCZOS)
        im.save(CHAR / "poses" / f"{name}.png")
        print("pose", name, im.size)
    for name, spec in TRAY.items():
        square(cut(spec), 64).save(CHAR / "tray-icon" / f"{name}.png")
        print("tray", name)
    hero = cut(HERO)
    square(hero, 256).save(
        ROOT / "assets" / "hatopo.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
    )
    hero.thumbnail((360, 360), Image.Resampling.LANCZOS)
    hero.save(ROOT / "assets" / "hero.png")
    logo = cut(LOGO)
    logo.thumbnail((480, 480), Image.Resampling.LANCZOS)
    logo.save(ROOT / "assets" / "logo.png")
    print("ico, hero, logo")


if __name__ == "__main__":
    main()
