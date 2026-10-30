#Requires -Version 5.1
<#
  HatoPo をログオン時に自動で起動するよう、タスクスケジューラへ登録する。

  直接は実行せず、隣の「自動起動を登録.bat」から呼ぶ。

  -- 保存の条件 (守らないと現場で壊れる) --
  ・UTF-8 BOM 付き + CRLF。BOM 無しだと Windows PowerShell 5.1 が CP932 として読み、
    日本語が壊れたうえ構文エラーになる
  ・ファイル名は ASCII。CP932 の .bat から日本語名を渡すとコードページ境界で壊れうる
  ・5.1 で動作確認する。.bat から powershell を呼ぶと 7 ではなく 5.1 が起動する

  -- なぜ schtasks /sc onlogon を使わないのか --
  /sc onlogon は /ru を省略すると「任意のユーザーのログオン時」になり、他人にも影響する
  ため管理者権限が要る。ダブルクリック起動は昇格していないので必ず落ちる。
  Register-ScheduledTask で -User と LogonType Interactive を指定すれば昇格が要らない。

  -- 解除について --
  解除はこのスクリプトでは行わない。HatoPo.exe --uninstall で解除する
  (schtasks /delete は persistence を作らない操作なので、exe 側に残している)。
#>

$ErrorActionPreference = 'Stop'
try {
    $cp = [int]((chcp) -replace '[^\d]')
    [Console]::OutputEncoding = [System.Text.Encoding]::GetEncoding($cp)
} catch { }

$TaskName = 'HatoPo'
$UserName = $env:USERNAME
$Account  = if ($env:USERDOMAIN) { "$env:USERDOMAIN\$env:USERNAME" } else { $env:USERNAME }

# ログオン名のフォルダに入れる。1 台を複数人で使っても互いの登録を消さない。
# HatoPo.exe 内部の自動登録 (旧版) も同じ場所・同じ名前なので、ここで上書きされる。
$TaskPath = "\$UserName\"

# 登録対象は常にローカルへ導入された exe (共有の exe は直接常駐させない設計のため)。
$exe = Join-Path $env:LOCALAPPDATA 'HatoPo\HatoPo.exe'

if (-not (Test-Path -LiteralPath $exe)) {
    Write-Host ''
    Write-Host ' [エラー] HatoPo がまだ導入されていません。'
    Write-Host "          見つかりませんでした: $exe"
    Write-Host '          先に共有フォルダの HatoPo.exe をダブルクリックして導入してください。'
    exit 1
}

Write-Host ''
Write-Host ' HatoPo を自動起動に登録します。'
Write-Host "   実行するもの : $exe"
Write-Host "   登録する先   : タスクスケジューラ の $TaskPath$TaskName"
Write-Host "   実行ユーザー : $Account"
Write-Host ''

# 既存の登録 (HatoPo.exe 内部の自動登録を含む) があれば消してから登録する。
$existing = Get-ScheduledTask -TaskPath $TaskPath -TaskName $TaskName -ErrorAction SilentlyContinue
if ($existing) {
    Unregister-ScheduledTask -TaskPath $TaskPath -TaskName $TaskName -Confirm:$false
    Write-Host '  以前の登録を解除しました。'
}

try {
    # -WorkingDirectory を省略すると C:\Windows\system32 が基準になる。
    $act = New-ScheduledTaskAction -Execute $exe -WorkingDirectory (Split-Path $exe -Parent)

    # -User を付けて「このユーザーのログオン時」に限定する。省略すると管理者権限が要る。
    $trg = New-ScheduledTaskTrigger -AtLogOn -User $Account

    # Interactive = 本人のトークンで動く。SYSTEM で動かすと共有パス (\\server\...) が見えない。
    $pri = New-ScheduledTaskPrincipal -UserId $Account -LogonType Interactive -RunLevel Limited

    # 既定のままだと ExecutionTimeLimit=72時間 で常駐プロセスが3日で殺され、
    # 電池関係が true だとノート PC の電源を抜くと止まり、RunOnlyIfNetworkAvailable が
    # true だと共有フォルダへの接続が遅いときに起動自体が行われない。すべて潰す。
    $set = New-ScheduledTaskSettingsSet `
        -MultipleInstances IgnoreNew `
        -ExecutionTimeLimit ([TimeSpan]::Zero) `
        -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
        -StartWhenAvailable `
        -RunOnlyIfNetworkAvailable:$false `
        -RestartInterval (New-TimeSpan -Minutes 1) -RestartCount 3

    Register-ScheduledTask -TaskName $TaskName -TaskPath $TaskPath `
        -Action $act -Trigger $trg -Principal $pri -Settings $set -Force | Out-Null
} catch {
    Write-Host ''
    Write-Host " [エラー] 登録に失敗しました: $($_.Exception.Message)"
    Write-Host ''
    Write-Host ' この画面をそのまま情報システム担当に見せてください。'
    exit 1
}

Write-Host ' 登録しました。次回ログオンから自動で起動します。'
Write-Host ''
Write-Host " 登録内容の確認 : schtasks /query /tn `"$UserName\$TaskName`""
Write-Host ' 自動起動を止めたいときは、HatoPo.exe --uninstall を実行してください。'
exit 0
