# HatoPo — Claude 向けプロジェクト規約

共有フォルダだけで動く Windows 常駐通知アプリ。**趣味の公開リポジトリ** (本業・TMK とは別)。
業務と機能の地図: https://claude.ai/artifact/U5a6oXEJtFbDvKxLsXuiFJ (PR を main に統合したら更新する)

## 公開リポジトリなので

- 社名・実在のサーバー名・共有パス・ユーザー名・PC 名を書かない。例は `\\server\HatoLoft` / `taro` / `PC-0123`
- キャラクター画像は MIT の対象外 (LICENSE 参照)

## 決めたこと (2026-10-22 Ken)

- 保存は **JSON 1 件 1 ファイル**。SQLite にしない (SMB 上のロックが信用できない)
- 書き込みは一時ファイル → rename。変更・退避は `.{id}.lock` を排他作成し、`revision` で同時更新を検出
- exe は共有から直接常駐させない。`%LOCALAPPDATA%\HatoPo` へコピーして起動、共有の exe が変われば自己更新
- 自動起動はタスクスケジューラに XML で登録 (`schtasks /sc onlogon` は管理者権限が要るので使わない)
- 書き込み口は DLL (64bit Office のみ) と CLI。**リンクの種類は制限しない** (更新用 exe も配るため)
- 自己更新は署名なし。代わりに共有の exe 3 本を管理者のみ書込みにする運用を README と同梱の説明書に書く (Ken 決定)
- push・main への統合・リリース作成は Claude に任せてよい。ファイル削除・公開範囲の変更は確認する

## 構成

`crates/hatopo-core` (ロジックはここだけ) / `hatopo-app` (HatoPo.exe) / `hatopo-dll` (hatopo.dll) / `hatopo-cli`。
`loft-template/` は配布する鳩の巣の雛形、`vba/HatoPo.src.txt` が VBA の正 (`.bas` は生成物)。

## 検証

```
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
cargo build --workspace --release && pwsh -File tests\vba_boundary_test.ps1
pwsh -File scripts\check_bas.ps1
```

- DLL の境界は UTF-16 BSTR (StrPtr) + `ByRef Variant`。別の表現に変えない
- テストの削除・緩和・`#[ignore]` は禁止
- 画面を変えたら `--loft <試験用フォルダ> --show-list` で起動し、スクリーンショットで見切れ・文字化けを確かめる
  (日本語フォントに無い記号は □ になる。開閉の印は自前で描いている)
