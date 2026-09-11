//!
//! The wallet's note key database (FORK-PLAN P7.1, POOL-SPEC.md P5.6 "Key database
//! format"). Unlike the seed-derived `PrvKeyData` accounts, a note has no derivation
//! path — the wallet is "a key-database manager, not an identity" (P5.6) holding one
//! raw private key per note (or, under the shared-pk policy, one key shared by
//! several). This module is serial-keyed (one row per `sn`, per FORK-PLAN P7.1's own
//! wording) rather than key-keyed (POOL-SPEC's `KeyDbEntry` sketch, which lists
//! `known_serials` per key) — a deliberate simplification for this initial DB: the
//! shared-pk case (POS landing pad, P7.5) tolerates the small redundancy of storing
//! the same `sk` under more than one `sn`, and a flat per-serial row is what P7.2-P7.4's
//! spend/receive selection logic wants to query directly.
//!

use crate::imports::*;
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::notepool::DenominationTag;
use secp256k1::{Keypair, SECP256K1, SecretKey};

/// Whether a note's key ever crossed a wallet boundary (POOL-SPEC.md P5.6,
/// "same-key-in-two-wallets hazard", rule 2). `Cold` keys were generated locally and
/// never exported; lazy isolation is fine. `Hot` keys — bearer imports, cross-device
/// exports, backup restores — must be rotated to a fresh `Cold` key at the next
/// opportunity, since their shared-state history may include a wallet this one
/// doesn't control or trust in this moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum NoteProvenance {
    Cold,
    Hot,
}

/// Whether the wallet still believes a serial's row reflects live pool state. Flipped
/// by [`crate::storage::NoteKeyStore::mark_status`] as `NotesChanged` notifications
/// (FORK-PLAN P6.9) arrive for a watched serial or pk — deliberately a plaintext-only
/// mutation (see [`NoteKeyInfo`]) so it never needs the wallet secret, and can be
/// applied by a passive background listener even while the wallet is locked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum NoteStatus {
    /// Still believed to be a live, spendable note under this row's key.
    Active,
    /// The pool does not have this serial, and a synced node has said so three
    /// separate times. Something happened to it and this wallet cannot tell
    /// what: either the transaction that would have created it never landed —
    /// in which case no money ever moved and the balance is still on the
    /// ledger — or it was consumed by a wallet sharing the key.
    ///
    /// Deliberately not [`Self::Superseded`], which asserts the note was
    /// spent. Saying that without evidence would misstate what happened to
    /// somebody's money. Excluded from the balance, kept for a later archival
    /// lookup that can say which of the two it was.
    Unknown,
    /// The serial was consumed on-chain (spent by us via rotation/split/merge, spent
    /// elsewhere by a wallet sharing this key, or redeemed) — kept as a tombstone
    /// rather than deleted immediately, so a caller with the wallet secret can later
    /// reconcile (e.g. drop the row, or confirm a rotation landed under a new `sn`).
    Superseded,
    /// Bearer-exported (FORK-PLAN P7.4, POOL-SPEC.md P5.5a): the key was handed to
    /// someone else, and the note is theirs the moment they rotate it — until then
    /// both parties can technically spend it (the defining property of a bearer
    /// instrument). Excluded from balance and from every spend/fee-source selection;
    /// flips to [`Self::Superseded`] when the receiver's rotation is observed
    /// on-chain (the ordinary `NotesChanged` removal path).
    HandedOver,
    /// A copy of this key is on a phone (DECISIONS.md: "Mobile custody: mirror
    /// plus rotation"). The home wallet keeps the key — that is what makes this
    /// a mirror rather than a move, and what preserves the ability to revoke by
    /// rotating the serial — but will not spend it, source a fee from it, or
    /// merge it. Housekeeping that consumed a note the holder was carrying
    /// would make their money vanish at the till with nothing to explain it.
    ///
    /// Distinct from [`Self::HandedOver`], which means given away for good, and
    /// from [`Self::Active`], which means free to spend here.
    Mirrored,
}

/// One row of the note key database (POOL-SPEC.md P5.6's `KeyDbEntry`, flattened to
/// per-serial per FORK-PLAN P7.1). Holds the sole copy of a note's private key —
/// "losing the key database is losing the notes" (P5.6) — so this is the sensitive
/// half of the store; kept encrypted at rest (mirrors `PrvKeyData`'s handling of raw
/// key material) and zeroized on drop.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct NoteKeyEntry {
    pub sn: Hash,
    pub sk: [u8; 32],
    pub d: DenominationTag,
    pub provenance: NoteProvenance,
}

impl NoteKeyEntry {
    pub fn new(sn: Hash, sk: [u8; 32], d: DenominationTag, provenance: NoteProvenance) -> Self {
        Self { sn, sk, d, provenance }
    }

    /// The x-only BIP340 Schnorr public key this entry's `sk` corresponds to — never
    /// stored redundantly on this type (POOL-SPEC.md P5.6's `KeyDbEntry` doc comment:
    /// "the corresponding pk is derivable, not stored redundantly"), but cached
    /// plaintext on [`NoteKeyInfo`] since, unlike `sk`, a note's `pk` is not sensitive
    /// (the pool is plaintext — anyone can already see it on-chain).
    pub fn derive_pk(&self) -> Result<[u8; 32]> {
        let secret_key = SecretKey::from_slice(&self.sk).map_err(|e| Error::Custom(format!("invalid note secret key: {e}")))?;
        let keypair = Keypair::from_secret_key(SECP256K1, &secret_key);
        Ok(keypair.x_only_public_key().0.serialize())
    }
}

impl Zeroize for NoteKeyEntry {
    fn zeroize(&mut self) {
        self.sk.zeroize();
    }
}

impl Drop for NoteKeyEntry {
    fn drop(&mut self) {
        self.sk.zeroize();
    }
}

/// The plaintext-safe half of a note key row: everything about a held note that isn't
/// the private key itself — `sn`, `pk`, `d` and `provenance` are all either already
/// public on-chain or metadata about the wallet's own key hygiene, none of it secret.
/// Kept as a separate in-memory index (mirrors `PrvKeyDataInfo` alongside
/// `PrvKeyData`) so cheap operations — enumeration, building a `NotesChangedScope`,
/// flipping `status` on a live notification — never need the wallet secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct NoteKeyInfo {
    pub sn: Hash,
    pub pk: [u8; 32],
    pub d: DenominationTag,
    pub provenance: NoteProvenance,
    pub status: NoteStatus,
}

impl NoteKeyInfo {
    pub fn new(sn: Hash, pk: [u8; 32], d: DenominationTag, provenance: NoteProvenance) -> Self {
        Self { sn, pk, d, provenance, status: NoteStatus::Active }
    }
}

impl TryFrom<&NoteKeyEntry> for NoteKeyInfo {
    type Error = Error;

    fn try_from(entry: &NoteKeyEntry) -> Result<Self> {
        Ok(Self::new(entry.sn, entry.derive_pk()?, entry.d, entry.provenance))
    }
}

impl crate::storage::IdT for NoteKeyInfo {
    type Id = Hash;
    fn id(&self) -> &Hash {
        &self.sn
    }
}

pub type NoteKeyMap = HashMap<Hash, NoteKeyEntry>;

/// A payment-request key (FORK-PLAN P7.3, POOL-SPEC.md P5.5b "sign-to-fresh-pk"):
/// a locally generated keypair whose `pk` has been handed out in a payment-request
/// QR but which owns no serial *yet* — the spec's "unpaid-invoice semantics" ("the
/// wallet was watching that `pk` since generating it"). Persisted the moment the
/// request is created, before the QR is ever shown: a crash between issuing a
/// request and the payment landing must not lose the only key that can ever spend
/// the payer's notes. Sensitive (holds a raw `sk`) — stored encrypted, mirroring
/// [`NoteKeyEntry`]'s handling, and zeroized on drop.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct PaymentRequestKey {
    pub sk: [u8; 32],
    /// The requested amount, if the request pinned one (POOL-SPEC.md P5.6's QR
    /// formats: the 40-byte form carries an amount; the 32-byte static/printed form
    /// omits it and the payer enters the amount manually).
    pub amount_petals: Option<u64>,
}

impl PaymentRequestKey {
    pub fn new(sk: [u8; 32], amount_petals: Option<u64>) -> Self {
        Self { sk, amount_petals }
    }

    /// The x-only BIP340 pk this request advertises — derived, never stored (same
    /// rule as [`NoteKeyEntry::derive_pk`]).
    pub fn derive_pk(&self) -> Result<[u8; 32]> {
        let secret_key = SecretKey::from_slice(&self.sk).map_err(|e| Error::Custom(format!("invalid request secret key: {e}")))?;
        Ok(Keypair::from_secret_key(SECP256K1, &secret_key).x_only_public_key().0.serialize())
    }
}

impl Zeroize for PaymentRequestKey {
    fn zeroize(&mut self) {
        self.sk.zeroize();
    }
}

impl Drop for PaymentRequestKey {
    fn drop(&mut self) {
        self.sk.zeroize();
    }
}

/// Plaintext-safe half of a payment request (mirrors [`NoteKeyInfo`]'s split from
/// [`NoteKeyEntry`]): the `pk` is public by construction — it's literally the
/// content of the QR being shown around — and the amount is invoice metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct PaymentRequestInfo {
    pub pk: [u8; 32],
    pub amount_petals: Option<u64>,
}

pub type PaymentRequestMap = HashMap<[u8; 32], PaymentRequestKey>;

/// Result of [`crate::storage::NoteKeyStore::apply_notes_changed`] — which serials it
/// actually updated, split by kind so a caller can tell "fully reconciled" from
/// "partially applied, retry `deferred` once a wallet secret is available."
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NotesChangedApplyResult {
    /// Rows whose `status` was flipped to `Superseded` (plaintext-only, always applied).
    pub superseded: Vec<Hash>,
    /// New rows inserted for a `pk` this wallet already held a key for.
    pub added: Vec<Hash>,
    /// `added`-notification serials that matched a held `pk` but couldn't be written
    /// because no wallet secret was supplied.
    pub deferred: Vec<Hash>,
}
