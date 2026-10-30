//! 起動の段取り: 導入 (共有から初回起動) / 自己更新。
//!
//! - 共有の `HatoPo.exe` は直接常駐させない。実行中の exe は差し替えられず、共有が切れると常駐ごと落ちる
//! - 初回は共有の exe をダブルクリック → `%LOCALAPPDATA%\HatoPo\` へ自分をコピー → そちらを起動
//! - 以後はローカルの exe が動き、共有の exe が変わったら自分を入れ替えて起動し直す
//! - 自動起動 (ログオン時に起動) の**登録**は exe からは行わない。exe 自身が `schtasks /create` を
//!   呼んで永続化 (persistence) を仕込む挙動は Microsoft Defender に誤検知されるため、配布物同梱の
//!   `loft-template/autostart/自動起動を登録.bat` (`Register-ScheduledTask`) に任せる (2026-10-30)。
//!   **解除** (`schtasks /delete`) は persistence を作らない操作なので `--uninstall` 用に exe 側に残す

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const EXE_NAME: &str = "HatoPo.exe";
pub const TASK_NAME: &str = "HatoPo";

/// `%LOCALAPPDATA%\HatoPo`。
pub fn local_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("HatoPo")
}

/// 鳩の巣の場所を覚えておくファイル。
fn loft_file() -> PathBuf {
    local_dir().join("loft.txt")
}

pub fn saved_loft() -> Option<PathBuf> {
    let s = fs::read_to_string(loft_file()).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| PathBuf::from(s))
}

/// フォルダが鳩の巣らしいか (共有から起動されたのかの判定に使う)。
pub fn looks_like_loft(dir: &Path) -> bool {
    ["settings.json", "broadcast", "PigeonHole"]
        .iter()
        .any(|n| dir.join(n).exists())
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        fs::canonicalize(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .to_lowercase()
    };
    norm(a) == norm(b)
}

/// 共有の exe の「版」。大きさと更新日時で見分ける (中身のハッシュは遅い共有では高い)。
pub fn stamp(exe: &Path) -> Option<String> {
    let m = fs::metadata(exe).ok()?;
    let t = m
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(format!("{}:{t}", m.len()))
}

fn stamp_file() -> PathBuf {
    local_dir().join("source-stamp.txt")
}

pub fn saved_stamp() -> Option<String> {
    fs::read_to_string(stamp_file())
        .ok()
        .map(|s| s.trim().to_string())
}

/// 実行中でも exe を置き換える。Windows は実行中の exe を消せないが、名前は変えられる。
/// 古い方は `HatoPo.old.exe` に退け、次の起動で消す。
pub fn replace_exe(src: &Path, dest: &Path) -> std::io::Result<()> {
    let old = dest.with_file_name("HatoPo.old.exe");
    let _ = fs::remove_file(&old);
    if dest.exists() && fs::remove_file(dest).is_err() {
        fs::rename(dest, &old)?;
    }
    // 途中で切れても壊れた exe が残らないよう、一時名でコピーしてから名前を付ける
    let tmp = dest.with_file_name("HatoPo.new.exe");
    fs::copy(src, &tmp)?;
    fs::rename(&tmp, dest)
}

pub fn cleanup_old() {
    let _ = fs::remove_file(local_dir().join("HatoPo.old.exe"));
}

/// 共有の exe をローカルへ入れ、鳩の巣の場所を覚える。戻り値はローカルの exe。
/// 自動起動の登録はここでは行わない (同梱の bat に任せる)。
pub fn install(loft: &Path, running_exe: &Path) -> Result<PathBuf, String> {
    let dir = local_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("{} を作れません: {e}", dir.display()))?;
    let dest = dir.join(EXE_NAME);
    replace_exe(running_exe, &dest).map_err(|e| format!("HatoPo をコピーできません: {e}"))?;
    fs::write(loft_file(), loft.display().to_string())
        .map_err(|e| format!("設定を書けません: {e}"))?;
    if let Some(s) = stamp(running_exe) {
        let _ = fs::write(stamp_file(), s);
    }
    Ok(dest)
}

/// 共有に新しい exe があればローカルへ入れ替える。入れ替えたら `true` (起動し直すこと)。
pub fn update_if_changed(loft: &Path, local_exe: &Path) -> Result<bool, String> {
    let src = loft.join(EXE_NAME);
    let Some(now) = stamp(&src) else {
        return Ok(false); // 共有に exe が無い・読めない (オフラインなど)
    };
    if saved_stamp().as_deref() == Some(now.as_str()) {
        return Ok(false);
    }
    replace_exe(&src, local_exe).map_err(|e| format!("更新をコピーできません: {e}"))?;
    let _ = fs::write(stamp_file(), now);
    Ok(true)
}

/// 新しいプロセスを起動する。`wait_pid` を渡すと、新しい方はこのプロセスの終了を待ってから立ち上がる。
pub fn spawn(exe: &Path, wait_pid: Option<u32>) -> Result<(), String> {
    let mut c = Command::new(exe);
    if let Some(p) = wait_pid {
        c.arg("--wait-pid").arg(p.to_string());
    }
    c.spawn()
        .map(|_| ())
        .map_err(|e| format!("{} を起動できません: {e}", exe.display()))
}

// ---------------------------------------------------------------------------
// 自動起動 (タスクスケジューラ) — ここに残るのは「読む」と「消す」だけ
//
// 登録 (`schtasks /create`。persistence を新規に作る操作) は配布 zip 同梱の
// loft-template/autostart/自動起動を登録.bat (Register-ScheduledTask) が行う。
// exe 自身が schtasks /create を呼ぶと Microsoft Defender の誤検知を招くため、2026-10 に
// exe からこの機能を外した。current_account / task_full_name はタスク名を bat 側と揃えるため、
// is_autostart_registered はトレイの読み取り専用表示のため、unregister_autostart は
// `schtasks /delete` (persistence を作らない) なので `--uninstall` 用に、それぞれ残す。
// ---------------------------------------------------------------------------

/// `DOMAIN\USER` (ドメイン非参加なら `PC名\USER`)。
pub fn current_account() -> Option<(String, String)> {
    let user = std::env::var("USERNAME").ok()?.trim().to_string();
    if user.is_empty() {
        return None;
    }
    let domain = std::env::var("USERDOMAIN").unwrap_or_default();
    let domain = domain.trim();
    let account = if domain.is_empty() {
        user.clone()
    } else {
        format!("{domain}\\{user}")
    };
    Some((account, user))
}

/// `<ログオン名>\HatoPo`。1 台を複数人で使っても互いの登録を消さない。
pub fn task_full_name(user: &str) -> String {
    let folder: String = user
        .chars()
        .filter(|c| !matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect();
    let folder = folder.trim();
    if folder.is_empty() {
        TASK_NAME.into()
    } else {
        format!("{folder}\\{TASK_NAME}")
    }
}

fn schtasks(args: &[&str]) -> Result<std::process::Output, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Command::new("schtasks")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("schtasks を起動できません: {e}"))
}

/// 自分の自動起動 (`<ログオン名>\HatoPo`) が登録されているか。
pub fn is_autostart_registered() -> bool {
    let Some((_, user)) = current_account() else {
        return false;
    };
    let full = task_full_name(&user);
    schtasks(&["/query", "/tn", &full]).is_ok_and(|o| o.status.success())
}

pub fn unregister_autostart() -> Result<(), String> {
    let (_, user) = current_account().ok_or("ログオンユーザー名が取れません")?;
    let full = task_full_name(&user);
    // 無ければ失敗するが、それは「登録されていない」なので成功扱い
    let q = schtasks(&["/query", "/tn", &full])?;
    if !q.status.success() {
        return Ok(());
    }
    let out = schtasks(&["/delete", "/tn", &full, "/f"])?;
    if out.status.success() {
        Ok(())
    } else {
        Err("自動起動の登録を解除できませんでした".into())
    }
}

/// `%LOCALAPPDATA%\HatoPo` を数秒後に削除する。実行中の exe は自分自身のファイルを
/// 削除できないため、自分が終了してロックが外れるのを別プロセスで待たせる
/// (Windows の自己アンインストールの定番手法)。失敗しても `--uninstall` 自体は続行させたいので
/// 戻り値は無く、プロセスの起動に失敗しても黙って諦める。
pub fn schedule_delete_local_dir() {
    schedule_delete_dir(&local_dir());
}

fn schedule_delete_dir(dir: &Path) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let cmd = format!(
        "ping 127.0.0.1 -n 3 >nul & rmdir /s /q \"{}\"",
        dir.display()
    );
    let _ = Command::new("cmd")
        .raw_arg(format!("/C {cmd}"))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_lives_under_user_folder() {
        assert_eq!(task_full_name("taro"), r"taro\HatoPo");
        assert_eq!(task_full_name("山田"), r"山田\HatoPo");
        assert_eq!(task_full_name(r"a\b"), r"ab\HatoPo");
        assert_eq!(task_full_name(""), "HatoPo");
    }

    #[test]
    fn replace_exe_works_even_if_target_is_locked_by_rename_trick() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src.exe");
        let dest = d.path().join("HatoPo.exe");
        fs::write(&src, b"v2").unwrap();
        fs::write(&dest, b"v1").unwrap();
        replace_exe(&src, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"v2");
        assert!(!d.path().join("HatoPo.new.exe").exists());
    }

    #[test]
    fn schedule_delete_dir_removes_the_folder_after_a_short_delay() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("日本語フォルダ").join("HatoPo");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("dummy.txt"), b"x").unwrap();
        schedule_delete_dir(&target);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while target.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        assert!(!target.exists(), "数秒待っても削除されなかった");
    }

    #[test]
    fn loft_is_detected_by_its_contents() {
        let d = tempfile::tempdir().unwrap();
        assert!(!looks_like_loft(d.path()));
        fs::create_dir(d.path().join("PigeonHole")).unwrap();
        assert!(looks_like_loft(d.path()));
    }
}
