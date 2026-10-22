# vba\HatoPo.bas が vba\HatoPo.src.txt (UTF-8) から作った状態のままかを確かめる。
# .bas は VBE が CP932 として読むので、CP932 + CRLF + BOM なしでなければならない。
#   pwsh -File scripts\check_bas.ps1

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$root = Split-Path -Parent $PSScriptRoot
$bas = [IO.File]::ReadAllBytes((Join-Path $root 'vba\HatoPo.bas'))
$src = [IO.File]::ReadAllText((Join-Path $root 'vba\HatoPo.src.txt'), [Text.Encoding]::UTF8)

# PowerShell 7 (.NET Core) は CP932 を持たないので登録する
[Text.Encoding]::RegisterProvider([Text.CodePagesEncodingProvider]::Instance)
$cp932 = [Text.Encoding]::GetEncoding(932, [Text.EncoderFallback]::ExceptionFallback, [Text.DecoderFallback]::ExceptionFallback)

$fail = @()
if ($bas.Length -ge 3 -and $bas[0] -eq 0xEF -and $bas[1] -eq 0xBB -and $bas[2] -eq 0xBF) { $fail += 'UTF-8 の BOM が付いている' }
$text = $cp932.GetString($bas)
if ($text -match "(?<!`r)`n") { $fail += 'CRLF でない行末がある' }
$want = ($src -replace "`r`n", "`n" -replace "`n", "`r`n")
if (-not $want.EndsWith("`r`n")) { $want += "`r`n" }
if ($text.TrimEnd() -ne $want.TrimEnd()) { $fail += 'HatoPo.src.txt と中身が一致しない (write_bas.ps1 で作り直す)' }

if ($fail.Count -gt 0) {
    $fail | ForEach-Object { Write-Host "::error::HatoPo.bas: $_" }
    exit 1
}
Write-Host 'HatoPo.bas: OK (CP932 / CRLF / ソースと一致)'
