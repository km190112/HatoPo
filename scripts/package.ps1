# 配布物の zip を作る。リリースの GitHub Actions と手元の両方から使う。
#
#   pwsh -File scripts\package.ps1 -Version v0.1.0
#
# 前提: cargo build --workspace --release を済ませてあること。
# 中身:
#   HatoLoft\          … 共有フォルダへそのまま置く「鳩の巣」一式
#     HatoPo.exe / HatoPoAdmin.exe / hatopo.dll / hatopo-cli.exe / settings.json
#     character_icon\HatoPoBlue\ (poses, tray-icon)
#     autostart\         … 自動起動の登録 (自動起動を登録.bat / register-autostart.ps1)
#     broadcast\ PigeonHole\user\ PigeonHole\device\ archive\ status\ read\
#   vba\HatoPo.bas     … Excel にインポートするモジュール
#   はじめにお読みください.txt / キャラクター画像の作り方.md / THIRD-PARTY-NOTICES.md

param(
    [Parameter(Mandatory)] [string]$Version
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$root = Split-Path -Parent $PSScriptRoot
$rel = Join-Path $root 'target\release'
$dist = Join-Path $root 'dist'
$stage = Join-Path $dist "HatoPo-$Version"

$programs = @('HatoPo.exe', 'HatoPoAdmin.exe', 'hatopo.dll', 'hatopo-cli.exe')
foreach ($f in $programs) {
    if (-not (Test-Path -LiteralPath (Join-Path $rel $f))) {
        throw "$f がありません。先に cargo build --workspace --release を実行してください。"
    }
}

if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
$loft = Join-Path $stage 'HatoLoft'
New-Item -ItemType Directory -Force $loft | Out-Null

foreach ($f in $programs) {
    Copy-Item -LiteralPath (Join-Path $rel $f) -Destination $loft
}
Copy-Item -LiteralPath (Join-Path $root 'loft-template\settings.json') -Destination $loft
Copy-Item -LiteralPath (Join-Path $root 'loft-template\character_icon') -Destination $loft -Recurse
Copy-Item -LiteralPath (Join-Path $root 'loft-template\autostart') -Destination $loft -Recurse

# zip は空のフォルダを入れないので、置き場所が分かるように説明を 1 枚ずつ置く
$dirs = @{
    'broadcast'          = '全員へのお知らせを置く場所です。'
    'PigeonHole\user'    = 'ユーザー宛ての便りを、ユーザー名のフォルダに置く場所です。'
    'PigeonHole\device'  = 'PC 宛ての便りを、コンピューター名のフォルダに置く場所です。'
    'archive'            = '完了・期限切れ・削除した便りが月ごとに移る場所です。'
    'status'             = '各 PC の HatoPo が最終アクセスや版を書く場所です。全員が書けるようにしてください。'
    'read'               = '「既読にする」を押した印を置く場所です。全員が書けるようにしてください。'
}
foreach ($d in $dirs.Keys) {
    $p = Join-Path $loft $d
    New-Item -ItemType Directory -Force $p | Out-Null
    # .json ではないので HatoPo は読み飛ばす
    Set-Content -LiteralPath (Join-Path $p '_この場所について.txt') -Value $dirs[$d] -Encoding utf8BOM
}

New-Item -ItemType Directory -Force (Join-Path $stage 'vba') | Out-Null
Copy-Item -LiteralPath (Join-Path $root 'vba\HatoPo.bas') -Destination (Join-Path $stage 'vba')
Copy-Item -LiteralPath (Join-Path $root 'loft-template\はじめにお読みください.txt') -Destination $stage
Copy-Item -LiteralPath (Join-Path $root 'docs\キャラクター画像の作り方.md') -Destination $stage
Copy-Item -LiteralPath (Join-Path $root 'THIRD-PARTY-NOTICES.md') -Destination $stage

$zip = Join-Path $dist "HatoPo-$Version.zip"
if (Test-Path -LiteralPath $zip) { Remove-Item -LiteralPath $zip -Force }
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip
Write-Host "作成: $zip"
@(Get-ChildItem -LiteralPath $stage -Recurse -File) | ForEach-Object { $_.FullName.Substring($stage.Length + 1) }
