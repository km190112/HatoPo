//! 複数の端末が同時に同じ通知を触ったとき、ファイルが壊れず、勝者が 1 人だけになること。
//! 共有フォルダの代わりにローカルの一時フォルダで、スレッドを端末に見立てる。

use chrono::NaiveDate;
use hatopo_core::{CloseReason, Error, Kind, Loft, NewNotice, Origin, Target};
use std::sync::{Arc, Barrier};

fn now() -> chrono::NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 11, 1)
        .unwrap()
        .and_hms_opt(9, 0, 0)
        .unwrap()
}

fn who(i: usize) -> Origin {
    Origin {
        user: format!("user{i}"),
        host: format!("pc-{i}"),
    }
}

#[test]
fn concurrent_updates_on_same_revision_have_at_most_one_winner() {
    let dir = tempfile::tempdir().unwrap();
    let loft = Arc::new(Loft::new(dir.path()));
    let target = Target::new(Kind::User, "taro").unwrap();
    let base = NewNotice {
        title: "v1".into(),
        ..Default::default()
    };
    let made = loft.add(&target, &base, &who(0), now()).unwrap();

    let n = 16;
    let barrier = Arc::new(Barrier::new(n));
    let handles: Vec<_> = (0..n)
        .map(|i| {
            let (loft, barrier, id) = (loft.clone(), barrier.clone(), made.id.clone());
            std::thread::spawn(move || {
                barrier.wait();
                let edit = NewNotice {
                    title: format!("by {i}"),
                    ..Default::default()
                };
                loft.update(&id, 1, &edit, &who(i), now())
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    let winners = results.iter().filter(|r| r.is_ok()).count();
    assert!(winners <= 1, "版 1 から書けるのは 1 人まで: {winners}");
    for r in &results {
        match r {
            Ok(_) | Err(Error::Conflict { .. }) | Err(Error::Locked { .. }) => {}
            Err(e) => panic!("想定外のエラー: {e}"),
        }
    }
    // ファイルは常に読める (壊れていない)
    let now_on_disk = loft.get(&made.id).unwrap();
    assert_eq!(now_on_disk.revision, 1 + winners as u64);
}

#[test]
fn concurrent_complete_from_many_pcs_archives_exactly_once() {
    let dir = tempfile::tempdir().unwrap();
    let loft = Arc::new(Loft::new(dir.path()));
    let target = Target::new(Kind::Device, "pc-shared").unwrap();
    let made = loft
        .add(
            &target,
            &NewNotice {
                title: "更新してください".into(),
                ..Default::default()
            },
            &who(0),
            now(),
        )
        .unwrap();

    let n = 12;
    let barrier = Arc::new(Barrier::new(n));
    let handles: Vec<_> = (0..n)
        .map(|i| {
            let (loft, barrier, id) = (loft.clone(), barrier.clone(), made.id.clone());
            std::thread::spawn(move || {
                barrier.wait();
                loft.close(&id, CloseReason::Done, &who(i), now())
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    let archived = loft.archived("2026-11").unwrap();
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].notice.title, "更新してください");
    assert!(
        loft.inbox("x", "pc-shared", now())
            .unwrap()
            .active
            .is_empty()
    );

    // 一時ファイル・ロックファイルが残っていない
    let leftovers: Vec<_> = walk(dir.path())
        .into_iter()
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy();
            n.ends_with(".tmp") || n.ends_with(".lock")
        })
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

fn walk(p: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(p).unwrap().flatten() {
        let path = e.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}
