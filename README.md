<p align="center"><img src="assets/logo.png" alt="HatoPo — いつでも、あなたのデスクトップに。小さな伝書鳩・HatoPo" width="420"></p>

# HatoPo

**共有フォルダだけで動く、Windows のデスクトップ常駐通知アプリ。**
サーバーもデータベースもメールも要りません。

- Excel VBA のツールから「1 週間後に、この人に、このファイルを開いて作業してもらう」通知を予約できる
- 特定の PC に「プログラムを更新してください」と送れる
- 通知は利用者が **1 件ずつ［完了］を押すまで** 出し続ける
- 全員へのお知らせ (一斉送信) は、管理者が片付けるまで残る

<p align="center"><img src="assets/hero.png" alt="手紙を運ぶ青い鳩" width="220"></p>

デスクトップの隅に鳩のマスコットが居て、便りが届くと手紙をくわえて知らせます。
クリックすると一覧が開き、そこから Excel などのファイルを開けます。
タスクトレイにも鳩が居て、未完了の件数をバッジで出します。

> **Windows 専用です。** VBA 用の DLL は 64bit 版 Excel 専用です。

## しくみ

```
 管理者・ツール作者                         利用者の PC (何台でも)
   Excel VBA ── hatopo.dll ─┐                HatoPo.exe (常駐)
   bat / PS ── hatopo-cli ──┤                 ├ マスコット / タスクトレイ / トースト
                            ▼                 └ 一覧 → ファイルを開く → 完了
          \\server\HatoLoft\  ◀── 60 秒ごとに自分宛てだけを見に行く ──┘
            HatoPo.exe, hatopo.dll, hatopo-cli.exe, settings.json
            character_icon\HatoPoBlue\
            broadcast\                    全員へのお知らせ
            PigeonHole\device\<PC名>\     その PC への便り
            PigeonHole\user\<ユーザー名>\  その人への便り
            archive\2026-10\              完了・期限切れ・削除した便り
```

- **通知 1 件 = JSON ファイル 1 つ。** データベースを使わないので、遅い共有フォルダでも壊れにくい
  (SQLite はネットワーク共有上でのロックが信用できないため採用していません)
- 書き込みは一時ファイルに書いてから名前を変える方式なので、書きかけのファイルを読むことがありません
- 同じ通知を 2 人が同時に変えようとすると、後の人は「他の人が先に変更しました」と止まります (版番号で検出)
- 一覧の順番は **全員へのお知らせ → この PC への便り → あなたへの便り**
- 通知開始日より前は出ず、通知終了日を過ぎたら出さずに `archive\` へ移ります

## 導入

### 管理者 (最初の 1 回)

1. [Releases](https://github.com/km190112/HatoPo/releases/latest) から `HatoPo-vX.Y.Z.zip` を落として展開する
2. 中の `HatoLoft` フォルダを共有フォルダに置く (例: `\\server\HatoLoft`)。**利用者全員が読み書きできる**ようにする
3. 必要なら `settings.json` を書き換える (下記)

### 利用者 (1 人 1 回)

1. 共有フォルダの `HatoPo.exe` をダブルクリックする
2. 「HatoPo を導入しました」と出たら OK を押す

これで自分の PC (`%LOCALAPPDATA%\HatoPo\`) にコピーされ、次回からはログオン時に自動で起動します
(タスクスケジューラに自分専用で登録します。管理者権限は要りません)。

- **更新は自動です。** 管理者が共有の `HatoPo.exe` を新しくすると、各 PC は次の確認で入れ替えて起動し直します
- 共有に繋がらないときも、最後に読めた一覧を出し続けます (鳩が「？」の顔になります)
- やめるとき: `%LOCALAPPDATA%\HatoPo\HatoPo.exe --uninstall` で自動起動を解除し、フォルダを消す

## 通知の出し方

### Excel VBA から

1. 配布 zip の `vba\HatoPo.bas` を VBE の「ファイル > ファイルのインポート」で取り込む
2. モジュール先頭の `HATOPO_ROOT` を自分の鳩の巣に書き換える

```vb
' 1 週間後の朝 9 時から、自分に「確認してください」と出す。終了日の翌日には自動で片付く
id = HP_Add(HP_Me(), "入力内容を確認してください", _
            本文:="先週入力したデータの確認をお願いします。", _
            URL1:=ThisWorkbook.FullName, _
            開始:=Date + 7 + TimeSerial(9, 0, 0), 終了:=Date + 14)

' 他の人へ (期限付き)
HP_Add HP_User("taro"), "工数表を入力してください", 期限:=#11/5/2026#, URL1:="\\server\共有\工数表.xlsm"

' 特定の PC へ
HP_Add HP_Device("PC-0123"), "工数集計ツールを v2.3 に更新してください", URL1:="\\server\tools\setup.exe"

' 全員へ (利用者は完了できない。HP_Delete で消す)
HP_Add HP_Broadcast(), "12/27 は全館停電です", 終了:=#12/27/2026#

' 探す (0 始まりの 2 次元配列。0 行目が見出し) / シートに貼る
a = HP_Find(種別:="user", 宛先名:="taro")
HP_FindToSheet Sheet1.Range("A1"), キーワード:="工数"

' 書き換える (版は HP_Find で読んだ HP_COL_REVISION の値) / 消す
HP_Update id, 1, "件名を直しました", 期限:=#11/10/2026#
HP_Delete id
```

失敗すると `Err.Raise` でエラーになり、`Err.Description` に日本語で理由が入ります。

| 関数 | 内容 |
|---|---|
| `HP_Add(宛先, 件名, [本文], [期限], [URL1], [URL2], [URL3], [開始], [終了])` | 通知を作る。戻り値は ID |
| `HP_Update(id, 版, 件名, ...)` | 書き換える (宛先は変えられない)。戻り値は新しい版 |
| `HP_Delete(id)` | 消す (`archive\` へ移す) |
| `HP_Find([種別], [宛先名], [キーワード])` | 探す。見出し付き 2 次元配列 |
| `HP_Get(id)` | 1 件取る |
| `HP_FindToSheet(貼り付け先, ...)` | 探した結果をシートに貼る |
| `HP_User(名前)` / `HP_Device(PC名)` / `HP_Broadcast()` / `HP_Me()` | 宛先を作る |

日付は Date 型でも `"2026/11/05 9:00"` のような文字列でも渡せます。
開始を省略すると今すぐ、終了を省略すると完了されるまで出し続けます。終了に日付だけを渡すとその日いっぱい出します。

### コマンド (hatopo-cli) から

鳩の巣の場所は `--root`、無ければ環境変数 `HATOPO_ROOT`、それも無ければ `hatopo-cli.exe` のあるフォルダです。

```bat
hatopo-cli add --to user:taro --to user:jiro --title "工数表を入力してください" --due 2026-11-05 --url "\\server\共有\工数表.xlsm"
hatopo-cli add --to device:PC-0123 --title "ツールを更新してください" --end 2026-11-30
hatopo-cli list --kind device
hatopo-cli update <ID> --revision 1 --title "件名を直しました"
hatopo-cli delete <ID>
REM 誰がいつ完了したか
hatopo-cli archive 2026-10
```

終了コード: 0 成功 / 2 入力の誤り / 3 見つからない / 4 他の人が先に変更 / 5 変更中 / 6 鳩の巣に繋がらない。

## 設定 (`settings.json`)

```json
{
  "character": "HatoPoBlue",
  "poll_seconds": 60,
  "remind_minutes": 60,
  "show_mascot": true
}
```

| 項目 | 内容 |
|---|---|
| `character` | `character_icon\` の下のフォルダ名。フォルダを足せばキャラクターを切り替えられる |
| `poll_seconds` | 共有を見に行く間隔 (秒)。10 秒未満は 10 秒 |
| `remind_minutes` | 未完了の便りをトーストで催促する間隔 (分)。0 で催促しない |
| `show_mascot` | `false` でマスコットを出さずタスクトレイだけにする |

キャラクターのフォルダには `poses\` (idle / letter / letter_calm / happy / celebrate / puzzled / sleep) と
`tray-icon\` (normal / unread / inactive / error) の PNG を置きます。足りない絵は同梱の HatoPoBlue を使います。

## 知っておいてほしいこと

- **共有フォルダに書ける人は、誰でも通知を作れ、任意のファイルや exe へのリンクを置けます。**
  利用者どうしが信頼できる職場で使う前提です (リンクの種類は制限していません)
- 利用者の名前・PC 名は Windows のログオン名 (`%USERNAME%`) とコンピューター名 (`%COMPUTERNAME%`) です。大文字小文字は区別しません
- 既読 (開いて読んだか、一斉送信を既読にしたか) は各 PC にだけ記録し、共有には書きません

## 開発

```sh
cargo test --workspace             # ロジック・DLL 境界・同時更新のテスト
cargo build --workspace --release
pwsh -File tests\vba_boundary_test.ps1     # VBA と同じ呼び方で DLL を叩く
pwsh -File scripts\package.ps1 -Version v0.1.0
```

| クレート | 成果物 | 役目 |
|---|---|---|
| `hatopo-core` | — | 通知の読み書き・検索・期限判定・archive 移動。ロジックはここだけ |
| `hatopo-app` | `HatoPo.exe` | マスコット・一覧・トレイ・トースト・導入と自己更新 |
| `hatopo-dll` | `hatopo.dll` | VBA 用 (UTF-16 BSTR で受け渡し、panic で Excel を落とさない) |
| `hatopo-cli` | `hatopo-cli.exe` | 管理者用コマンド |

VBA モジュールは `vba/HatoPo.src.txt` (UTF-8) を編集し、Shift-JIS + CRLF の `vba/HatoPo.bas` を作り直します
(`scripts/check_bas.ps1` が一致を確かめます)。
キャラクター画像は `assets/src/` のシートから `python assets/cut.py` で切り出しています。

## ライセンス

プログラムは [MIT License](LICENSE)。**キャラクター画像は MIT の対象外**で、著作権は作者に帰属します (詳細は LICENSE)。
