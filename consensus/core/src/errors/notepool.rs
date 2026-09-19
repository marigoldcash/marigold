use crate::Hash;
use thiserror::Error;

/// Stateless `PoolOp` validation failures (POOL-SPEC.md P5.3, PLAN P6.3) — checks
/// that hold regardless of any pool/consensus state: structural shape, duplicate
/// serials, and collection-size bounds. Everything requiring the live pool view is
/// [`PoolOpContextError`]'s domain instead.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum PoolOpValidationError {
    #[error("pool op has no {0}")]
    EmptyCollection(&'static str),

    #[error("pool op has {0} {1} where the max allowed is {2}")]
    TooManyItems(usize, &'static str, usize),

    #[error("signed group #{0} has no serials")]
    EmptySignedGroup(usize),

    #[error("serial {0} appears more than once across the op's consumed set")]
    DuplicateSerial(Hash),

    #[error("lock index {0} is out of range for {1} produced notes")]
    LockIndexOutOfRange(u32, usize),

    #[error("lock indices must be strictly ascending")]
    LockIndicesNotAscending,
}

/// Stateful `PoolOp` validation failures (POOL-SPEC.md P5.3's validation orders, PLAN
/// P6.4) — checks against a live composed pool view. A transaction failing one of these
/// is excluded from its context's accepted transactions (the P5.3 "first accepted wins"
/// mechanism), exactly like a UTXO double-spend in a merged block.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum PoolOpContextError {
    #[error("pool-op transaction payload failed to decode")]
    MalformedPayload,

    #[error("consumed serial {0} does not exist in the pool")]
    SerialNotFound(Hash),

    #[error("produced serial {0} already exists in the pool")]
    SerialAlreadyExists(Hash),

    #[error("signed group #{0} spans serials under different current keys (one signature cannot authenticate two keys)")]
    MixedKeysInGroup(usize),

    #[error("signed group #{0}'s current pk is not a valid x-only public key")]
    BadPublicKey(usize),

    #[error("signed group #{0}'s signature does not verify against the serials' current pk")]
    BadSignature(usize),

    #[error("freshness anchor {anchor} is in the future of the validation context's POV DAA score {pov}")]
    AnchorInFuture { anchor: u64, pov: u64 },

    #[error("freshness anchor {anchor} is stale at POV DAA score {pov} (window: {window})")]
    StaleAnchor { anchor: u64, pov: u64, window: u64 },

    #[error("conservation violated: consumed {consumed} petals < produced {produced} petals")]
    InsufficientConsumedValue { consumed: u64, produced: u64 },

    #[error("locked notes are not active yet at POV DAA score {pov}")]
    LocksNotActive { pov: u64 },

    #[error("pool diff algebra violation: {0}")]
    Algebra(#[from] PoolAlgebraError),
}

/// Violations of the pool-diff composition algebra (mirrors
/// `crate::utxo::utxo_error::UtxoAlgebraError`). Reaching one of these from validated
/// ops indicates duplicate serial usage that earlier checks should have excluded.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum PoolAlgebraError {
    #[error("serial {0} added twice")]
    DuplicateAdd(Hash),

    #[error("serial {0} removed twice")]
    DuplicateRemove(Hash),
}
