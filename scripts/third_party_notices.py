"""使っているライブラリの一覧 (THIRD-PARTY-NOTICES.md) を cargo metadata から作る。

    python scripts/third_party_notices.py

配布する Windows 向けの exe / dll に入る依存 (開発用・ビルド用を除く) だけを数える。
依存を足したり更新したりしたら流し直して、差分をコミットする。
"""

import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGET = "x86_64-pc-windows-msvc"


def main():
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--filter-platform", TARGET],
        cwd=ROOT, check=True, capture_output=True, text=True, encoding="utf-8",
    ).stdout
    meta = json.loads(out)
    pkgs = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}

    # ワークスペースの各 crate から、通常の依存 (kind が null) だけをたどる
    seen = set()
    stack = list(meta["workspace_members"])
    while stack:
        i = stack.pop()
        if i in seen:
            continue
        seen.add(i)
        for d in nodes[i]["deps"]:
            if any(k["kind"] is None for k in d["dep_kinds"]):
                stack.append(d["pkg"])

    members = set(meta["workspace_members"])
    rows = sorted(
        (pkgs[i] for i in seen if i not in members),
        key=lambda p: (p["name"].lower(), p["version"]),
    )

    lines = [
        "# THIRD-PARTY-NOTICES",
        "",
        "HatoPo が使っているオープンソースのライブラリです (配布する exe / dll に入るもの)。",
        "それぞれの著作権とライセンスは各プロジェクトに帰属します。",
        "HatoPo 自身のライセンスは [LICENSE](LICENSE) を見てください。",
        "",
        "`python scripts/third_party_notices.py` で作り直せます。",
        "",
        f"{len(rows)} 件",
        "",
        "| ライブラリ | 版 | ライセンス | 場所 |",
        "|---|---|---|---|",
    ]
    for p in rows:
        lic = (p["license"] or "(Cargo.toml に記載なし)").replace("|", "/")
        repo = p["repository"] or p["homepage"] or f"https://crates.io/crates/{p['name']}"
        lines.append(f"| {p['name']} | {p['version']} | {lic} | {repo} |")
    lines.append("")
    (ROOT / "THIRD-PARTY-NOTICES.md").write_text("\n".join(lines), encoding="utf-8", newline="\n")
    print(f"{len(rows)} 件 -> THIRD-PARTY-NOTICES.md")


if __name__ == "__main__":
    main()
