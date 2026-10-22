//! VBA と同じ形 (BSTR を StrPtr で渡し、VARIANT* で受ける) でエクスポート関数を呼ぶ。

use super::*;
use crate::oleaut::*;
use std::ptr::null;

/// VBA の String と同じ BSTR。Drop で解放する。
struct Bstr(*mut u16);

impl Bstr {
    fn new(s: &str) -> Bstr {
        let w: Vec<u16> = s.encode_utf16().collect();
        Bstr(unsafe { SysAllocStringLen(w.as_ptr(), w.len() as u32) })
    }
    fn p(&self) -> *const u16 {
        self.0
    }
}

impl Drop for Bstr {
    fn drop(&mut self) {
        let mut v = VARIANT::empty();
        v.vt = VT_BSTR;
        v.value.bstrVal = self.0;
        unsafe { VariantClear(&mut v) };
    }
}

/// VBA の `Dim result As Variant`。
struct Out(VARIANT);

impl Out {
    fn new() -> Out {
        Out(VARIANT::empty())
    }
    fn p(&mut self) -> *mut VARIANT {
        &mut self.0
    }
    fn text(&self) -> String {
        assert_eq!(self.0.vt, VT_BSTR, "文字列ではない: vt={}", self.0.vt);
        unsafe { bstr_arg(self.0.value.bstrVal) }
    }
    fn int(&self) -> i32 {
        assert_eq!(self.0.vt, VT_I4);
        unsafe { self.0.value.lVal }
    }
    fn rows_cols(&self) -> (i32, i32) {
        assert_eq!(self.0.vt, VT_ARRAY | VT_VARIANT);
        let (mut r, mut c) = (0, 0);
        unsafe {
            SafeArrayGetUBound(self.0.value.parray, 1, &mut r);
            SafeArrayGetUBound(self.0.value.parray, 2, &mut c);
        }
        (r, c)
    }
    /// VBA の `result(row, col)`。
    fn cell(&self, row: i32, col: i32) -> VARIANT {
        let mut v = VARIANT::empty();
        // SafeArrayGetElement の添字は VBA と同じく左の次元 (行) が先頭
        let idx = [row, col];
        let hr = unsafe {
            SafeArrayGetElement(
                self.0.value.parray,
                idx.as_ptr(),
                &mut v as *mut VARIANT as *mut _,
            )
        };
        assert_eq!(hr, 0);
        v
    }
    fn cell_text(&self, row: i32, col: i32) -> String {
        let mut v = self.cell(row, col);
        let s = unsafe { bstr_arg(v.value.bstrVal) };
        unsafe { VariantClear(&mut v) };
        s
    }
}

impl Drop for Out {
    fn drop(&mut self) {
        unsafe { VariantClear(&mut self.0) };
    }
}

fn root() -> (tempfile::TempDir, Bstr) {
    let d = tempfile::tempdir().unwrap();
    let r = Bstr::new(&d.path().join("鳩の巣").to_string_lossy());
    std::fs::create_dir(d.path().join("鳩の巣")).unwrap();
    (d, r)
}

fn add(root: &Bstr, target: &str, title: &str, end: &str, out: &mut Out) -> i32 {
    let (t, ti, b, du, u1, e) = (
        Bstr::new(target),
        Bstr::new(title),
        Bstr::new("本文\r\n2 行目 🕊️"),
        Bstr::new("2026/11/05"),
        Bstr::new(r"\\server\共有\工数表.xlsm"),
        Bstr::new(end),
    );
    unsafe {
        HatoPo_Add(
            root.p(),
            t.p(),
            ti.p(),
            b.p(),
            du.p(),
            u1.p(),
            null(),
            null(),
            null(),
            e.p(),
            out.p(),
        )
    }
}

#[test]
fn version_is_returned() {
    let mut out = Out::new();
    assert_eq!(unsafe { HatoPo_Version(out.p()) }, HP_OK);
    assert_eq!(out.text(), env!("CARGO_PKG_VERSION"));
}

#[test]
fn add_then_find_round_trips_japanese_and_emoji() {
    let (_d, r) = root();
    let mut id = Out::new();
    assert_eq!(add(&r, "user:Taro", "工数入力 📝", "", &mut id), HP_OK);
    let id = id.text();

    let mut res = Out::new();
    let empty = Bstr::new("");
    let rc = unsafe { HatoPo_Find(r.p(), empty.p(), null(), null(), res.p()) };
    assert_eq!(rc, HP_OK);
    assert_eq!(res.rows_cols(), (1, COLUMNS.len() as i32 - 1));
    assert_eq!(res.cell_text(0, 0), "ID");
    assert_eq!(res.cell_text(1, 0), id);
    assert_eq!(res.cell_text(1, 3), "taro");
    assert_eq!(res.cell_text(1, 4), "工数入力 📝");
    assert_eq!(
        res.cell_text(1, 5),
        "本文\n2 行目 🕊️",
        "CRLF は LF にそろう"
    );
    assert_eq!(res.cell_text(1, 7), r"\\server\共有\工数表.xlsm");
    assert_eq!(res.cell_text(1, 8), "");

    let due = res.cell(1, 6);
    assert_eq!(due.vt, VT_DATE);
    // 2026-11-05 の OLE 日付 (Excel のシリアル値と同じ)
    assert_eq!(unsafe { due.value.date }, 46_331.0);
    assert_eq!(res.cell(1, 11).vt, VT_EMPTY, "終了日なしは Empty");
}

#[test]
fn find_with_no_hits_still_has_header_row() {
    let (_d, r) = root();
    let mut res = Out::new();
    let kw = Bstr::new("存在しない");
    assert_eq!(
        unsafe { HatoPo_Find(r.p(), null(), null(), kw.p(), res.p()) },
        HP_OK
    );
    assert_eq!(res.rows_cols().0, 0);
}

#[test]
fn update_conflict_and_delete_codes() {
    let (_d, r) = root();
    let mut id = Out::new();
    add(&r, "device:PC-1", "更新してください", "2026-12-31", &mut id);
    let id = Bstr::new(&id.text());
    let title = Bstr::new("v2");

    let call_update = |rev: i32, out: &mut Out| unsafe {
        HatoPo_Update(
            r.p(),
            id.p(),
            rev,
            title.p(),
            null(),
            null(),
            null(),
            null(),
            null(),
            null(),
            null(),
            out.p(),
        )
    };
    let mut out = Out::new();
    assert_eq!(call_update(1, &mut out), HP_OK);
    assert_eq!(out.int(), 2);
    assert_eq!(call_update(1, &mut out), HP_E_CONFLICT);
    assert!(out.text().contains("他の人が先に変更"));

    assert_eq!(unsafe { HatoPo_Delete(r.p(), id.p(), out.p()) }, HP_OK);
    assert_eq!(out.0.vt, VT_EMPTY);
    assert_eq!(
        unsafe { HatoPo_Delete(r.p(), id.p(), out.p()) },
        HP_E_NOT_FOUND
    );
    assert_eq!(
        unsafe { HatoPo_Get(r.p(), id.p(), out.p()) },
        HP_E_NOT_FOUND
    );
}

#[test]
fn bad_input_returns_japanese_message_not_crash() {
    let (_d, r) = root();
    let mut out = Out::new();
    assert_eq!(add(&r, "group:x", "t", "", &mut out), HP_E_INVALID);
    assert!(out.text().contains("宛先の種類"));
    assert_eq!(add(&r, "user:taro", "t", "来週", &mut out), HP_E_INVALID);
    assert!(out.text().contains("読めません"));
    assert_eq!(add(&r, "user:taro", "", "", &mut out), HP_E_INVALID);

    // 全部 null (VBA の vbNullString) でも落ちない
    let rc = unsafe {
        HatoPo_Add(
            null(),
            null(),
            null(),
            null(),
            null(),
            null(),
            null(),
            null(),
            null(),
            null(),
            out.p(),
        )
    };
    assert!(rc < 0);
}

#[test]
fn offline_root_is_its_own_code() {
    let mut out = Out::new();
    let r = Bstr::new(r"\\no-such-server-hatopo\HatoLoft");
    assert_eq!(add(&r, "user:taro", "x", "", &mut out), HP_E_OFFLINE);
    assert!(out.text().contains("接続できません"));
}

#[test]
fn result_variant_holding_old_string_is_freed_and_overwritten() {
    // VBA は前回の結果が入ったままの Variant を渡してくる
    let mut out = Out::new();
    out.0 = VARIANT::bstr("前回の結果");
    assert_eq!(unsafe { HatoPo_Version(out.p()) }, HP_OK);
    assert_eq!(out.text(), env!("CARGO_PKG_VERSION"));
}
