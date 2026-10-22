//! 起動の段取り: 導入 (共有から初回起動) / 自己更新 / 自動起動の登録。
//!
//! - 共有の `HatoPo.exe` は直接常駐させない。実行中の exe は差し替えられず、共有が切れると常駐ごと落ちる
//! - 初回は共有の exe をダブルクリック → `%LOCALAPPDATA%\HatoPo\` へ自分をコピー → 自動起動を登録 → そちらを起動
//! - 以後はローカルの exe が動き、共有の exe が変わったら自分を入れ替えて起動し直す

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

/// 共有の exe をローカルへ入れ、鳩の巣の場所を覚え、自動起動を登録する。戻り値はローカルの exe。
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
    register_autostart(&dest)?;
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
// 自動起動 (タスクスケジューラ)
//
// `schtasks /sc onlogon` は管理者権限が要るので使わない。XML でトリガーの UserId を
// 自分に限定し、LogonType=InteractiveToken にすると昇格せずに登録できる。
// 既定値の地雷 (ExecutionTimeLimit=PT72H で 3 日後に止まる、電池駆動で止まる) も XML で潰す。
// ---------------------------------------------------------------------------

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

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

/// 登録用の XML。`<Command>` は引用符で囲まない (囲むと引用符ごとファイル名になる)。
/// エスケープはパス操作を全部終えてから最後に 1 回だけ掛ける。
pub fn task_xml(exe: &Path, account: &str) -> String {
    let dir = exe
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let exe = xml_escape(&exe.display().to_string());
    let dir = xml_escape(&dir);
    let account = xml_escape(account);
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>HatoPo (デスクトップ常駐の通知) をログオン時に起動する</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{account}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{account}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>3</Count>
    </RestartOnFailure>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <WorkingDirectory>{dir}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#
    )
}

/// schtasks は BOM で文字コードを判定する。UTF-8 で書くと日本語のユーザー名やパスが化ける。
fn utf16le_bom(s: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    for u in s.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
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

pub fn register_autostart(exe: &Path) -> Result<(), String> {
    let (account, user) = current_account().ok_or("ログオンユーザー名が取れません")?;
    let full = task_full_name(&user);
    let xml_path = local_dir().join("autostart-task.xml");
    fs::write(&xml_path, utf16le_bom(&task_xml(exe, &account)))
        .map_err(|e| format!("自動起動の設定を書けません: {e}"))?;
    let xml_arg = xml_path.display().to_string();
    let out = schtasks(&["/create", "/xml", &xml_arg, "/tn", &full, "/f"])?;
    if out.status.success() {
        let _ = fs::remove_file(&xml_path);
        Ok(())
    } else {
        // 失敗したら XML は残す (案内したコマンドをそのまま実行できるように)
        Err(format!(
            "自動起動を登録できませんでした。次のコマンドを実行してください:\nschtasks /create /xml \"{xml_arg}\" /tn \"{full}\" /f"
        ))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_xml_escapes_once_and_does_not_quote_command() {
        let exe = Path::new(r"C:\Users\R&D 太郎\AppData\Local\HatoPo\HatoPo.exe");
        let xml = task_xml(exe, r"CORP\r&d");
        assert!(
            xml.contains(
                r"<Command>C:\Users\R&amp;D 太郎\AppData\Local\HatoPo\HatoPo.exe</Command>"
            )
        );
        assert!(xml.contains(
            r"<WorkingDirectory>C:\Users\R&amp;D 太郎\AppData\Local\HatoPo</WorkingDirectory>"
        ));
        assert!(xml.contains(r"<UserId>CORP\r&amp;d</UserId>"));
        assert!(!xml.contains("&amp;amp;"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(xml.contains("<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>"));
        assert!(xml.contains("<LogonType>InteractiveToken</LogonType>"));
    }

    #[test]
    fn task_lives_under_user_folder() {
        assert_eq!(task_full_name("taro"), r"taro\HatoPo");
        assert_eq!(task_full_name("山田"), r"山田\HatoPo");
        assert_eq!(task_full_name(r"a\b"), r"ab\HatoPo");
        assert_eq!(task_full_name(""), "HatoPo");
    }

    #[test]
    fn xml_is_written_as_utf16le_with_bom() {
        let b = utf16le_bom("あ");
        assert_eq!(b, [0xFF, 0xFE, 0x42, 0x30]);
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
    fn loft_is_detected_by_its_contents() {
        let d = tempfile::tempdir().unwrap();
        assert!(!looks_like_loft(d.path()));
        fs::create_dir(d.path().join("PigeonHole")).unwrap();
        assert!(looks_like_loft(d.path()));
    }
}
