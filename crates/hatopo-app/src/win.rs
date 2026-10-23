//! Win32 の小さな呼び出しをまとめる。ここ以外で unsafe を書かない。

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, LPARAM, RECT,
};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MessageBoxW, SW_SHOWNORMAL,
};
use windows_sys::core::BOOL;

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

/// 同じログオンセッションで 2 つ目が起動しないようにする名前付きミューテックス。
/// 戻り値を持っている間だけ有効。既に他が持っていれば `None`。
pub struct SingleInstance(HANDLE);

impl SingleInstance {
    pub fn acquire(name: &str) -> Option<SingleInstance> {
        let n = wide(format!("Local\\{name}"));
        let h = unsafe { CreateMutexW(std::ptr::null(), 1, n.as_ptr()) };
        if h.is_null() {
            return None;
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(h) };
            return None;
        }
        Some(SingleInstance(h))
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// 前のプロセスが終わるのを待つ (自己更新で起動し直したとき)。
pub fn wait_for_pid(pid: u32, timeout_ms: u32) {
    let h = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if h.is_null() {
        return; // もう居ない
    }
    unsafe {
        WaitForSingleObject(h, timeout_ms);
        CloseHandle(h);
    }
}

/// ファイル・フォルダ・URL を既定のアプリで開く (エクスプローラーでダブルクリックしたのと同じ)。
pub fn shell_open(target: &str) -> Result<(), String> {
    let op = wide("open");
    let t = wide(target);
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            t.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // 32 以下は失敗 (ShellExecute の仕様)
    if (r as isize) > 32 {
        Ok(())
    } else {
        Err(format!(
            "「{target}」を開けませんでした (コード {})。場所が移動していないか確認してください。",
            r as isize
        ))
    }
}

pub fn message_box(text: &str, error: bool) {
    let t = wide(text);
    let c = wide("HatoPo");
    let icon = if error {
        MB_ICONERROR
    } else {
        MB_ICONINFORMATION
    };
    unsafe { MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), MB_OK | icon) };
}

/// トーストの差出人名とアイコンを登録する (HKCU なので管理者権限は要らない)。
/// これが無いと Windows はトーストを出さないか、差出人を空にする。
pub fn register_aumid(aumid: &str, display_name: &str, icon_png: &Path) -> Result<(), String> {
    let key_path = wide(format!("Software\\Classes\\AppUserModelId\\{aumid}"));
    let mut key: HKEY = std::ptr::null_mut();
    let rc = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key_path.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if rc != 0 {
        return Err(format!("レジストリを開けません (コード {rc})"));
    }
    let set = |name: &str, value: &str| {
        let n = wide(name);
        let v = wide(value);
        unsafe {
            RegSetValueExW(
                key,
                n.as_ptr(),
                0,
                REG_SZ,
                v.as_ptr() as *const u8,
                (v.len() * 2) as u32,
            )
        }
    };
    let r1 = set("DisplayName", display_name);
    let r2 = set("IconUri", &icon_png.display().to_string());
    unsafe { RegCloseKey(key) };
    if r1 != 0 || r2 != 0 {
        return Err(format!("レジストリに書けません (コード {r1}/{r2})"));
    }
    Ok(())
}

pub fn unregister_aumid(aumid: &str) {
    let key_path = wide(format!("Software\\Classes\\AppUserModelId\\{aumid}"));
    unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, key_path.as_ptr()) };
}

/// `MONITORINFO::dwFlags` のメイン画面の印
const MONITORINFOF_PRIMARY: u32 = 1;

/// 1 枚のモニター。座標は仮想デスクトップ上の実ピクセル (左上が x, y)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Monitor {
    /// モニター全体 (left, top, right, bottom)
    pub rect: [i32; 4],
    /// タスクバーを除いた作業領域
    pub work: [i32; 4],
    pub primary: bool,
}

unsafe extern "system" fn collect_monitor(
    h: HMONITOR,
    _dc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let out = unsafe { &mut *(data as *mut Vec<Monitor>) };
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(h, &mut info) } != 0 {
        let r = |r: RECT| [r.left, r.top, r.right, r.bottom];
        out.push(Monitor {
            rect: r(info.rcMonitor),
            work: r(info.rcWork),
            primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
    }
    1
}

/// 今つながっているモニターの一覧 (取れなければ空)。ケーブルを抜き挿しすると変わる。
pub fn monitors() -> Vec<Monitor> {
    let mut v: Vec<Monitor> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(collect_monitor),
            &mut v as *mut Vec<Monitor> as LPARAM,
        );
    }
    v
}
