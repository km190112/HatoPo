//! hatopo-cli — 管理者が bat / PowerShell / タスクスケジューラから通知を出し入れする口。
//!
//! 鳩の巣の場所は `--root`、無ければ環境変数 `HATOPO_ROOT`、それも無ければこの exe のあるフォルダ。

use chrono::{Local, NaiveDateTime};
use clap::{Args, Parser, Subcommand};
use hatopo_core::{
    CloseReason, DayEdge, Error, Filter, Kind, Loft, NewNotice, Notice, Origin, Target, parse_date,
    parse_when,
};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "hatopo-cli",
    version,
    about = "HatoPo の通知を作る・探す・変える・消す"
)]
struct Cli {
    /// 鳩の巣のフォルダ (例: \\server\HatoLoft)
    #[arg(long, global = true)]
    root: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 通知を作る。作った ID を出力する
    Add {
        /// 宛先。user:taro / device:PC-0123 / broadcast。複数指定すると人数分作る
        #[arg(long = "to", required = true)]
        to: Vec<String>,
        #[command(flatten)]
        fields: Fields,
    },
    /// 通知を書き換える (宛先は変えられない)
    Update {
        id: String,
        /// 読んだときの版。他の人が先に変えていたら失敗する。0 は確かめない
        #[arg(long)]
        revision: u64,
        #[command(flatten)]
        fields: Fields,
    },
    /// 通知を消す (archive へ移す)
    Delete { id: String },
    /// 通知を 1 件 JSON で出す
    Get { id: String },
    /// 通知を探す
    List {
        /// broadcast / device / user
        #[arg(long)]
        kind: Option<String>,
        /// ユーザー名かコンピューター名
        #[arg(long)]
        name: Option<String>,
        /// 件名・本文に含まれる文字
        #[arg(long)]
        keyword: Option<String>,
        /// JSON で出す
        #[arg(long)]
        json: bool,
    },
    /// archive に移った通知 (誰がいつ完了したか) を出す
    Archive {
        /// 2026-10 の形。省略すると今月
        month: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// 鳩の巣のフォルダ一式と settings.json を作る (既にあるものは触らない)
    Init,
}

#[derive(Args)]
struct Fields {
    /// 通知内容 (件名)
    #[arg(long)]
    title: String,
    /// 詳しい説明
    #[arg(long, default_value = "")]
    body: String,
    /// 依頼期限 (2026-11-05)
    #[arg(long, default_value = "")]
    due: String,
    /// リンク (ファイル・フォルダ・URL)。3 つまで
    #[arg(long = "url")]
    urls: Vec<String>,
    /// 通知開始日時。省略すると今すぐ
    #[arg(long, default_value = "")]
    start: String,
    /// 通知終了日時。日付だけならその日いっぱい。省略すると完了まで
    #[arg(long, default_value = "")]
    end: String,
}

impl Fields {
    fn to_notice(&self) -> Result<NewNotice, Error> {
        Ok(NewNotice {
            title: self.title.clone(),
            body: self.body.replace("\\n", "\n"),
            due: parse_date(&self.due)?,
            urls: self.urls.clone(),
            start_at: parse_when(&self.start, DayEdge::Start)?,
            end_at: parse_when(&self.end, DayEdge::End)?,
        })
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let root = cli
        .root
        .or_else(|| std::env::var_os("HATOPO_ROOT").map(PathBuf::from))
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(Into::into))
        })
        .unwrap_or_else(|| PathBuf::from("."));
    match run(&Loft::new(root), cli.cmd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("エラー: {e}");
            ExitCode::from(exit_code(&e))
        }
    }
}

/// バッチから分岐しやすいように種類ごとに番号を分ける。
fn exit_code(e: &Error) -> u8 {
    match e {
        Error::Invalid(_) => 2,
        Error::NotFound { .. } => 3,
        Error::Conflict { .. } => 4,
        Error::Locked { .. } => 5,
        Error::Offline { .. } => 6,
        _ => 1,
    }
}

fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

fn run(loft: &Loft, cmd: Cmd) -> Result<(), Error> {
    let me = Origin::current();
    match cmd {
        Cmd::Add { to, fields } => {
            let n = fields.to_notice()?;
            // 宛先を全部読めてから作る (途中で失敗して一部だけ作られるのを避ける)
            let targets = to
                .iter()
                .map(|t| Target::parse(t))
                .collect::<Result<Vec<_>, _>>()?;
            for t in targets {
                let made = loft.add(&t, &n, &me, now())?;
                println!("{}\t{}", made.id, made.target);
            }
        }
        Cmd::Update {
            id,
            revision,
            fields,
        } => {
            let n = loft.update(&id, revision, &fields.to_notice()?, &me, now())?;
            println!("{}\trevision {}", n.id, n.revision);
        }
        Cmd::Delete { id } => {
            loft.close(&id, CloseReason::Deleted, &me, now())?;
            println!("{id}\tarchive へ移しました");
        }
        Cmd::Get { id } => println!("{}", json(&loft.get(&id)?)),
        Cmd::List {
            kind,
            name,
            keyword,
            json: as_json,
        } => {
            let filter = Filter {
                kind: kind.as_deref().map(Kind::parse).transpose()?,
                name,
                keyword,
            };
            let (found, broken) = loft.find(&filter)?;
            if as_json {
                println!("{}", json(&found));
            } else {
                print_table(&found);
            }
            for b in broken {
                eprintln!("読めないファイル: {} ({})", b.path.display(), b.detail);
            }
        }
        Cmd::Archive {
            month,
            json: as_json,
        } => {
            let month = month.unwrap_or_else(|| now().format("%Y-%m").to_string());
            let list = loft.archived(&month)?;
            if as_json {
                println!("{}", json(&list));
            } else {
                for a in list {
                    println!(
                        "{}\t{:?}\t{}@{}\t{}\t{}\t{}",
                        a.notice.id,
                        a.closed.reason,
                        a.closed.by.user,
                        a.closed.by.host,
                        a.closed.at.format("%Y-%m-%d %H:%M"),
                        a.notice.target,
                        a.notice.title
                    );
                }
            }
        }
        Cmd::Init => init(loft)?,
    }
    Ok(())
}

fn json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

fn print_table(list: &[Notice]) {
    println!("ID\t版\t宛先\t開始\t終了\t件名");
    for n in list {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            n.id,
            n.revision,
            n.target,
            n.start_at.format("%Y-%m-%d %H:%M"),
            n.end_at
                .map(|e| e.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "-".into()),
            n.title
        );
    }
}

fn init(loft: &Loft) -> Result<(), Error> {
    let root = loft.root();
    for d in [
        "broadcast",
        "PigeonHole/user",
        "PigeonHole/device",
        "archive",
        "status",
        "character_icon",
    ] {
        let p = root.join(d);
        std::fs::create_dir_all(&p).map_err(|e| Error::Io { path: p, source: e })?;
    }
    let settings = root.join("settings.json");
    if !settings.exists() {
        std::fs::write(
            &settings,
            include_str!("../../../loft-template/settings.json"),
        )
        .map_err(|e| Error::Io {
            path: settings.clone(),
            source: e,
        })?;
    }
    println!("{} に鳩の巣を用意しました", root.display());
    Ok(())
}
