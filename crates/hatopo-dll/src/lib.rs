//! hatopo.dll — Excel VBA から HatoPo の通知を作る・探す・変える・消すための DLL (64bit Office 専用)。
//!
//! # 呼び出し規約 (duckdb-vba-dll と同じ)
//!
//! - 文字列の引数は `ByVal x As LongPtr` で `StrPtr(s)` を渡す。ANSI 変換を通らないので日本語が化けない
//! - 結果は最後の引数 `ByRef result As Variant` に入る。失敗したときはエラーメッセージ (文字列) が入る
//! - 戻り値は `Long`。0 が成功、負がエラー (`HP_E_*`)
//! - panic は `catch_unwind` で捕まえて文字列で返す。Excel を落とさない
//!   (そのため release で `panic = "abort"` にしてはいけない)

mod oleaut;

use chrono::{Local, NaiveDate, NaiveDateTime};
use hatopo_core::{
    CloseReason, DayEdge, Error, Filter, Kind, Loft, NewNotice, Notice, Origin, Target, parse_date,
    parse_when,
};
use oleaut::{VARIANT, bstr_arg, write_out};

pub const HP_OK: i32 = 0;
pub const HP_E_GENERAL: i32 = -1;
pub const HP_E_INVALID: i32 = -2;
pub const HP_E_NOT_FOUND: i32 = -3;
pub const HP_E_CONFLICT: i32 = -4;
pub const HP_E_LOCKED: i32 = -5;
pub const HP_E_OFFLINE: i32 = -6;
pub const HP_E_PANIC: i32 = -999;

/// `HatoPo_Find` が返す表の見出し。VBA 側の列定数 (`HP_COL_*`) と順番を揃えること。
const COLUMNS: [&str; 15] = [
    "ID",
    "版",
    "宛先種別",
    "宛先",
    "件名",
    "本文",
    "依頼期限",
    "URL1",
    "URL2",
    "URL3",
    "通知開始",
    "通知終了",
    "設定日時",
    "設定ユーザー",
    "設定端末",
];

fn code(e: &Error) -> i32 {
    match e {
        Error::Invalid(_) => HP_E_INVALID,
        Error::NotFound { .. } => HP_E_NOT_FOUND,
        Error::Conflict { .. } => HP_E_CONFLICT,
        Error::Locked { .. } => HP_E_LOCKED,
        Error::Offline { .. } => HP_E_OFFLINE,
        _ => HP_E_GENERAL,
    }
}

/// FFI 境界の共通処理。成功でも失敗でも `out` に書くので、VBA 側の分岐が 1 本で済む。
fn guard(out: *mut VARIANT, f: impl FnOnce() -> Result<VARIANT, Error>) -> i32 {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(v)) => {
            unsafe { write_out(out, v) };
            HP_OK
        }
        Ok(Err(e)) => {
            unsafe { write_out(out, VARIANT::bstr(&e.to_string())) };
            code(&e)
        }
        Err(payload) => {
            let detail = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "詳細不明".into());
            let msg = format!(
                "HatoPo の内部エラー (panic): {detail}\nDLL の不具合です。操作内容と合わせて報告してください。"
            );
            unsafe { write_out(out, VARIANT::bstr(&msg)) };
            HP_E_PANIC
        }
    }
}

fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

/// VBA から渡された文字列群を通知の入力にする。
#[allow(clippy::too_many_arguments)]
fn new_notice(
    title: String,
    body: String,
    due: String,
    url1: String,
    url2: String,
    url3: String,
    start: String,
    end: String,
) -> Result<NewNotice, Error> {
    Ok(NewNotice {
        title,
        // セル内改行 (vbLf) も vbCrLf も LF にそろえる
        body: body.replace("\r\n", "\n"),
        due: parse_date(&due)?,
        urls: vec![url1, url2, url3],
        start_at: parse_when(&start, DayEdge::Start)?,
        end_at: parse_when(&end, DayEdge::End)?,
    })
}

/// OLE の日付 (1899-12-30 からの日数)。
fn ole_date(dt: NaiveDateTime) -> VARIANT {
    let epoch = NaiveDate::from_ymd_opt(1899, 12, 30)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    let secs = (dt - epoch).num_seconds() as f64;
    VARIANT::date(secs / 86_400.0)
}

fn opt_date(dt: Option<NaiveDateTime>) -> VARIANT {
    dt.map(ole_date).unwrap_or_else(VARIANT::empty)
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Broadcast => "broadcast",
        Kind::Device => "device",
        Kind::User => "user",
    }
}

/// 見出し 1 行 + 通知 1 件 1 行の 0 始まり 2 次元配列。0 件でも見出し行はあるので `UBound` が落ちない。
fn table(list: &[Notice]) -> Result<VARIANT, Error> {
    let mut cells = Vec::with_capacity((list.len() + 1) * COLUMNS.len());
    cells.extend(COLUMNS.iter().map(|c| VARIANT::bstr(c)));
    for n in list {
        let url = |i: usize| VARIANT::bstr(n.urls.get(i).map(String::as_str).unwrap_or(""));
        cells.extend([
            VARIANT::bstr(&n.id),
            VARIANT::i32(n.revision as i32),
            VARIANT::bstr(kind_name(n.target.kind)),
            VARIANT::bstr(&n.target.name),
            VARIANT::bstr(&n.title),
            VARIANT::bstr(&n.body),
            opt_date(n.due.map(|d| d.and_hms_opt(0, 0, 0).unwrap())),
            url(0),
            url(1),
            url(2),
            ole_date(n.start_at),
            opt_date(n.end_at),
            ole_date(n.created_at),
            VARIANT::bstr(&n.created_by.user),
            VARIANT::bstr(&n.created_by.host),
        ]);
    }
    VARIANT::grid(list.len() + 1, COLUMNS.len(), cells)
        .ok_or_else(|| Error::Invalid("結果の配列を確保できませんでした (メモリ不足)。".into()))
}

// ---------------------------------------------------------------------------
// エクスポート関数
// ---------------------------------------------------------------------------

/// DLL の版を返す。
///
/// # Safety
/// `out` は有効な VARIANT* か null。
#[unsafe(no_mangle)]
pub unsafe extern "system" fn HatoPo_Version(out: *mut VARIANT) -> i32 {
    guard(out, || Ok(VARIANT::bstr(env!("CARGO_PKG_VERSION"))))
}

/// 通知を 1 件作る。`result` に ID が入る。
///
/// `target` は `user:taro` / `device:PC-0123` / `broadcast`。日付は `2026/11/05` や `2026-11-05 09:00`。
/// 空の開始日は今すぐ、空の終了日は完了まで。
///
/// # Safety
/// 文字列引数は有効な BSTR か null、`out` は有効な VARIANT* か null。
#[unsafe(no_mangle)]
pub unsafe extern "system" fn HatoPo_Add(
    root: *const u16,
    target: *const u16,
    title: *const u16,
    body: *const u16,
    due: *const u16,
    url1: *const u16,
    url2: *const u16,
    url3: *const u16,
    start: *const u16,
    end: *const u16,
    out: *mut VARIANT,
) -> i32 {
    guard(out, || unsafe {
        let n = new_notice(
            bstr_arg(title),
            bstr_arg(body),
            bstr_arg(due),
            bstr_arg(url1),
            bstr_arg(url2),
            bstr_arg(url3),
            bstr_arg(start),
            bstr_arg(end),
        )?;
        let t = Target::parse(&bstr_arg(target))?;
        let made = Loft::new(bstr_arg(root)).add(&t, &n, &Origin::current(), now())?;
        Ok(VARIANT::bstr(&made.id))
    })
}

/// 通知を書き換える。`revision` は `HatoPo_Find` で読んだ版 (0 で確かめない)。`result` に新しい版が入る。
///
/// # Safety
/// `HatoPo_Add` と同じ。
#[unsafe(no_mangle)]
pub unsafe extern "system" fn HatoPo_Update(
    root: *const u16,
    id: *const u16,
    revision: i32,
    title: *const u16,
    body: *const u16,
    due: *const u16,
    url1: *const u16,
    url2: *const u16,
    url3: *const u16,
    start: *const u16,
    end: *const u16,
    out: *mut VARIANT,
) -> i32 {
    guard(out, || unsafe {
        let n = new_notice(
            bstr_arg(title),
            bstr_arg(body),
            bstr_arg(due),
            bstr_arg(url1),
            bstr_arg(url2),
            bstr_arg(url3),
            bstr_arg(start),
            bstr_arg(end),
        )?;
        let updated = Loft::new(bstr_arg(root)).update(
            &bstr_arg(id),
            revision.max(0) as u64,
            &n,
            &Origin::current(),
            now(),
        )?;
        Ok(VARIANT::i32(updated.revision as i32))
    })
}

/// 通知を消す (archive へ移す)。
///
/// # Safety
/// `HatoPo_Add` と同じ。
#[unsafe(no_mangle)]
pub unsafe extern "system" fn HatoPo_Delete(
    root: *const u16,
    id: *const u16,
    out: *mut VARIANT,
) -> i32 {
    guard(out, || unsafe {
        Loft::new(bstr_arg(root)).close(
            &bstr_arg(id),
            CloseReason::Deleted,
            &Origin::current(),
            now(),
        )?;
        Ok(VARIANT::empty())
    })
}

/// 通知を探す。空の条件は絞り込まない。`result` は見出し付きの 2 次元配列 (0 始まり)。
///
/// # Safety
/// `HatoPo_Add` と同じ。
#[unsafe(no_mangle)]
pub unsafe extern "system" fn HatoPo_Find(
    root: *const u16,
    kind: *const u16,
    name: *const u16,
    keyword: *const u16,
    out: *mut VARIANT,
) -> i32 {
    guard(out, || unsafe {
        let some = |s: String| Some(s).filter(|s| !s.trim().is_empty());
        let filter = Filter {
            kind: some(bstr_arg(kind))
                .as_deref()
                .map(Kind::parse)
                .transpose()?,
            name: some(bstr_arg(name)),
            keyword: some(bstr_arg(keyword)),
        };
        let (found, _broken) = Loft::new(bstr_arg(root)).find(&filter)?;
        table(&found)
    })
}

/// ID で 1 件取る。`result` は見出し + 1 行の 2 次元配列。
///
/// # Safety
/// `HatoPo_Add` と同じ。
#[unsafe(no_mangle)]
pub unsafe extern "system" fn HatoPo_Get(
    root: *const u16,
    id: *const u16,
    out: *mut VARIANT,
) -> i32 {
    guard(out, || unsafe {
        let n = Loft::new(bstr_arg(root)).get(&bstr_arg(id))?;
        table(std::slice::from_ref(&n))
    })
}

#[cfg(test)]
mod tests;
