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
- 自動起動の**登録**は exe 自身が schtasks を呼ぶと Microsoft Defender に誤検知されるため exe から外し、
  `loft-template/autostart/自動起動を登録.bat` (`Register-ScheduledTask`。`schtasks /sc onlogon` は
  管理者権限が要るので使わない) で行う (2026-10-30 Ken)。**解除** (`schtasks /delete`) は persistence を
  作らないので `HatoPo.exe --uninstall` に残す (このとき `%LOCALAPPDATA%\HatoPo\` も削除する)。
  トレイの「ログオン時に自動で起動する」は登録状態の表示のみで、クリックしては切り替えない
- 書き込み口は DLL (64bit Office のみ) と CLI。**リンクの種類は制限しない** (更新用 exe も配るため)
- 自己更新は署名なし。代わりに共有の exe 3 本を管理者のみ書込みにする運用を README と同梱の説明書に書く (Ken 決定)
- push・main への統合・リリース作成は Claude に任せてよい。ファイル削除・公開範囲の変更は確認する

## 構成

`crates/hatopo-core` (ロジックはここだけ) / `hatopo-app` (HatoPo.exe) / `hatopo-dll` (hatopo.dll) / `hatopo-cli` /
`hatopo-admin` (HatoPoAdmin.exe。通知・完了の記録・名簿・設定)。
鳩の巣の `directory.json` (名簿) と `settings.json` は管理画面だけが書く (core の `Directory::save` / `Settings::save` が検査と同時編集の検出をする)。
`read\<通知ID>\<ユーザー>.json` は「既読にする」を押した印 (core の `read.rs`。開いただけでは書かない Ken 決定 2026-10-27)。`close` で archive の `read_by` へ写して片付ける。利用者側は `state.json` の `read_dirty` に溜め、裏方が次の確認で書く。
利用者の HatoPo は差出人の名前を出すために `directory.json` を読む (読むだけ)。
`status\user|device\<名前>.json` は各 PC の HatoPo が起動時・日付が変わって最初の確認・完了時に書く生存印 (1 日 1 回。Ken 指摘 2026-10-23 で 15 分ごとから変更) (core の `touch`)。
Ken は Releases の zip を Desktop に展開して試験導入している (loft.txt が指す)。そこには触らない。画面の確認は scratchpad に試験用の鳩の巣を作り、`LOCALAPPDATA` を差し替えて `--loft` で起動する (常駐中の HatoPo は止めて、終わったら起こし直す)。
マスコットは画面構成 (モニター) が変わると、落ち着くのを待ってメイン画面の作業領域の右下へ戻す (`place.rs` が座標の計算、`win::monitors` が構成の取得)。一覧の窓はマスコットの隣に開く。
利用者が選んだキャラクターは `state.json` の `character` (None なら settings.json の既定)。著作権・ライセンスの画面は一覧の［HatoPo について］。
`THIRD-PARTY-NOTICES.md` は `python scripts/third_party_notices.py` の生成物 (依存を変えたら流し直す)。AI で絵を増やす手順は `docs/キャラクター画像の作り方.md`。
`loft-template/` は配布する鳩の巣の雛形 (`autostart/` に自動起動登録の bat/ps1 を含む)、
`vba/HatoPo.src.txt` が VBA の正 (`.bas` は生成物)。

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
