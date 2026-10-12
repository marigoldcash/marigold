#![allow(clippy::arithmetic_side_effects)]

// public for benchmarks
#[doc(hidden)]
pub mod matrix;
pub mod stratum;
#[cfg(feature = "wasm32-sdk")]
pub mod wasm;
#[doc(hidden)]
pub mod xoshiro;

use std::cmp::max;

use crate::matrix::Matrix;
use kaspa_consensus_core::{BlockLevel, hashing, header::Header};
use kaspa_hashes::{Hash, PowHash};
use kaspa_math::Uint256;

/// State is an intermediate data structure with pre-computed values to speed up mining.
pub struct State {
    pub(crate) matrix: Matrix,
    pub(crate) target: Uint256,
    // PRE_POW_HASH || TIME || 32 zero byte padding; without NONCE
    pub(crate) hasher: PowHash,
}

impl State {
    #[inline]
    pub fn new(header: &Header) -> Self {
        let target = Uint256::from_compact_target_bits(header.bits);
        // Zero out the time and nonce.
        let pre_pow_hash = hashing::header::hash_override_nonce_time(header, 0, 0);
        // PRE_POW_HASH || TIME || 32 zero byte padding || NONCE
        let hasher = PowHash::new(pre_pow_hash, header.timestamp);
        let matrix = Matrix::generate(pre_pow_hash);

        Self { matrix, target, hasher }
    }

    /// The same state built from what a stratum job carries instead of a header.
    ///
    /// A stratum miner never sees the header: the pool sends the pre-PoW hash
    /// and the timestamp, and the share target comes from the difficulty it
    /// was given (`stratum::target_for_stratum_difficulty`). This is `new`
    /// with the hashing step already done, so a client and the node agree on
    /// every nonce.
    #[inline]
    pub fn from_pre_pow(pre_pow_hash: Hash, timestamp: u64, target: Uint256) -> Self {
        let hasher = PowHash::new(pre_pow_hash, timestamp);
        let matrix = Matrix::generate(pre_pow_hash);
        Self { matrix, target, hasher }
    }

    #[inline]
    #[must_use]
    /// PRE_POW_HASH || TIME || 32 zero byte padding || NONCE
    pub fn calculate_pow(&self, nonce: u64) -> Uint256 {
        // Hasher already contains PRE_POW_HASH || TIME || 32 zero byte padding; so only the NONCE is missing
        let hash = self.hasher.clone().finalize_with_nonce(nonce);
        let hash = self.matrix.heavy_hash(hash);
        Uint256::from_le_bytes(hash.as_bytes())
    }

    #[inline]
    #[must_use]
    pub fn check_pow(&self, nonce: u64) -> (bool, Uint256) {
        let pow = self.calculate_pow(nonce);
        // The pow hash must be less or equal than the claimed target.
        (pow <= self.target, pow)
    }
}

pub fn calc_block_level(header: &Header, max_block_level: BlockLevel) -> BlockLevel {
    let (block_level, _) = calc_block_level_check_pow(header, max_block_level);
    block_level
}

pub fn calc_block_level_check_pow(header: &Header, max_block_level: BlockLevel) -> (BlockLevel, bool) {
    if header.parents_by_level.is_empty() {
        return (max_block_level, true); // Genesis has the max block level
    }

    let state = State::new(header);
    let (passed, pow) = state.check_pow(header.nonce);
    let block_level = calc_level_from_pow(pow, max_block_level);
    (block_level, passed)
}

pub fn calc_level_from_pow(pow: Uint256, max_block_level: BlockLevel) -> BlockLevel {
    let signed_block_level = max_block_level as i64 - pow.bits() as i64;
    max(signed_block_level, 0) as BlockLevel
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(bits: u32, timestamp: u64) -> Header {
        let mut header = Header::from_precomputed_hash(Hash::from_bytes([7u8; 32]), vec![]);
        header.bits = bits;
        header.timestamp = timestamp;
        header
    }

    #[test]
    fn from_pre_pow_hashes_exactly_like_new() {
        let header = header(0x1e7fffff, 1_700_000_000_123);
        let from_header = State::new(&header);
        let from_job = State::from_pre_pow(
            hashing::header::hash_override_nonce_time(&header, 0, 0),
            header.timestamp,
            Uint256::from_compact_target_bits(header.bits),
        );
        for nonce in [0u64, 1, 42, 0xdead_beef, u64::MAX] {
            assert_eq!(from_header.calculate_pow(nonce), from_job.calculate_pow(nonce), "nonce {nonce:#x}");
            assert_eq!(from_header.check_pow(nonce), from_job.check_pow(nonce));
        }
    }

    #[test]
    fn from_pre_pow_checks_against_the_target_it_was_given() {
        let header = header(0x1e7fffff, 1);
        let pre_pow = hashing::header::hash_override_nonce_time(&header, 0, 0);
        let everything = State::from_pre_pow(pre_pow, header.timestamp, Uint256::MAX);
        let nothing = State::from_pre_pow(pre_pow, header.timestamp, Uint256::from_u64(0));
        assert!(everything.check_pow(5).0);
        assert!(!nothing.check_pow(5).0);
    }
}
