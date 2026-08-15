use thiserror::Error;

/// Stateless `PoolOp` validation failures (POOL-SPEC.md P5.3, FORK-PLAN P6.3) — checks
/// that hold regardless of any pool/consensus state: structural shape, duplicate
/// serials, collection-size bounds, and Schnorr-signature well-formedness. Deliberately
/// excludes anything requiring the live pool view (serial existence, current-`pk`
/// verification, freshness-window comparison against a POV DAA score, or actual value
/// conservation against consumed notes' *current* denominations) — those are P6.4's job.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum PoolOpValidationError {
    #[error("pool op has no {0}")]
    EmptyCollection(&'static str),

    #[error("pool op has {0} {1} where the max allowed is {2}")]
    TooManyItems(usize, &'static str, usize),

    #[error("signed group #{0} has no serials")]
    EmptySignedGroup(usize),

    #[error("serial {0} appears more than once across the op's consumed set")]
    DuplicateSerial(crate::Hash),
}
