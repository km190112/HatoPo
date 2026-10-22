//! OLE Automation (`oleaut32.dll`) の最小限の手書きバインディング。
//!
//! duckdb-vba-dll と同じ理由で `windows` クレートを使わない:
//! VBA が所有するメモリに VARIANT を書くので、Drop を持たない `#[repr(C)]` の POD が欲しい。
//! 対象は x64 のみ (`sizeof(VARIANT) == 24`)。

#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms)]

use std::os::raw::c_void;

pub const VT_EMPTY: u16 = 0;
pub const VT_I4: u16 = 3;
pub const VT_DATE: u16 = 7;
pub const VT_BSTR: u16 = 8;
pub const VT_VARIANT: u16 = 12;
pub const VT_ARRAY: u16 = 0x2000;

#[repr(C)]
pub struct SAFEARRAY {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SAFEARRAYBOUND {
    pub cElements: u32,
    pub lLbound: i32,
}

/// x64 では 16 バイト。小さいメンバだけ書くと上位に残りが出るので、必ず全体をゼロにしてから書く。
#[repr(C)]
#[derive(Clone, Copy)]
pub union VARIANT_VALUE {
    pub lVal: i32,
    pub date: f64,
    pub bstrVal: *mut u16,
    pub parray: *mut SAFEARRAY,
    _pad: [u8; 16],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VARIANT {
    pub vt: u16,
    pub wReserved1: u16,
    pub wReserved2: u16,
    pub wReserved3: u16,
    pub value: VARIANT_VALUE,
}

const ZERO: VARIANT_VALUE = VARIANT_VALUE { _pad: [0u8; 16] };

impl VARIANT {
    fn of(vt: u16, value: VARIANT_VALUE) -> Self {
        VARIANT {
            vt,
            wReserved1: 0,
            wReserved2: 0,
            wReserved3: 0,
            value,
        }
    }

    pub fn empty() -> Self {
        Self::of(VT_EMPTY, ZERO)
    }

    pub fn i32(v: i32) -> Self {
        let mut u = ZERO;
        u.lVal = v;
        Self::of(VT_I4, u)
    }

    /// OLE の日付 (1899-12-30 からの日数。時刻は小数部)。
    pub fn date(v: f64) -> Self {
        let mut u = ZERO;
        u.date = v;
        Self::of(VT_DATE, u)
    }

    pub fn bstr(s: &str) -> Self {
        let wide: Vec<u16> = s.encode_utf16().collect();
        let b = unsafe { SysAllocStringLen(wide.as_ptr(), wide.len() as u32) };
        if b.is_null() {
            return Self::empty();
        }
        let mut u = ZERO;
        u.bstrVal = b;
        Self::of(VT_BSTR, u)
    }

    /// 0 始まりの 2 次元 Variant 配列 `(row, col)`。`cells` は行優先で `rows * cols` 個。
    /// duckdb-vba-dll で実証済みのやり方 (AccessData で列優先の位置へ直接 move) に倣う。
    pub fn grid(rows: usize, cols: usize, mut cells: Vec<VARIANT>) -> Option<Self> {
        debug_assert_eq!(cells.len(), rows * cols);
        // rgsabound[0] が VBA の最初の添字 (行)
        let bounds = [
            SAFEARRAYBOUND {
                cElements: rows as u32,
                lLbound: 0,
            },
            SAFEARRAYBOUND {
                cElements: cols as u32,
                lLbound: 0,
            },
        ];
        let psa = unsafe { SafeArrayCreate(VT_VARIANT, 2, bounds.as_ptr()) };
        let mut data: *mut c_void = std::ptr::null_mut();
        if psa.is_null() || unsafe { SafeArrayAccessData(psa, &mut data) } != 0 || data.is_null() {
            if !psa.is_null() {
                unsafe { SafeArrayDestroy(psa) };
            }
            for v in cells.iter_mut() {
                unsafe { VariantClear(v) };
            }
            return None;
        }
        // SAFEARRAY は列優先。(r, c) の位置は c * rows + r。作りたてはゼロ埋めなので move でよい
        let dst = data as *mut VARIANT;
        for (i, v) in cells.into_iter().enumerate() {
            let (r, c) = (i / cols, i % cols);
            unsafe { dst.add(c * rows + r).write(v) };
        }
        unsafe { SafeArrayUnaccessData(psa) };
        let mut u = ZERO;
        u.parray = psa;
        Some(Self::of(VT_ARRAY | VT_VARIANT, u))
    }
}

/// VBA の `ByRef result As Variant` に書く。前の中身は解放する。
///
/// # Safety
/// `out` は有効な VARIANT* か null。
pub unsafe fn write_out(out: *mut VARIANT, v: VARIANT) {
    if out.is_null() {
        let mut v = v;
        unsafe { VariantClear(&mut v) };
        return;
    }
    unsafe {
        VariantClear(out);
        out.write(v);
    }
}

/// VBA が `StrPtr(s)` で渡した BSTR を読む。null (`vbNullString`) は空文字。
///
/// # Safety
/// `p` は有効な BSTR か null。
pub unsafe fn bstr_arg(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let len = unsafe { SysStringLen(p as *mut u16) } as usize;
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })
}

#[link(name = "oleaut32")]
unsafe extern "system" {
    pub fn SysAllocStringLen(psz: *const u16, len: u32) -> *mut u16;
    pub fn SysStringLen(bstr: *mut u16) -> u32;
    pub fn VariantClear(pvarg: *mut VARIANT) -> i32;
    pub fn SafeArrayCreate(vt: u16, cDims: u32, rgsabound: *const SAFEARRAYBOUND)
    -> *mut SAFEARRAY;
    pub fn SafeArrayDestroy(psa: *mut SAFEARRAY) -> i32;
    pub fn SafeArrayAccessData(psa: *mut SAFEARRAY, ppv: *mut *mut c_void) -> i32;
    pub fn SafeArrayUnaccessData(psa: *mut SAFEARRAY) -> i32;
    // 以下 2 つはテストで結果の配列を読むためだけに使う
    #[cfg(test)]
    pub fn SafeArrayGetElement(psa: *mut SAFEARRAY, idx: *const i32, pv: *mut c_void) -> i32;
    #[cfg(test)]
    pub fn SafeArrayGetUBound(psa: *mut SAFEARRAY, nDim: u32, plUbound: *mut i32) -> i32;
}
