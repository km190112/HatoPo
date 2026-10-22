//! 日付・日時の文字列を解釈する。VBA の `Format` や Date 型を文字列にしたもの
//! (`2026/11/5 9:00:00`) も、ISO 形式 (`2026-11-05T09:00`) も受け付ける。

use crate::{Error, Result};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};

/// 日付だけが渡されたとき、その日のどの時刻とみなすか。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DayEdge {
    /// 00:00:00 (通知開始日)
    Start,
    /// 23:59:59 (通知終了日。その日いっぱいは出す)
    End,
}

/// 空文字は `None`。読めない文字列はエラー。
pub fn parse_when(s: &str, edge: DayEdge) -> Result<Option<NaiveDateTime>> {
    let t = s.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let norm = t.replace('/', "-").replace('T', " ");
    let mut parts = norm.split_whitespace();
    let date = parts.next().unwrap_or_default();
    let time = parts.next();
    if parts.next().is_some() {
        return Err(bad(t));
    }
    let d = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| bad(t))?;
    let tm = match time {
        None => match edge {
            DayEdge::Start => NaiveTime::MIN,
            DayEdge::End => NaiveTime::from_hms_opt(23, 59, 59).unwrap(),
        },
        Some(x) => NaiveTime::parse_from_str(x, "%H:%M:%S")
            .or_else(|_| NaiveTime::parse_from_str(x, "%H:%M"))
            .map_err(|_| bad(t))?,
    };
    Ok(Some(d.and_time(tm)))
}

/// 依頼期限のように日付だけ使うもの。時刻が付いていても日付部分だけ取る。
pub fn parse_date(s: &str) -> Result<Option<NaiveDate>> {
    Ok(parse_when(s, DayEdge::Start)?.map(|dt| dt.date()))
}

fn bad(s: &str) -> Error {
    Error::Invalid(format!(
        "日付「{s}」が読めません。2026-11-05、2026/11/05 9:00 のように書いてください。"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, mi, s)
            .unwrap()
    }

    #[test]
    fn accepts_formats_vba_and_iso_produce() {
        let cases = [
            ("2026-11-05", DayEdge::Start, dt(2026, 11, 5, 0, 0, 0)),
            ("2026-11-05", DayEdge::End, dt(2026, 11, 5, 23, 59, 59)),
            ("2026/11/5", DayEdge::Start, dt(2026, 11, 5, 0, 0, 0)),
            ("2026/11/05 9:00:00", DayEdge::End, dt(2026, 11, 5, 9, 0, 0)),
            (
                "2026-11-05T09:30",
                DayEdge::Start,
                dt(2026, 11, 5, 9, 30, 0),
            ),
            (
                "  2026-11-05 09:30  ",
                DayEdge::Start,
                dt(2026, 11, 5, 9, 30, 0),
            ),
        ];
        for (s, e, want) in cases {
            assert_eq!(parse_when(s, e).unwrap(), Some(want), "{s}");
        }
    }

    #[test]
    fn empty_is_none() {
        assert_eq!(parse_when("", DayEdge::Start).unwrap(), None);
        assert_eq!(parse_when("   ", DayEdge::End).unwrap(), None);
    }

    #[test]
    fn rejects_garbage_with_japanese_message() {
        for s in [
            "明日",
            "2026-13-01",
            "2026-02-30",
            "11/05",
            "2026-11-05 25:00",
            "2026-11-05 9:00 extra",
        ] {
            let e = parse_when(s, DayEdge::Start).unwrap_err().to_string();
            assert!(e.contains("読めません"), "{s}: {e}");
        }
    }

    #[test]
    fn date_drops_time() {
        assert_eq!(
            parse_date("2026/11/05 18:00").unwrap(),
            NaiveDate::from_ymd_opt(2026, 11, 5)
        );
    }
}
