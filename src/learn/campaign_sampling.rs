//! Checked layout/rollout slots within one stage seed band.
pub fn one() -> usize {
    1
}
pub fn sample_seed(
    base: u64,
    layout: usize,
    seed: usize,
    layouts: usize,
    seeds: usize,
) -> Result<u64, String> {
    let count = layouts.checked_mul(seeds).ok_or("sample count overflow")?;
    if count == 0 || count > 1_000_000 || layout >= layouts || seed >= seeds {
        return Err("invalid sample count or stage seed interval".into());
    }
    base.checked_add((layout * seeds + seed) as u64)
        .ok_or("sample seed overflow".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checked_disjoint_slots() {
        assert_eq!(sample_seed(10, 1, 1, 2, 2).unwrap(), 13);
        assert_eq!(sample_seed(0, 0, 999999, 1, 1000000).unwrap(), 999999);
        assert!(sample_seed(0, 0, 0, 1, 1000001).is_err());
        assert!(sample_seed(u64::MAX, 1, 0, 2, 1).is_err());
        assert!(sample_seed(0, 0, 0, 0, 1).is_err());
    }
}
