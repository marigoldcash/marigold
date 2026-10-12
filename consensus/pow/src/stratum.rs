//! The share target a stratum difficulty stands for.
//!
//! A pool hands a miner a difficulty, not a target; the miner (and the pool,
//! checking the share) turns it into one the same way, or they disagree
//! about every share. This is that one way, kept here so the stratum bridge
//! and a stratum client built on this crate compute identical targets.
//!
//! `target = (2^224 − 1) / difficulty`, the convention the kHeavyHash ASIC
//! firmware and the Go bridge family settled on (so difficulty 1 is 2^224 − 1,
//! not Bitcoin's 0xffff · 2^208). The division is done with eighteen decimal
//! places of the difficulty, as the Go bridge's big.Float division did, so
//! fractional difficulties round the same way they always have.

use kaspa_math::{Uint256, Uint320};

/// Eighteen decimal places of difficulty survive the integer division.
const SCALE: u128 = 1_000_000_000_000_000_000;

fn max_target() -> Uint320 {
    (Uint320::from_u64(1) << 224) - Uint320::from_u64(1)
}

/// The target a share must hash at or under to count for `difficulty`.
///
/// A difficulty that is zero, negative or not a number gives the maximum
/// target (every hash counts), as does one so small that the scaled divisor
/// underflows to zero; a target wider than 256 bits saturates to the largest
/// value, which also lets every hash count. A difficulty too large for the
/// scaled `u128` saturates the divisor instead, and the target gets as small
/// as the arithmetic allows.
pub fn target_for_stratum_difficulty(difficulty: f64) -> Uint256 {
    let max = max_target();
    if difficulty <= 0.0 || !difficulty.is_finite() {
        return narrow(max);
    }
    // `as u128` saturates on overflow and truncates toward zero, matching the
    // bridge's historical conversion.
    let divisor = (difficulty * SCALE as f64) as u128;
    if divisor == 0 {
        return narrow(max);
    }
    narrow(max * Uint320::from_u128(SCALE) / Uint320::from_u128(divisor))
}

/// A 320-bit value into 256 bits, saturating.
fn narrow(wide: Uint320) -> Uint256 {
    if (wide >> 256).is_zero() {
        let bytes = wide.to_le_bytes();
        let mut narrow = [0u8; 32];
        narrow.copy_from_slice(&bytes[..32]);
        Uint256::from_le_bytes(narrow)
    } else {
        Uint256::MAX
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pow2(bits: u32) -> Uint256 {
        Uint256::from_u64(1) << bits
    }

    #[test]
    fn difficulty_one_is_the_maximum_target() {
        assert_eq!(target_for_stratum_difficulty(1.0), pow2(224) - Uint256::from_u64(1));
    }

    #[test]
    fn a_power_of_two_difficulty_shifts_the_target() {
        let max = pow2(224) - Uint256::from_u64(1);
        assert_eq!(target_for_stratum_difficulty(8192.0), max >> 13);
        assert_eq!(target_for_stratum_difficulty(4.0), max >> 2);
    }

    #[test]
    fn higher_difficulty_means_lower_target() {
        let one = target_for_stratum_difficulty(1.0);
        let big = target_for_stratum_difficulty(8192.0);
        let bigger = target_for_stratum_difficulty(1_000_000.0);
        assert!(bigger < big && big < one);
        assert!(!bigger.is_zero());
    }

    #[test]
    fn a_fraction_widens_the_target() {
        // 0.5 doubles the maximum target: exact, since the scaled division has no remainder.
        assert_eq!(target_for_stratum_difficulty(0.5), (pow2(224) - Uint256::from_u64(1)) * 2);
    }

    #[test]
    fn nonsense_and_tiny_difficulties_let_every_hash_count() {
        let max = pow2(224) - Uint256::from_u64(1);
        assert_eq!(target_for_stratum_difficulty(0.0), max);
        assert_eq!(target_for_stratum_difficulty(-3.0), max);
        assert_eq!(target_for_stratum_difficulty(f64::NAN), max);
        assert_eq!(target_for_stratum_difficulty(1e-30), max);
        // Wider than 256 bits: saturates, which is also "everything passes".
        assert_eq!(target_for_stratum_difficulty(1e-12), Uint256::MAX);
    }
}
