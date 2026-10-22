Attribute VB_Name = "HatoPo"
'==============================================================================
' HatoPo - Excel VBA から HatoPo の通知を作る・探す・変える・消すモジュール
'
' 【重要】64bit 版 Excel 専用です。
'
' 【このファイルは生成物です】
'   直接編集せず vba\HatoPo.src.txt (UTF-8) を直してから、
'   vba-export の write_bas.ps1 で HatoPo.bas (Shift-JIS + CRLF) を作り直してください。
'
' 【使い方】
'   1. VBE で「ファイル > ファイルのインポート」でこの HatoPo.bas を取り込む
'   2. 下の HATOPO_ROOT を自分の鳩の巣 (共有フォルダ) に書き換える
'      hatopo.dll は鳩の巣の直下に置く
'   3. 例:
'        id = HP_Add(HP_User("taro"), "工数表を入力してください", _
'                    本文:="10月分です", 期限:=#11/5/2026#, _
'                    URL1:=ThisWorkbook.FullName, 開始:=Date + 7)
'
' 【宛先の書き方】
'   HP_User("taro")         … そのユーザー (ログオン名) に
'   HP_Device("PC-0123")    … その PC に (誰がログオンしていても出る)
'   HP_Broadcast()          … 全員に (利用者は完了できない。HP_Delete で消す)
'   HP_Me()                 … 自分に
'
' 【日付】Date 型でも "2026/11/05 9:00" のような文字列でも渡せます。
'   開始を省略すると今すぐ、終了を省略すると完了されるまで出し続けます。
'   終了に日付だけを渡すと、その日いっぱい出します。
'
' 【失敗したとき】Err.Raise でエラーになります。Err.Description に理由が入ります。
'   Err.Number - vbObjectError - 1000 が DLL の戻り値です (HP_E_* と比べられます)。
'==============================================================================
Option Explicit

' ★ 自分の鳩の巣に書き換える
Public Const HATOPO_ROOT As String = "\\server\HatoLoft"

Private Const ERR_BASE As Long = vbObjectError + 1000

' DLL の戻り値
Public Const HP_OK As Long = 0
Public Const HP_E_GENERAL As Long = -1
Public Const HP_E_INVALID As Long = -2      ' 入力の誤り (宛先・日付など)
Public Const HP_E_NOT_FOUND As Long = -3    ' 通知が無い (完了・削除済み)
Public Const HP_E_CONFLICT As Long = -4     ' 他の人が先に変更した
Public Const HP_E_LOCKED As Long = -5       ' 他の人が変更中 (数秒後にやり直す)
Public Const HP_E_OFFLINE As Long = -6      ' 鳩の巣に繋がらない
Public Const HP_E_PANIC As Long = -999      ' DLL の不具合

' HP_Find / HP_Get が返す 2 次元配列の列 (0 行目は見出し、1 行目から通知)
Public Const HP_COL_ID As Long = 0
Public Const HP_COL_REVISION As Long = 1
Public Const HP_COL_KIND As Long = 2
Public Const HP_COL_TARGET As Long = 3
Public Const HP_COL_TITLE As Long = 4
Public Const HP_COL_BODY As Long = 5
Public Const HP_COL_DUE As Long = 6
Public Const HP_COL_URL1 As Long = 7
Public Const HP_COL_URL2 As Long = 8
Public Const HP_COL_URL3 As Long = 9
Public Const HP_COL_START As Long = 10
Public Const HP_COL_END As Long = 11
Public Const HP_COL_CREATED As Long = 12
Public Const HP_COL_CREATED_USER As Long = 13
Public Const HP_COL_CREATED_HOST As Long = 14

'------------------------------------------------------------------ 宣言部 ----
' 文字列は StrPtr() で UTF-16 のまま渡す。String のまま Declare に渡すと
' CP932 に変換されて日本語が化けるため。結果は必ず ByRef Variant で受ける。

Private Declare PtrSafe Function LoadLibraryW Lib "kernel32" ( _
    ByVal lpFileName As LongPtr) As LongPtr

Private Declare PtrSafe Function HatoPo_Version Lib "hatopo.dll" ( _
    ByRef result As Variant) As Long

Private Declare PtrSafe Function HatoPo_Add Lib "hatopo.dll" ( _
    ByVal root As LongPtr, ByVal target As LongPtr, ByVal title As LongPtr, _
    ByVal body As LongPtr, ByVal due As LongPtr, _
    ByVal url1 As LongPtr, ByVal url2 As LongPtr, ByVal url3 As LongPtr, _
    ByVal startAt As LongPtr, ByVal endAt As LongPtr, _
    ByRef result As Variant) As Long

Private Declare PtrSafe Function HatoPo_Update Lib "hatopo.dll" ( _
    ByVal root As LongPtr, ByVal id As LongPtr, ByVal revision As Long, _
    ByVal title As LongPtr, ByVal body As LongPtr, ByVal due As LongPtr, _
    ByVal url1 As LongPtr, ByVal url2 As LongPtr, ByVal url3 As LongPtr, _
    ByVal startAt As LongPtr, ByVal endAt As LongPtr, _
    ByRef result As Variant) As Long

Private Declare PtrSafe Function HatoPo_Delete Lib "hatopo.dll" ( _
    ByVal root As LongPtr, ByVal id As LongPtr, ByRef result As Variant) As Long

Private Declare PtrSafe Function HatoPo_Find Lib "hatopo.dll" ( _
    ByVal root As LongPtr, ByVal kind As LongPtr, ByVal name As LongPtr, _
    ByVal keyword As LongPtr, ByRef result As Variant) As Long

Private Declare PtrSafe Function HatoPo_Get Lib "hatopo.dll" ( _
    ByVal root As LongPtr, ByVal id As LongPtr, ByRef result As Variant) As Long

Private m_loaded As Boolean

'------------------------------------------------------------------ 準備 ------
' 鳩の巣の hatopo.dll を読み込む。各関数が最初に自動で呼ぶので、普段は呼ばなくてよい。
Private Sub EnsureLoaded()
    If m_loaded Then Exit Sub
#If Not Win64 Then
    Err.Raise ERR_BASE, "HatoPo", _
        "このモジュールは 64bit 版 Excel 専用です。" & vbCrLf & _
        "「ファイル > アカウント > Excel のバージョン情報」で確認してください。"
#End If
    Dim path As String
    path = HATOPO_ROOT & "\hatopo.dll"
    If LoadLibraryW(StrPtr(path)) = 0 Then
        Err.Raise ERR_BASE, "HatoPo", _
            "hatopo.dll を読み込めません: " & path & vbCrLf & _
            "HATOPO_ROOT の場所と、鳩の巣に hatopo.dll があるかを確認してください。"
    End If
    m_loaded = True
End Sub

Private Sub Check(ByVal rc As Long, ByRef result As Variant, ByVal fn As String)
    If rc <> HP_OK Then
        Err.Raise ERR_BASE + rc, "HatoPo." & fn, CStr(result)
    End If
End Sub

' 日付は Date でも文字列でもよい。時刻が 0 時ちょうどの Date は「日付だけ」として渡す
' (終了日なら、その日いっぱい出す)。
Private Function WhenText(ByVal v As Variant) As String
    If IsMissing(v) Or IsEmpty(v) Or IsNull(v) Then
        WhenText = ""
    ElseIf VarType(v) = vbDate Then
        If TimeValue(v) = 0 Then
            WhenText = Format$(v, "yyyy-mm-dd")
        Else
            WhenText = Format$(v, "yyyy-mm-dd hh:nn:ss")
        End If
    Else
        WhenText = Trim$(CStr(v))
    End If
End Function

'------------------------------------------------------------------ 宛先 ------
Public Function HP_User(ByVal userName As String) As String
    HP_User = "user:" & userName
End Function

Public Function HP_Device(ByVal computerName As String) As String
    HP_Device = "device:" & computerName
End Function

Public Function HP_Broadcast() As String
    HP_Broadcast = "broadcast"
End Function

Public Function HP_Me() As String
    HP_Me = "user:" & Environ$("USERNAME")
End Function

'------------------------------------------------------------------ 操作 ------
Public Function HP_Version() As String
    Dim r As Variant
    EnsureLoaded
    Check HatoPo_Version(r), r, "HP_Version"
    HP_Version = CStr(r)
End Function

' 通知を 1 件作る。戻り値は通知 ID。
Public Function HP_Add(ByVal 宛先 As String, ByVal 件名 As String, _
        Optional ByVal 本文 As String = "", Optional ByVal 期限 As Variant, _
        Optional ByVal URL1 As String = "", Optional ByVal URL2 As String = "", _
        Optional ByVal URL3 As String = "", _
        Optional ByVal 開始 As Variant, Optional ByVal 終了 As Variant) As String
    Dim r As Variant, due As String, s As String, e As String
    EnsureLoaded
    due = WhenText(期限): s = WhenText(開始): e = WhenText(終了)
    Check HatoPo_Add(StrPtr(HATOPO_ROOT), StrPtr(宛先), StrPtr(件名), StrPtr(本文), _
        StrPtr(due), StrPtr(URL1), StrPtr(URL2), StrPtr(URL3), StrPtr(s), StrPtr(e), r), _
        r, "HP_Add"
    HP_Add = CStr(r)
End Function

' 通知を書き換える。版は HP_Find / HP_Get で読んだ HP_COL_REVISION の値 (0 で確かめない)。
' 渡さなかった項目は空になる (開始だけは省略すると今の値を保つ)。戻り値は新しい版。
Public Function HP_Update(ByVal id As String, ByVal 版 As Long, ByVal 件名 As String, _
        Optional ByVal 本文 As String = "", Optional ByVal 期限 As Variant, _
        Optional ByVal URL1 As String = "", Optional ByVal URL2 As String = "", _
        Optional ByVal URL3 As String = "", _
        Optional ByVal 開始 As Variant, Optional ByVal 終了 As Variant) As Long
    Dim r As Variant, due As String, s As String, e As String
    EnsureLoaded
    due = WhenText(期限): s = WhenText(開始): e = WhenText(終了)
    Check HatoPo_Update(StrPtr(HATOPO_ROOT), StrPtr(id), 版, StrPtr(件名), StrPtr(本文), _
        StrPtr(due), StrPtr(URL1), StrPtr(URL2), StrPtr(URL3), StrPtr(s), StrPtr(e), r), _
        r, "HP_Update"
    HP_Update = CLng(r)
End Function

' 通知を消す (archive へ移す)。
Public Sub HP_Delete(ByVal id As String)
    Dim r As Variant
    EnsureLoaded
    Check HatoPo_Delete(StrPtr(HATOPO_ROOT), StrPtr(id), r), r, "HP_Delete"
End Sub

' 通知を探す。条件を空にすると絞り込まない。
' 戻り値は 0 始まりの 2 次元配列 (0 行目が見出し)。0 件でも見出し行はある。
'   種別: "user" / "device" / "broadcast"
Public Function HP_Find(Optional ByVal 種別 As String = "", _
        Optional ByVal 宛先名 As String = "", _
        Optional ByVal キーワード As String = "") As Variant
    Dim r As Variant
    EnsureLoaded
    Check HatoPo_Find(StrPtr(HATOPO_ROOT), StrPtr(種別), StrPtr(宛先名), StrPtr(キーワード), r), _
        r, "HP_Find"
    HP_Find = r
End Function

' ID で 1 件取る (見出し + 1 行)。
Public Function HP_Get(ByVal id As String) As Variant
    Dim r As Variant
    EnsureLoaded
    Check HatoPo_Get(StrPtr(HATOPO_ROOT), StrPtr(id), r), r, "HP_Get"
    HP_Get = r
End Function

' 検索結果をシートに貼る (見出し付き)。
Public Sub HP_FindToSheet(ByVal 貼り付け先 As Range, Optional ByVal 種別 As String = "", _
        Optional ByVal 宛先名 As String = "", Optional ByVal キーワード As String = "")
    Dim a As Variant
    a = HP_Find(種別, 宛先名, キーワード)
    貼り付け先.Resize(UBound(a, 1) + 1, UBound(a, 2) + 1).Value = a
End Sub

'------------------------------------------------------------------ 例 --------
' 入力した人に、1 週間後の朝 9 時から「確認してください」と知らせる例。
Public Sub HP_Sample_1週間後に自分へ通知()
    Dim id As String
    id = HP_Add(HP_Me(), "入力内容を確認してください", _
        本文:="先週入力したデータの確認をお願いします。", _
        URL1:=ThisWorkbook.FullName, _
        開始:=Date + 7 + TimeSerial(9, 0, 0), _
        終了:=Date + 14)
    MsgBox "通知を予約しました: " & id, vbInformation, "HatoPo"
End Sub
