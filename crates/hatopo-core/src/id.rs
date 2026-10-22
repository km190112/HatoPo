use chrono::NaiveDateTime;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 時刻順に並ぶ ID。`20261022-153012-a1b2c3d4e5`。ファイル名にそのまま使う。
pub(crate) fn new_id(now: NaiveDateTime) -> String {
    format!("{}-{}", now.format("%Y%m%d-%H%M%S"), random_hex(10))
}

/// 衝突しにくい短い 16 進文字列。暗号用途ではない。
pub(crate) fn random_hex(len: usize) -> String {
    // RandomState はプロセスごとに OS の乱数で種を取る
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    h.write_u32(std::process::id());
    let s = format!("{:016x}", h.finish());
    s[..len.min(16)].to_string()
}

/// ID として受け付ける文字。パス区切りや `..` を通さないための検査。
pub(crate) fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_valid() {
        let now = chrono::NaiveDate::from_ymd_opt(2026, 10, 22)
            .unwrap()
            .and_hms_opt(15, 30, 12)
            .unwrap();
        let ids: std::collections::HashSet<_> = (0..1000).map(|_| new_id(now)).collect();
        assert_eq!(ids.len(), 1000);
        assert!(
            ids.iter()
                .all(|i| i.starts_with("20261022-153012-") && is_valid_id(i))
        );
    }

    #[test]
    fn rejects_path_like_ids() {
        for bad in ["", "..", "a/b", "a\\b", "x.json", "日本"] {
            assert!(!is_valid_id(bad), "{bad}");
        }
    }
}
