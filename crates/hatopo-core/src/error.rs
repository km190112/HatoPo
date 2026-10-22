use std::fmt;
use std::path::PathBuf;

/// 呼び出し側 (VBA の利用者を含む) にそのまま見せるので、メッセージは日本語で書く。
#[derive(Debug)]
pub enum Error {
    /// 共有フォルダそのものに届かない (ネットワーク断・パス違い)。
    Offline {
        root: PathBuf,
        detail: String,
    },
    NotFound {
        id: String,
    },
    /// 読み込んだ後に他の人が先に変更した。
    Conflict {
        id: String,
        expected: u64,
        actual: u64,
    },
    /// 他の人が変更中。少し待てば通る。
    Locked {
        id: String,
    },
    /// 入力の誤り (宛先名・日付など)。
    Invalid(String),
    /// 通知ファイルが JSON として読めない。
    Broken {
        path: PathBuf,
        detail: String,
    },
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Offline { root, detail } => write!(
                f,
                "鳩の巣 ({}) に接続できません。ネットワークとパスを確認してください。({detail})",
                root.display()
            ),
            Error::NotFound { id } => write!(
                f,
                "通知 {id} が見つかりません。既に完了・削除された可能性があります。"
            ),
            Error::Conflict {
                id,
                expected,
                actual,
            } => write!(
                f,
                "通知 {id} は他の人が先に変更しました (読んだ版 {expected}、今の版 {actual})。読み直してからやり直してください。"
            ),
            Error::Locked { id } => write!(
                f,
                "通知 {id} は他の人が変更中です。数秒後にやり直してください。"
            ),
            Error::Invalid(msg) => write!(f, "{msg}"),
            Error::Broken { path, detail } => {
                write!(f, "通知ファイル {} が読めません: {detail}", path.display())
            }
            Error::Io { path, source } => {
                write!(f, "{} の読み書きに失敗しました: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for Error {}

pub(crate) fn io(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Error {
    let path = path.into();
    move |source| Error::Io { path, source }
}
