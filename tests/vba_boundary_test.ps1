# VBA 境界の自動テスト (Excel を起動しない)。
#
# .NET の [MarshalAs(UnmanagedType.Struct)] ref object は VARIANT* として渡る。
# これは VBA の ByRef result As Variant と同じ ABI。入力の文字列も
# Marshal.StringToBSTR (= VBA の StrPtr が指す BSTR) で渡すので、VBA から呼んだときと同じ形になる。
#
# 使い方:  pwsh -File tests\vba_boundary_test.ps1 [-Config debug]

param(
    [ValidateSet('debug', 'release')]
    [string]$Config = 'release'
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$root = Split-Path -Parent $PSScriptRoot
$dll = Join-Path $root "target\$Config\hatopo.dll"

$script:passed = 0
$script:failed = 0
function Assert-That([bool]$Condition, [string]$Name, [string]$Detail = '') {
    if ($Condition) { $script:passed++; Write-Host "  [OK]   $Name" -ForegroundColor Green }
    else { $script:failed++; Write-Host "  [FAIL] $Name  $Detail" -ForegroundColor Red }
}

Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class Hp {
    const string L = @"$dll";
    [DllImport(L)] public static extern int HatoPo_Version([MarshalAs(UnmanagedType.Struct)] ref object r);
    [DllImport(L)] public static extern int HatoPo_Add(IntPtr root, IntPtr target, IntPtr title, IntPtr body, IntPtr due,
        IntPtr u1, IntPtr u2, IntPtr u3, IntPtr start, IntPtr end, [MarshalAs(UnmanagedType.Struct)] ref object r);
    [DllImport(L)] public static extern int HatoPo_Update(IntPtr root, IntPtr id, int rev, IntPtr title, IntPtr body, IntPtr due,
        IntPtr u1, IntPtr u2, IntPtr u3, IntPtr start, IntPtr end, [MarshalAs(UnmanagedType.Struct)] ref object r);
    [DllImport(L)] public static extern int HatoPo_Delete(IntPtr root, IntPtr id, [MarshalAs(UnmanagedType.Struct)] ref object r);
    [DllImport(L)] public static extern int HatoPo_Find(IntPtr root, IntPtr kind, IntPtr name, IntPtr kw, [MarshalAs(UnmanagedType.Struct)] ref object r);
    [DllImport(L)] public static extern int HatoPo_Get(IntPtr root, IntPtr id, [MarshalAs(UnmanagedType.Struct)] ref object r);
}
"@

# VBA の StrPtr(s) と同じ BSTR。"" は vbNullString (0) として渡す
function B([string]$s) { if ($null -eq $s -or $s -eq '') { [IntPtr]::Zero } else { [Runtime.InteropServices.Marshal]::StringToBSTR($s) } }

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("hatopo-test-" + [Guid]::NewGuid())
$loft = Join-Path $tmp '鳩の巣'
New-Item -ItemType Directory -Force $loft | Out-Null
$Z = [IntPtr]::Zero

try {
    Write-Host "hatopo.dll ($Config) の VBA 境界テスト"

    $r = $null
    Assert-That ([Hp]::HatoPo_Version([ref]$r) -eq 0 -and $r -match '^\d+\.\d+\.\d+$') 'HatoPo_Version' "$r"

    $r = $null
    $rc = [Hp]::HatoPo_Add((B $loft), (B 'user:Taro'), (B '工数表の入力 🕊️'), (B "10月分`r`n2 行目"), (B '2026/11/05'),
        (B '\server\共有\工数表.xlsm'), $Z, $Z, $Z, (B '2026-12-31'), [ref]$r)
    Assert-That ($rc -eq 0) 'HatoPo_Add 成功' "rc=$rc $r"
    $id = [string]$r

    $r = $null
    $rc = [Hp]::HatoPo_Find((B $loft), $Z, (B 'taro'), $Z, [ref]$r)
    Assert-That ($rc -eq 0) 'HatoPo_Find 成功' "rc=$rc"
    Assert-That ($r.Rank -eq 2 -and $r.GetLength(0) -eq 2 -and $r.GetLength(1) -eq 15) '見出し + 1 行 x 15 列'
    Assert-That ($r[0, 0] -eq 'ID' -and $r[1, 0] -eq $id) 'ID 列'
    Assert-That ($r[1, 4] -eq '工数表の入力 🕊️') '日本語と絵文字が化けない' "$($r[1,4])"
    Assert-That ($r[1, 5] -eq "10月分`n2 行目") '改行は LF にそろう'
    Assert-That ($r[1, 6] -is [datetime] -and $r[1, 6] -eq [datetime]'2026-11-05') '期限は Date 型'
    Assert-That ($r[1, 7] -eq '\server\共有\工数表.xlsm') 'URL1'
    Assert-That ($r[1, 11] -eq [datetime]'2026-12-31 23:59:59') '終了日はその日いっぱい'
    Assert-That ($r[1, 1] -eq 1) '版は 1'

    $r = $null
    $rc = [Hp]::HatoPo_Update((B $loft), (B $id), 1, (B '更新後'), $Z, $Z, $Z, $Z, $Z, $Z, $Z, [ref]$r)
    Assert-That ($rc -eq 0 -and $r -eq 2) 'HatoPo_Update で版が 2 に' "rc=$rc $r"
    $r = $null
    $rc = [Hp]::HatoPo_Update((B $loft), (B $id), 1, (B '古い版から'), $Z, $Z, $Z, $Z, $Z, $Z, $Z, [ref]$r)
    Assert-That ($rc -eq -4 -and "$r" -match '他の人が先に変更') '古い版は HP_E_CONFLICT' "rc=$rc"

    $r = $null
    Assert-That ([Hp]::HatoPo_Get((B $loft), (B $id), [ref]$r) -eq 0 -and $r[1, 4] -eq '更新後') 'HatoPo_Get'

    $r = $null
    $rc = [Hp]::HatoPo_Add((B $loft), (B 'group:x'), (B 'x'), $Z, $Z, $Z, $Z, $Z, $Z, $Z, [ref]$r)
    Assert-That ($rc -eq -2 -and "$r" -match '宛先の種類') '誤った宛先は HP_E_INVALID' "rc=$rc $r"

    $r = 'old value'
    Assert-That ([Hp]::HatoPo_Delete((B $loft), (B $id), [ref]$r) -eq 0 -and $null -eq $r) 'HatoPo_Delete (結果は Empty)'
    $r = $null
    Assert-That ([Hp]::HatoPo_Delete((B $loft), (B $id), [ref]$r) -eq -3) '2 回目は HP_E_NOT_FOUND'

    $r = $null
    $rc = [Hp]::HatoPo_Find((B $loft), $Z, $Z, $Z, [ref]$r)
    Assert-That ($rc -eq 0 -and $r.GetLength(0) -eq 1) '0 件でも見出し行がある'

    $r = $null
    $rc = [Hp]::HatoPo_Add((B '\no-such-server-hatopo\x'), (B 'user:a'), (B 'x'), $Z, $Z, $Z, $Z, $Z, $Z, $Z, [ref]$r)
    Assert-That ($rc -eq -6) '繋がらない鳩の巣は HP_E_OFFLINE' "rc=$rc"
}
finally {
    Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host ""
Write-Host "合格 $script:passed / 不合格 $script:failed"
if ($script:failed -gt 0) { exit 1 }
