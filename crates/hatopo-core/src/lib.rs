//! HatoPo のロジック本体。共有フォルダ (鳩の巣 = Loft) にある通知ファイルの読み書きをする。
//!
//! - 通知 1 件 = JSON ファイル 1 つ。置き場所で宛先が決まる (broadcast / device / user)
//! - 書き込みは同じフォルダの一時ファイルに書いてから rename する (読み手は書きかけを見ない)
//! - 変更・退避は `{id}.lock` を排他作成してから行い、`revision` で同時更新を検出する
//!
//! exe・DLL・CLI はこのクレートを呼ぶだけの殻にする。

mod directory;
mod error;
mod id;
mod loft;
mod model;
mod read;
mod settings;
mod status;
mod when;

pub use directory::{DIRECTORY_FILE, Directory, Member};
pub use error::Error;
pub use loft::{Broken, Filter, Inbox, Loft};
pub use model::{
    Archived, CloseReason, Closed, Kind, NewNotice, Notice, NoticeEdit, Origin, ReadMark, Target,
};
pub use settings::{
    MASCOT_SCALES, MAX_POLL_SECONDS, MAX_REMIND_MINUTES, MIN_POLL_SECONDS, Settings,
};
pub use status::{ClientStatus, read_all_status, read_status, touch};
pub use when::{DayEdge, parse_date, parse_when};

pub type Result<T> = std::result::Result<T, Error>;
