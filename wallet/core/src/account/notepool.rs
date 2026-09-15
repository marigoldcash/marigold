//!
//! Wallet-side construction of note-pool operation transactions (FORK-PLAN P7.2+).
//!
//! Mint needs real transparent inputs (funding the new notes), so it goes through the
//! ordinary [`Generator`]/[`Signer`] pipeline exactly like [`Account::send`] — the only
//! addition is `GeneratorSettings::with_subnetwork_id` (added alongside this module) so
//! the final transaction carries `SUBNETWORK_ID_NOTE_POOL` and a `MintOp` payload
//! instead of `SUBNETWORK_ID_NATIVE`. Redeem, by contrast, needs *no* transparent
//! inputs at all — POOL-SPEC.md P5.2 designs it to be self-funding, its transparent
//! output paid entirely from the consumed notes' value — which doesn't fit the
//! Generator's UTXO-aggregation model (it always aggregates *toward* a requested
//! output value; it has no notion of value arriving from outside the UTXO set). Redeem
//! is therefore hand-built, mirroring `trustee-signer::anchor_transaction`'s zero-input
//! pattern: construct the `Transaction` directly, compute its mass with the consensus
//! `MassCalculator` (the wallet's own `tx::mass` calculator doesn't know about
//! `SUBNETWORK_ID_NOTE_POOL` payload costing — see NOTES.md's P7.2 entry), sign the
//! pool-op's `SignedGroup`s with the note's own raw secret key (never a BIP32-derived
//! one — notes have no derivation path), and submit directly over RPC.
//!

use crate::account::Account;
use crate::imports::*;
use crate::storage::{NoteKeyEntry, NoteKeyInfo, NoteProvenance, NoteStatus, PaymentRequestKey};
use crate::tx::{Fees, Generator, GeneratorSettings, PaymentDestination, PaymentOutputs, Signer};
use crate::wallet::Wallet;
use kaspa_consensus_core::config::params::Params;
use kaspa_consensus_core::constants::TX_VERSION_TOCCATA;
use kaspa_consensus_core::mass::MassCalculator;
use kaspa_consensus_core::notepool::{
    DENOMINATION_PETALS, DenominationTag, FreshnessAnchor, MintOp, NewNote, PoolOp, RedeemOp, SignedGroup, TransferOp,
    hashing::{serial_hash, signing_hash, transparent_outputs_hash},
};
use kaspa_consensus_core::subnets::SUBNETWORK_ID_NOTE_POOL;
use kaspa_consensus_core::tx::{PopulatedTransaction, Transaction, TransactionOutput};
use kaspa_hashes::Hash;
use kaspa_notify::scope::{NotesChangedScope, Scope};
use kaspa_rpc_core::notify::connection::{ChannelConnection, ChannelType};
use kaspa_txscript::pay_to_address_script;
use rand::seq::SliceRandom;
use rand::{Rng, RngCore};
use secp256k1::{Keypair, Message, SECP256K1, SecretKey};
use std::time::Duration;
use workflow_core::abortable::Abortable;
use workflow_core::channel::Channel;

/// The fee granularity of a pure pool `Transfer` (FORK-PLAN P7.3 design decision,
/// recorded in NOTES.md): since every consumed and produced value is a multiple of the
/// smallest denomination, `fee = Σconsumed − Σproduced` is *necessarily* a multiple of
/// 0.01 MAGLD — pool-op fees are quantized whether we like it or not. The wallet
/// sources them per POOL-SPEC.md P5.2's fee-stamp mechanism (an extra consumed note
/// with its excess over the fee returned as change to fresh own keys), or — when the
/// wallet holds no spare note at all, e.g. a first-ever bearer receive — by
/// withholding one quantum from the rotation's own produced decomposition.
pub const FEE_QUANTUM_PETALS: u64 = DENOMINATION_PETALS[0];

/// Decompose `petals` into the fixed P1.6 denomination ladder, largest-first. Always
/// exact for a multiple of the smallest denomination — each rung is exactly 10x the
/// last, so this is plain base-10 digit decomposition. `None` if `petals` isn't an
/// exact multiple of the smallest denomination (0.01 MAGLD / 1,000,000 petals); nothing
/// in POOL-SPEC lets a mint leave a sub-denomination remainder unminted-but-spent.
pub fn decompose_amount(petals: u64) -> Option<Vec<DenominationTag>> {
    let smallest = DENOMINATION_PETALS[0];
    if petals == 0 || !petals.is_multiple_of(smallest) {
        return None;
    }
    let mut remaining = petals;
    let mut notes = Vec::new();
    for (index, &value) in DENOMINATION_PETALS.iter().enumerate().rev() {
        let count = remaining / value;
        remaining -= count * value;
        if count > 0 {
            let tag = DenominationTag::try_from(index as u8).expect("index within DENOMINATION_PETALS bounds");
            notes.extend(std::iter::repeat_n(tag, count as usize));
        }
    }
    debug_assert_eq!(remaining, 0);
    Some(notes)
}

/// Result of [`mint`] — every transaction submitted (compound aggregation steps, if
/// any, followed by the final mint transaction) and the freshly generated note key
/// entries, already persisted in the wallet's note key store as `Cold` (POOL-SPEC.md
/// P5.6 — a key generated locally and never exported is Cold by definition).
pub struct MintResult {
    pub transaction_ids: Vec<Hash>,
    pub notes: Vec<NoteKeyEntry>,
}

/// Fee rate for note-pool transactions, in sompi per gram.
///
/// The mempool's minimum relay price is 100 sompi/gram
/// (`DEFAULT_MINIMUM_RELAY_TRANSACTION_FEE` = 100_000 sompi/kg in
/// `mining/src/mempool/config.rs`), and the generator charges the MAX of that
/// minimum and the requested rate — not their sum — so any rate at or below
/// 100 changes nothing at all. Paying the exact minimum is fragile: the
/// mempool recomputes mass independently and has landed tens of grams above
/// the generator's figure, rejecting a mint for being 0.07% short. 105 buys a
/// 5% margin over the floor, which in absolute terms is a rounding error and
/// makes that class of rejection impossible.
pub const POOL_FEE_RATE: f64 = 105.0;

/// Display-oriented progress reporting for long-running note operations —
/// a mint over a mining wallet can sweep hundreds of thousands of UTXOs in
/// thousands of batch transactions, minutes during which a silent CLI reads
/// as a hang. Messages are pre-formatted, ready to print.
pub type NoteProgress = std::sync::Arc<dyn Fn(String) + Send + Sync>;

/// Mint `amount_petals` worth of notes, splitting into the P1.6 denomination ladder,
/// funded from the account's transparent balance (FORK-PLAN P7.2).
pub async fn mint(
    account: Arc<dyn Account>,
    wallet_secret: Secret,
    payment_secret: Option<Secret>,
    amount_petals: u64,
    fee_rate: Option<f64>,
    abortable: &Abortable,
) -> Result<MintResult> {
    mint_with_progress(account, wallet_secret, payment_secret, amount_petals, fee_rate, abortable, None).await
}

pub async fn mint_with_progress(
    account: Arc<dyn Account>,
    wallet_secret: Secret,
    payment_secret: Option<Secret>,
    amount_petals: u64,
    fee_rate: Option<f64>,
    abortable: &Abortable,
    progress: Option<NoteProgress>,
) -> Result<MintResult> {
    let denominations = decompose_amount(amount_petals).ok_or_else(|| {
        Error::Custom(format!(
            "{amount_petals} petals is not an exact multiple of the smallest denomination ({} petals = 0.01 MAGLD)",
            DENOMINATION_PETALS[0]
        ))
    })?;

    // Every new note needs a fresh keypair before the MintOp payload (which carries
    // each note's pk) can even be built.
    let mut generated: Vec<(DenominationTag, [u8; 32], [u8; 32])> = Vec::with_capacity(denominations.len());
    for d in denominations {
        let sk = SecretKey::new(&mut secp256k1::rand::thread_rng());
        let pk = Keypair::from_secret_key(SECP256K1, &sk).x_only_public_key().0.serialize();
        generated.push((d, sk.secret_bytes(), pk));
    }
    let new_notes: Vec<NewNote> = generated.iter().map(|(d, _, pk)| NewNote { d: *d, pk: *pk }).collect();
    let payload = PoolOp::Mint(MintOp { new_notes }).encode_payload();

    let keydata = account.prv_key_data(wallet_secret.clone()).await?;
    let signer = Arc::new(Signer::new(account.clone(), keydata, payment_secret));

    let fee_rate = fee_rate.or(Some(POOL_FEE_RATE));

    // No explicit payment output: the minted value is withheld from change via
    // `Fees::SenderPays(amount_petals)` rather than appearing as a real transparent
    // output (see this module's doc comment and NOTES.md's P7.2 entry for why this
    // is the one Generator trick this fork needed, and why it needed nothing more).
    let settings = GeneratorSettings::try_new_with_account(
        account.clone(),
        PaymentDestination::PaymentOutputs(PaymentOutputs { outputs: vec![] }),
        fee_rate,
        Fees::SenderPays(amount_petals),
        Some(payload),
    )?
    .with_subnetwork_id(SUBNETWORK_ID_NOTE_POOL);

    let generator = Generator::try_new(settings, Some(signer), Some(abortable))?;
    let mut stream = generator.stream();
    let mut transaction_ids = Vec::new();
    while let Some(transaction) = stream.try_next().await? {
        transaction.try_sign()?;
        transaction_ids.push(transaction.try_submit(&account.wallet().rpc_api()).await?);
        if let Some(progress) = &progress {
            let n = transaction_ids.len();
            if n == 1 || n % 50 == 0 {
                progress(format!("signed and submitted {n} batch transaction(s)..."));
            }
        }
    }
    let tx_id = *transaction_ids.last().ok_or_else(|| Error::Custom("mint produced no transaction".to_string()))?;

    // The wallet is the notes' creator, so — unlike a receive flow — it knows each
    // serial deterministically the instant the final transaction id is known, with no
    // need to wait for confirmation (POOL-SPEC.md P5.6: "the wallet already knows
    // every serial it holds").
    let note_key_store = account.wallet().store().as_note_key_store()?;
    let mut notes = Vec::with_capacity(generated.len());
    for (index, (d, sk, _pk)) in generated.into_iter().enumerate() {
        let sn = serial_hash(&tx_id, index as u32);
        let entry = NoteKeyEntry::new(sn, sk, d, NoteProvenance::Cold);
        note_key_store.store(&wallet_secret, entry.clone()).await?;
        notes.push(entry);
    }

    Ok(MintResult { transaction_ids, notes })
}

/// Which notes to redeem: an explicit serial list, or "select owned notes, largest
/// denomination first, until their combined value covers at least this many petals."
/// A fixed-denomination note can't be partially redeemed, so an amount-based redeem
/// pays out the *selected notes'* full combined value (minus fee) — which may exceed
/// the requested amount — never an exact-change partial note.
pub enum RedeemSelection {
    Serials(Vec<Hash>),
    Amount(u64),
}

pub struct RedeemResult {
    pub transaction_id: Hash,
    pub redeemed_value_petals: u64,
    pub fee_petals: u64,
    pub serials: Vec<Hash>,
}

/// Redeem notes back to transparent balance (FORK-PLAN P7.2). Builds a zero-transparent-
/// input `RedeemOp` transaction directly (see this module's doc comment for why),
/// signs each `SignedGroup` with its notes' own key(s), and submits over RPC.
pub async fn redeem(account: Arc<dyn Account>, wallet_secret: Secret, selection: RedeemSelection) -> Result<RedeemResult> {
    redeem_to(account, wallet_secret, selection, None).await
}

/// Redeem notes, optionally paying an EXTERNAL address in the same
/// transaction: `destination` is `(address, petals)`, and whatever the
/// redeemed notes are worth beyond that (less the fee) returns to this
/// account's own ledger address. One transaction destroys the notes and pays
/// the recipient — the shape an exchange deposit wants. Consensus already
/// supports it: a pool op's consumed value funds transparent outputs, the
/// count of which is unconstrained, and every pool-op signature covers
/// `tx.outputs` (POOL-SPEC P5.2, review-1 fix), so the destination is bound
/// by the signature and cannot be rewritten in flight.
pub async fn redeem_to(
    account: Arc<dyn Account>,
    wallet_secret: Secret,
    selection: RedeemSelection,
    destination: Option<(Address, u64)>,
) -> Result<RedeemResult> {
    let change = account.change_address()?;
    redeem_with(account.wallet(), wallet_secret, selection, destination, Some(change)).await
}

/// The redeem itself, bound to a wallet rather than an account. `change` is
/// where value beyond the destination amount (less the fee) goes; a wallet
/// that keeps notes only has no such place (FORK-PLAN P8.0b), and passes
/// `None`. Then the notes must cover the destination amount to within one
/// [`FEE_QUANTUM_PETALS`], and the whole redeemed value less the fee goes to
/// the destination — a deposit may arrive a fraction over what was asked,
/// never under. Anything further over is refused rather than handed to the
/// recipient or the miner: a `RedeemOp` cannot produce a note, so there is
/// nowhere else for it to go.
pub async fn redeem_with(
    wallet: &Arc<Wallet>,
    wallet_secret: Secret,
    selection: RedeemSelection,
    destination: Option<(Address, u64)>,
    change: Option<Address>,
) -> Result<RedeemResult> {
    let note_key_store = wallet.store().as_note_key_store()?;
    let change_script = match (&change, &destination) {
        (Some(address), _) => Some(pay_to_address_script(address)),
        (None, Some(_)) => None,
        (None, None) => return Err(Error::Custom("redeem: this wallet keeps notes only — there is no ledger to redeem to".to_string())),
    };

    let serials = match selection {
        RedeemSelection::Serials(serials) => {
            if serials.is_empty() {
                return Err(Error::Custom("no serials given to redeem".to_string()));
            }
            for sn in &serials {
                let info = note_key_store
                    .load_info(sn)
                    .await?
                    .ok_or_else(|| Error::Custom(format!("serial {sn} is not in the note key database")))?;
                if info.status != NoteStatus::Active {
                    return Err(Error::Custom(format!("serial {sn} is not active (already superseded)")));
                }
            }
            serials
        }
        RedeemSelection::Amount(target_petals) => {
            let candidates: Vec<_> = note_key_store
                .iter()
                .await?
                .try_filter(|info| futures::future::ready(info.status == NoteStatus::Active))
                .try_collect()
                .await?;
            let mut counts = [0usize; DENOMINATION_PETALS.len()];
            for info in &candidates {
                counts[info.d as usize] += 1;
            }
            let total: u64 = candidates.iter().map(|info| DENOMINATION_PETALS[info.d as usize]).sum();
            let Some(take) = cover_amount(&counts, target_petals) else {
                return Err(Error::Custom(format!(
                    "insufficient note balance: {total} petals available, {target_petals} requested"
                )));
            };
            let mut take = take;
            let mut selected = Vec::new();
            for info in candidates {
                let d = info.d as usize;
                if take[d] > 0 {
                    take[d] -= 1;
                    selected.push(info.sn);
                }
            }
            selected
        }
    };

    // Load full entries (needs the secret — the store's plaintext info half doesn't
    // carry `sk`) and group serials sharing a key into one `SignedGroup` per key
    // (POOL-SPEC.md P5.2 — one signature can cover every serial under a shared pk).
    let mut entries = Vec::with_capacity(serials.len());
    for sn in &serials {
        let entry = note_key_store
            .load_key(&wallet_secret, sn)
            .await?
            .ok_or_else(|| Error::Custom(format!("serial {sn} has no stored key")))?;
        entries.push(entry);
    }

    let redeemed_value_petals: u64 = entries.iter().map(|e| DENOMINATION_PETALS[e.d as usize]).sum();

    let mut groups_by_sk: HashMap<[u8; 32], Vec<Hash>> = HashMap::new();
    for entry in &entries {
        groups_by_sk.entry(entry.sk).or_default().push(entry.sn);
    }

    let network_id = wallet.network_id()?;
    let params = Params::from(network_id);
    let mass_calculator = MassCalculator::new_with_consensus_params(&params);

    let server_info = wallet.rpc_api().get_server_info().await?;
    let freshness = FreshnessAnchor { anchor_daa_score: server_info.virtual_daa_score };

    // `PoolOp::Redeem`'s borsh tag / `NotePoolSigningHash` op_type byte (notepool/mod.rs
    // `PoolOp::op_type()`) — pinned as a constant here rather than round-tripping
    // through a throwaway `RedeemOp` just to read it back off.
    const REDEEM_OP_TYPE: u8 = 2;

    // Build with placeholder (zero) signatures first — a `SignedGroup`'s wire size is
    // fixed regardless of signature content, so mass computed against the placeholder
    // payload is identical to the final one, and doesn't need recomputing after signing.
    let placeholder_groups: Vec<SignedGroup> =
        groups_by_sk.values().map(|group_serials| SignedGroup { serials: group_serials.clone(), signature: [0u8; 64] }).collect();
    let placeholder_payload = PoolOp::Redeem(RedeemOp { consumed: placeholder_groups, freshness }).encode_payload();
    // Placeholder must have the same OUTPUT COUNT as the final transaction —
    // mass (and therefore the fee) depends on it, and a two-output redeem
    // costs more than a one-output one.
    let placeholder_outputs = match (&destination, &change_script) {
        (None, Some(change_script)) => vec![TransactionOutput::new(redeemed_value_petals, change_script.clone())],
        (Some((address, petals)), Some(change_script)) => vec![
            TransactionOutput::new(*petals, pay_to_address_script(address)),
            TransactionOutput::new(redeemed_value_petals.saturating_sub(*petals), change_script.clone()),
        ],
        (Some((address, _)), None) => vec![TransactionOutput::new(redeemed_value_petals, pay_to_address_script(address))],
        (None, None) => unreachable!("rejected above"),
    };
    let placeholder_tx =
        Transaction::new(TX_VERSION_TOCCATA, vec![], placeholder_outputs, 0, SUBNETWORK_ID_NOTE_POOL, 0, placeholder_payload);
    let populated = PopulatedTransaction::new(&placeholder_tx, vec![]);
    // Same sizing rule as a transfer, and for the same reasons: the node
    // charges max(compute, transient, storage), and a redeem carries a note
    // payload whose compute mass can exceed the storage mass of its single
    // output. Reading storage mass alone underprices a redeem of many notes.
    let contextual = mass_calculator
        .calc_contextual_masses(&populated)
        .ok_or_else(|| Error::Custom("redeem: mass calculation failed".to_string()))?
        .storage_mass;
    let non_contextual = mass_calculator.calc_non_contextual_masses(&placeholder_tx);
    let mass = contextual.max(non_contextual.compute_mass).max(non_contextual.transient_mass);

    // And the same feerate floor: the node's estimate is a priority signal
    // reporting 1 petal per gram, while the mempool refuses anything under its
    // 100-per-gram relay minimum however patient the sender.
    let feerate = match wallet.rpc_api().get_fee_estimate().await {
        Ok(estimate) => estimate.normal_buckets.first().map(|b| b.feerate).unwrap_or(1.0),
        Err(_) => 1.0,
    }
    .max(POOL_FEE_RATE);
    let fee_petals = (mass as f64 * feerate).ceil() as u64;

    if redeemed_value_petals <= fee_petals {
        return Err(Error::Custom(format!(
            "redeemed value ({redeemed_value_petals} petals) does not cover the estimated fee ({fee_petals} petals)"
        )));
    }
    let output_value = redeemed_value_petals - fee_petals;
    let outputs = match (&destination, change_script) {
        (None, Some(change_script)) => vec![TransactionOutput::new(output_value, change_script)],
        (Some((address, petals)), Some(change_script)) => {
            if *petals > output_value {
                return Err(Error::Custom(format!(
                    "redeemed value after fee ({output_value} petals) does not cover the requested payment ({petals} petals)"
                )));
            }
            let change = output_value - petals;
            let mut outputs = vec![TransactionOutput::new(*petals, pay_to_address_script(address))];
            // Dust-sized change is left to the fee rather than created as an
            // unspendable output.
            if change >= DENOMINATION_PETALS[0] {
                outputs.push(TransactionOutput::new(change, change_script));
            }
            outputs
        }
        (Some((address, petals)), None) => {
            if *petals > output_value {
                return Err(Error::Custom(format!(
                    "redeemed value after fee ({output_value} petals) does not cover the requested payment ({petals} petals)"
                )));
            }
            let over = redeemed_value_petals - petals;
            if over > FEE_QUANTUM_PETALS {
                return Err(Error::Custom(format!(
                    "the notes chosen come to {}, which is {} over the {} asked for, and this wallet has no ledger to return the difference to — pick an amount your notes cover to within {}, or 'account create bip32' adds a ledger",
                    crate::utils::sompi_to_kaspa_string(redeemed_value_petals),
                    crate::utils::sompi_to_kaspa_string(over),
                    crate::utils::sompi_to_kaspa_string(*petals),
                    crate::utils::sompi_to_kaspa_string(FEE_QUANTUM_PETALS)
                )));
            }
            vec![TransactionOutput::new(output_value, pay_to_address_script(address))]
        }
        (None, None) => unreachable!("rejected above"),
    };
    let outputs_hash = transparent_outputs_hash(&outputs);

    let mut signed_groups = Vec::with_capacity(groups_by_sk.len());
    for (sk_bytes, group_serials) in groups_by_sk {
        let hash = signing_hash(REDEEM_OP_TYPE, &group_serials, &[], outputs_hash, freshness.anchor_daa_score);
        let keypair =
            Keypair::from_seckey_slice(SECP256K1, &sk_bytes).map_err(|e| Error::Custom(format!("invalid note secret key: {e}")))?;
        let msg = Message::from_digest(hash.into());
        let signature: [u8; 64] = *keypair.sign_schnorr(msg).as_ref();
        signed_groups.push(SignedGroup { serials: group_serials, signature });
    }

    let redeem_payload = PoolOp::Redeem(RedeemOp { consumed: signed_groups, freshness }).encode_payload();
    let tx = Transaction::new(TX_VERSION_TOCCATA, vec![], outputs, 0, SUBNETWORK_ID_NOTE_POOL, 0, redeem_payload);
    tx.set_storage_mass(contextual);

    let rpc_tx: kaspa_rpc_core::RpcTransaction = (&tx).into();
    let transaction_id = wallet.rpc_api().submit_transaction(rpc_tx, false).await?;

    // The redeemed serials are gone from the pool the instant this transaction
    // confirms; mark them superseded now (plaintext-only, matches how the P7.1
    // `NotesChanged` listener would eventually observe the same removal).
    for sn in &serials {
        note_key_store.mark_status(sn, NoteStatus::Superseded).await?;
    }

    Ok(RedeemResult { transaction_id, redeemed_value_petals, fee_petals, serials })
}

// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
// Receive flows (FORK-PLAN P7.3): QR/text payloads, the pure-pool transfer builder,
// bearer import, and the sign-to-fresh-pk payment-request flow.

/// The payment-request QR/text payload (POOL-SPEC.md P5.6 "QR payload formats"):
/// plaintext, no encryption — a public invitation to pay. 40 bytes with a pinned
/// amount (`pk || amount_petals u64 LE`), 32 bytes without one (the static/printed
/// form — the payer supplies the amount manually). The two forms are distinguished
/// by length alone, exactly as the spec lays them out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaymentRequest {
    pub pk: [u8; 32],
    pub amount_petals: Option<u64>,
}

/// Text-encoding prefix for payment requests. A wallet-level convention (what a QR
/// or a pasted string carries), not a consensus format.
pub const PAYMENT_REQUEST_PREFIX: &str = "marigoldreq:";
/// Text-encoding prefix for bearer-note handovers.
pub const BEARER_NOTE_PREFIX: &str = "marigoldnote:";

impl PaymentRequest {
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(40);
        bytes.extend_from_slice(&self.pk);
        if let Some(amount) = self.amount_petals {
            bytes.extend_from_slice(&amount.to_le_bytes());
        }
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        match bytes.len() {
            32 => Ok(Self { pk: bytes.try_into().unwrap(), amount_petals: None }),
            40 => {
                let pk: [u8; 32] = bytes[..32].try_into().unwrap();
                let amount = u64::from_le_bytes(bytes[32..].try_into().unwrap());
                Ok(Self { pk, amount_petals: Some(amount) })
            }
            len => Err(Error::Custom(format!("payment request must be 32 or 40 bytes, got {len}"))),
        }
    }

    pub fn to_text(&self) -> String {
        format!("{PAYMENT_REQUEST_PREFIX}{}", self.encode().to_hex())
    }

    pub fn from_text(text: &str) -> Result<Self> {
        let hex = text
            .trim()
            .strip_prefix(PAYMENT_REQUEST_PREFIX)
            .ok_or_else(|| Error::Custom(format!("payment request text must start with '{PAYMENT_REQUEST_PREFIX}'")))?;
        let bytes = Vec::<u8>::from_hex(hex).map_err(|e| Error::Custom(format!("invalid payment request hex: {e}")))?;
        Self::decode(&bytes)
    }
}

/// The bearer-note handover payload: everything the receiver needs to take custody
/// of one note — `(sn, sk, d)`, 65 bytes, deliberately the same triple the paper
/// backup stores per note (POOL-SPEC.md P5.6's backup format) rather than a third
/// invented shape. Plaintext by design: a bearer handover QR is shown briefly,
/// point-to-point, and its entire value is *supposed* to transfer to whoever scans
/// it — the mitigations are physical (show it only to the payee) and protocol-level
/// (the receiver's immediate rotation, below), not encryption.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BearerNote {
    pub sn: Hash,
    pub sk: [u8; 32],
    pub d: DenominationTag,
}

impl BearerNote {
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(65);
        bytes.extend_from_slice(&self.sn.as_bytes());
        bytes.extend_from_slice(&self.sk);
        bytes.push(self.d as u8);
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 65 {
            return Err(Error::Custom(format!("bearer note must be 65 bytes, got {}", bytes.len())));
        }
        let sn = Hash::from_slice(&bytes[..32]);
        let sk: [u8; 32] = bytes[32..64].try_into().unwrap();
        let d = DenominationTag::try_from(bytes[64]).map_err(|_| Error::Custom(format!("unknown denomination tag {}", bytes[64])))?;
        Ok(Self { sn, sk, d })
    }

    pub fn to_text(&self) -> String {
        format!("{BEARER_NOTE_PREFIX}{}", self.encode().to_hex())
    }

    pub fn from_text(text: &str) -> Result<Self> {
        let hex = text
            .trim()
            .strip_prefix(BEARER_NOTE_PREFIX)
            .ok_or_else(|| Error::Custom(format!("bearer note text must start with '{BEARER_NOTE_PREFIX}'")))?;
        let bytes = Vec::<u8>::from_hex(hex).map_err(|e| Error::Custom(format!("invalid bearer note hex: {e}")))?;
        Self::decode(&bytes)
    }
}

/// Like [`decompose_amount`] but maps zero to an empty list (a fee source consumed
/// exactly as a stamp produces no change) instead of `None`.
fn decompose_amount_allow_zero(petals: u64) -> Option<Vec<DenominationTag>> {
    if petals == 0 { Some(Vec::new()) } else { decompose_amount(petals) }
}

/// A freshly generated note key destined to become an own `NoteKeyEntry` row once
/// the transfer's transaction id (and therefore each serial) is known.
struct FreshNote {
    d: DenominationTag,
    sk: [u8; 32],
    pk: [u8; 32],
}

fn generate_fresh_notes(denominations: &[DenominationTag]) -> Vec<FreshNote> {
    denominations
        .iter()
        .map(|d| {
            let sk = SecretKey::new(&mut secp256k1::rand::thread_rng());
            let pk = Keypair::from_secret_key(SECP256K1, &sk).x_only_public_key().0.serialize();
            FreshNote { d: *d, sk: sk.secret_bytes(), pk }
        })
        .collect()
}

/// Exact-sum note selection over the fixed ladder, greedy largest-first — optimal for
/// a canonical powers-of-ten system: taking as many of the largest usable
/// denomination as possible never forecloses an exact representation that skipping
/// them would have allowed. Returns `None` when the held multiset cannot represent
/// the amount exactly (the covering planner below then takes over).
fn select_exact(available: &[Arc<NoteKeyInfo>], amount_petals: u64) -> Option<Vec<Hash>> {
    let mut by_denom: Vec<Vec<Hash>> = vec![Vec::new(); DENOMINATION_PETALS.len()];
    for info in available {
        by_denom[info.d as usize].push(info.sn);
    }
    let mut remaining = amount_petals;
    let mut selected = Vec::new();
    for (index, value) in DENOMINATION_PETALS.iter().enumerate().rev() {
        let want = (remaining / value) as usize;
        let take = want.min(by_denom[index].len());
        for sn in by_denom[index].drain(..take) {
            selected.push(sn);
        }
        remaining -= take as u64 * value;
    }
    (remaining == 0).then_some(selected)
}

/// Covering selection (FORK-PLAN P7.4's split planning): notes summing to at least
/// `target_petals`. An exact representation is preferred (no change, fewest moving
/// parts); otherwise notes accumulate smallest-first until the target is covered —
/// deliberately sweeping small denominations into the transfer's change, which the
/// change decomposition then re-issues in canonical largest-first form (organic
/// merge hygiene: paying with dust consolidates it, POOL-SPEC.md P5.6's merge
/// motivation, without a dedicated merge step). The overshoot comes back as change
/// in the same `TransferOp` — "split then pay" is one transaction, not two (P5.2's
/// `produced` list already allows it), so no separate split planning stage exists.
fn select_covering(available: &[Arc<NoteKeyInfo>], target_petals: u64) -> Option<Vec<Hash>> {
    if let Some(exact) = select_exact(available, target_petals) {
        return Some(exact);
    }
    let mut sorted: Vec<&Arc<NoteKeyInfo>> = available.iter().collect();
    sorted.sort_by_key(|info| DENOMINATION_PETALS[info.d as usize]);
    let mut total: u64 = 0;
    let mut selected = Vec::new();
    for info in sorted {
        if total >= target_petals {
            break;
        }
        total += DENOMINATION_PETALS[info.d as usize];
        selected.push(info.sn);
    }
    (total >= target_petals).then_some(selected)
}

/// Estimate the consensus mass of a transfer with the given shape. A `SignedGroup`'s
/// wire size is independent of its signature content, so placeholder signatures give
/// byte-identical payload sizes to the final ones (same trick `redeem` uses).
fn estimate_transfer_mass(
    mass_calculator: &MassCalculator,
    group_serials: &[Vec<Hash>],
    produced: &[NewNote],
    freshness: FreshnessAnchor,
) -> Result<u64> {
    let consumed: Vec<SignedGroup> =
        group_serials.iter().map(|serials| SignedGroup { serials: serials.clone(), signature: [0u8; 64] }).collect();
    let payload = PoolOp::Transfer(TransferOp { consumed, produced: produced.to_vec(), freshness }).encode_payload();
    let tx = Transaction::new(TX_VERSION_TOCCATA, vec![], vec![], 0, SUBNETWORK_ID_NOTE_POOL, 0, payload);
    let populated = PopulatedTransaction::new(&tx, vec![]);
    // Storage mass ALONE is the wrong number here, and it silently made every
    // transfer cost exactly one quantum no matter how large: a pool op has no
    // transparent outputs, so its storage mass is structurally zero, and the
    // fee fixpoint above was being fed a constant 0. Consensus charges
    // max(compute, storage), and for a pool op it is compute — payload bytes —
    // that carries the whole cost. Measured: a fifty-note transfer is 6,667
    // bytes of payload, which the node prices at more than one quantum, so the
    // wallet was underpaying and the node would refuse it.
    let contextual = mass_calculator
        .calc_contextual_masses(&populated)
        .ok_or_else(|| Error::Custom("transfer: mass calculation failed".to_string()))?
        .storage_mass;
    let non_contextual = mass_calculator.calc_non_contextual_masses(&tx);
    Ok(contextual.max(non_contextual.compute_mass).max(non_contextual.transient_mass))
}

pub struct TransferResult {
    pub transaction_id: Hash,
    /// Every serial this transfer consumed (payment/rotation notes + fee sources).
    pub consumed_serials: Vec<Hash>,
    /// Own new rows (rotation/change notes), already persisted in the note key store.
    pub own_notes: Vec<NoteKeyEntry>,
    /// Serials of notes produced to an external pk (payment notes) — not own rows.
    pub external_serials: Vec<Hash>,
    pub fee_petals: u64,
}

/// The shared pure-pool transfer engine behind [`rotate_notes`] and
/// [`pay_payment_request`]: given the notes to consume, the external payment notes
/// (possibly none) and the own-note denominations to produce, it signs one
/// `SignedGroup` per distinct key, assembles/submits the zero-transparent-part
/// transaction, persists the own rows (`serial_hash(txid, index)` with external
/// payment notes occupying the leading indices) and tombstones everything consumed.
async fn submit_transfer(
    wallet: &Arc<Wallet>,
    wallet_secret: &Secret,
    consumed_entries: &[NoteKeyEntry],
    external: &[NewNote],
    own_fresh: &[FreshNote],
    own_provenance: NoteProvenance,
    freshness: FreshnessAnchor,
    fee_petals: u64,
) -> Result<TransferResult> {
    let mut produced: Vec<NewNote> = external.to_vec();
    produced.extend(own_fresh.iter().map(|f| NewNote { d: f.d, pk: f.pk }));

    let outputs_hash = transparent_outputs_hash(&[]);
    let mut groups_by_sk: HashMap<[u8; 32], Vec<Hash>> = HashMap::new();
    for entry in consumed_entries {
        groups_by_sk.entry(entry.sk).or_default().push(entry.sn);
    }

    const TRANSFER_OP_TYPE: u8 = 1;
    let mut signed_groups = Vec::with_capacity(groups_by_sk.len());
    for (sk_bytes, group_serials) in groups_by_sk {
        let hash = signing_hash(TRANSFER_OP_TYPE, &group_serials, &produced, outputs_hash, freshness.anchor_daa_score);
        let keypair =
            Keypair::from_seckey_slice(SECP256K1, &sk_bytes).map_err(|e| Error::Custom(format!("invalid note secret key: {e}")))?;
        let signature: [u8; 64] = *keypair.sign_schnorr(Message::from_digest(hash.into())).as_ref();
        signed_groups.push(SignedGroup { serials: group_serials, signature });
    }

    let payload = PoolOp::Transfer(TransferOp { consumed: signed_groups, produced: produced.clone(), freshness }).encode_payload();
    let tx = Transaction::new(TX_VERSION_TOCCATA, vec![], vec![], 0, SUBNETWORK_ID_NOTE_POOL, 0, payload);

    let network_id = wallet.network_id()?;
    let mass_calculator = MassCalculator::new_with_consensus_params(&Params::from(network_id));
    let populated = PopulatedTransaction::new(&tx, vec![]);
    let storage_mass = mass_calculator
        .calc_contextual_masses(&populated)
        .ok_or_else(|| Error::Custom("transfer: mass calculation failed".to_string()))?
        .storage_mass;
    tx.set_storage_mass(storage_mass);

    let rpc_tx: kaspa_rpc_core::RpcTransaction = (&tx).into();
    let transaction_id = wallet.rpc_api().submit_transaction(rpc_tx, false).await?;

    let note_key_store = wallet.store().as_note_key_store()?;
    let external_serials: Vec<Hash> = (0..external.len()).map(|i| serial_hash(&transaction_id, i as u32)).collect();
    let mut own_notes = Vec::with_capacity(own_fresh.len());
    for (offset, fresh) in own_fresh.iter().enumerate() {
        let sn = serial_hash(&transaction_id, (external.len() + offset) as u32);
        let entry = NoteKeyEntry::new(sn, fresh.sk, fresh.d, own_provenance);
        note_key_store.store(wallet_secret, entry.clone()).await?;
        own_notes.push(entry);
    }
    let consumed_serials: Vec<Hash> = consumed_entries.iter().map(|e| e.sn).collect();
    for sn in &consumed_serials {
        note_key_store.mark_status(sn, NoteStatus::Superseded).await?;
    }

    Ok(TransferResult { transaction_id, consumed_serials, own_notes, external_serials, fee_petals })
}

/// Shared fee/feerate plumbing: the estimated feerate (sompi per gram) to size a
/// transfer's fee quanta against, mirroring `redeem`'s sourcing.
async fn transfer_feerate(wallet: &Arc<Wallet>) -> f64 {
    let estimated = match wallet.rpc_api().get_fee_estimate().await {
        Ok(estimate) => estimate.normal_buckets.first().map(|b| b.feerate).unwrap_or(1.0),
        Err(_) => 1.0,
    };
    // The node's fee ESTIMATE is a priority signal, typically 1 sompi/gram.
    // The mempool's minimum RELAY fee is 100 sompi/gram, and a transaction
    // below it is refused however unhurried its sender. Sizing on the estimate
    // alone underprices every pool op by a factor of a hundred; small ones
    // survived only because the 0.01 quantum floor happened to cover them.
    // Same lesson POOL_FEE_RATE already encodes for minting.
    estimated.max(POOL_FEE_RATE)
}

fn required_fee_quanta(mass: u64, feerate: f64) -> u64 {
    let required = (mass as f64 * feerate).ceil() as u64;
    required.div_ceil(FEE_QUANTUM_PETALS).max(1)
}

/// Rotate the given (owned, Active) serials to fresh keys (FORK-PLAN P7.3 — the
/// receive flow's step 3, also the future P7.4 isolation primitive). The rotated
/// value is re-produced under fresh Cold keys, preserving denominations. The fee is
/// sourced from the wallet's smallest spare notes (consumed alongside, their excess
/// returned as change) — or, when the wallet holds nothing else, withheld from the
/// rotated value itself ("slack mode": the produced decomposition simply comes out
/// one fee-quantum short, the bootstrap case for a first-ever bearer receive).
pub async fn rotate_notes(wallet: &Arc<Wallet>, wallet_secret: Secret, serials: Vec<Hash>) -> Result<TransferResult> {
    if serials.is_empty() {
        return Err(Error::Custom("no serials given to rotate".to_string()));
    }
    let note_key_store = wallet.store().as_note_key_store()?;

    let mut rotate_entries = Vec::with_capacity(serials.len());
    for sn in &serials {
        let entry =
            note_key_store.load_key(&wallet_secret, sn).await?.ok_or_else(|| Error::Custom(format!("serial {sn} has no stored key")))?;
        rotate_entries.push(entry);
    }
    let rotate_value: u64 = rotate_entries.iter().map(|e| DENOMINATION_PETALS[e.d as usize]).sum();

    // Spare (fee-source) candidates: every Active note not being rotated,
    // smallest first — and confirmed present in the pool, for the same reason
    // the rotated group must be. A fee sourced from a note whose creating
    // transaction has not landed yet fails validation exactly as a consumed
    // one does, and the rejection names the spare's serial rather than
    // anything the caller chose, which makes it needlessly baffling.
    let mut spares: Vec<Arc<NoteKeyInfo>> = note_key_store
        .iter()
        .await?
        .try_filter(|info| futures::future::ready(info.status == NoteStatus::Active && !serials.contains(&info.sn)))
        .try_collect()
        .await?;
    let spare_confirmed = pool_confirmed(wallet, spares.iter().map(|info| info.sn).collect()).await?;
    spares.retain(|info| spare_confirmed.contains(&info.sn));
    spares.sort_by_key(|info| DENOMINATION_PETALS[info.d as usize]);

    let network_id = wallet.network_id()?;
    let mass_calculator = MassCalculator::new_with_consensus_params(&Params::from(network_id));
    let server_info = wallet.rpc_api().get_server_info().await?;
    let freshness = FreshnessAnchor { anchor_daa_score: server_info.virtual_daa_score };
    let feerate = transfer_feerate(wallet).await;

    let mut fee_quanta: u64 = 1;
    // Bounded fixpoint: the fee affects the change/produced shape, which affects
    // mass, which affects the fee. Each iteration only ever raises `fee_quanta`, and
    // one quantum covers several thousand grams at any sane feerate, so this
    // converges immediately in practice; the bound is a defensive backstop.
    for _ in 0..8 {
        let fee_petals = fee_quanta * FEE_QUANTUM_PETALS;

        // Stamp mode: pull spares (smallest first) until they cover the fee.
        let mut source_infos: Vec<Arc<NoteKeyInfo>> = Vec::new();
        let mut source_total: u64 = 0;
        for spare in &spares {
            if source_total >= fee_petals {
                break;
            }
            source_total += DENOMINATION_PETALS[spare.d as usize];
            source_infos.push(spare.clone());
        }

        // Refuse to shred a large note to pay a small fee. Spares are taken
        // smallest-first, but "smallest" is smallest *confirmed in the pool* —
        // and on a wallet minting every minute the fresh 0.01 stamps are still
        // unconfirmed, so the selector can reach past fifty of them to a 10
        // MAGLD note and spend it on a 0.01 fee (founder report, 2026-09-07:
        // two 10s gone, one stamp gone, nothing back). Housekeeping that can
        // wait should wait.
        if source_total >= fee_petals && source_total > fee_petals.saturating_mul(FEE_SOURCE_MAX_OVERSHOOT) {
            return Err(Error::Custom(format!(
                "no small note is confirmed yet to pay the {} petal fee — the smallest available is {} petals; \
                 retry once the fee stamps have landed",
                fee_petals, source_total
            )));
        }

        let (produced_denoms, consumed_serial_count) = if source_total >= fee_petals {
            let mut denoms = decompose_amount(rotate_value).expect("rotate value is a sum of denominations");
            denoms.extend(decompose_amount_allow_zero(source_total - fee_petals).expect("change is denomination-quantized"));
            (denoms, serials.len() + source_infos.len())
        } else {
            // Slack mode: no (sufficient) spares — the fee comes out of the rotated
            // value itself.
            if rotate_value <= fee_petals {
                return Err(Error::Custom(format!(
                    "rotating these note(s) ({rotate_value} petals) would burn them entirely as fee ({fee_petals} petals) — \
                     the wallet holds no spare note to fund the fee with"
                )));
            }
            (decompose_amount(rotate_value - fee_petals).expect("slack remainder is denomination-quantized"), serials.len())
        };

        // Estimate mass for this candidate shape (worst-case grouping: every consumed
        // entry under its own key — placeholder serials are fine, only counts matter).
        let placeholder_groups: Vec<Vec<Hash>> = (0..consumed_serial_count).map(|i| vec![Hash::from_u64_word(i as u64)]).collect();
        let placeholder_produced: Vec<NewNote> = produced_denoms.iter().map(|d| NewNote { d: *d, pk: [0u8; 32] }).collect();
        let mass = estimate_transfer_mass(&mass_calculator, &placeholder_groups, &placeholder_produced, freshness)?;
        let required = required_fee_quanta(mass, feerate);
        if required > fee_quanta {
            fee_quanta = required;
            continue;
        }

        // Shape settled — build for real.
        let mut consumed_entries = rotate_entries.clone();
        for info in &source_infos {
            let entry = note_key_store
                .load_key(&wallet_secret, &info.sn)
                .await?
                .ok_or_else(|| Error::Custom(format!("fee-source serial {} has no stored key", info.sn)))?;
            consumed_entries.push(entry);
        }
        let own_fresh = generate_fresh_notes(&produced_denoms);
        return submit_transfer(
            wallet,
            &wallet_secret,
            &consumed_entries,
            &[],
            &own_fresh,
            NoteProvenance::Cold,
            freshness,
            fee_quanta * FEE_QUANTUM_PETALS,
        )
        .await;
    }
    Err(Error::Custom("transfer fee sizing did not converge".to_string()))
}

pub struct BearerImportResult {
    pub imported_sn: Hash,
    pub rotation: TransferResult,
}

/// Bearer-note import (FORK-PLAN P7.3 flow (a), POOL-SPEC.md P5.5a/P5.6): verify the
/// serial's current on-chain `pk` matches the handed-over key, store it — Hot,
/// structurally (P7.1's `import_bearer_key` accepts no other provenance) — and
/// **immediately** rotate it to a fresh Cold key. The note is not considered
/// received until that rotation confirms; the caller reports confirmation by
/// watching the rotation's own serials.
pub async fn bearer_import(wallet: &Arc<Wallet>, wallet_secret: Secret, bearer: BearerNote) -> Result<BearerImportResult> {
    let secret_key = SecretKey::from_slice(&bearer.sk).map_err(|e| Error::Custom(format!("invalid bearer secret key: {e}")))?;
    let derived_pk = Keypair::from_secret_key(SECP256K1, &secret_key).x_only_public_key().0.serialize();

    // Verify against live pool state before touching the wallet: the serial must
    // exist, still be owned by exactly this key, and carry the claimed denomination.
    let on_chain = wallet.rpc_api().get_notes_by_serial(vec![bearer.sn]).await?;
    let entry = on_chain
        .iter()
        .find(|entry| entry.sn == bearer.sn)
        .ok_or_else(|| Error::Custom(format!("note {} does not exist in the pool (already spent, or never existed)", bearer.sn)))?;
    if entry.pk != derived_pk {
        return Err(Error::Custom(format!(
            "note {} is not currently owned by the handed-over key — it was already rotated away (spent) by someone else",
            bearer.sn
        )));
    }
    if entry.denomination != bearer.d as u8 {
        return Err(Error::Custom(format!(
            "note {} denomination mismatch: payload claims tag {}, chain says {}",
            bearer.sn, bearer.d as u8, entry.denomination
        )));
    }

    let note_key_store = wallet.store().as_note_key_store()?;
    note_key_store.import_bearer_key(&wallet_secret, bearer.sn, bearer.sk, bearer.d).await?;

    // The hot-key rule (POOL-SPEC.md P5.6): rotate immediately, not lazily.
    let rotation = rotate_notes(wallet, wallet_secret, vec![bearer.sn]).await?;
    Ok(BearerImportResult { imported_sn: bearer.sn, rotation })
}

/// Create (and persist, before anything is displayed) a payment request
/// (FORK-PLAN P7.3 flow (b), POOL-SPEC.md P5.5b "sign-to-fresh-pk").
pub async fn create_payment_request(wallet: &Arc<Wallet>, wallet_secret: &Secret, amount_petals: Option<u64>) -> Result<PaymentRequest> {
    if let Some(amount) = amount_petals {
        decompose_amount(amount).ok_or_else(|| {
            Error::Custom(format!("{amount} petals is not representable in the denomination ladder (must be a nonzero multiple of 0.01 MAGLD)"))
        })?;
    }
    let sk = SecretKey::new(&mut secp256k1::rand::thread_rng());
    let key = PaymentRequestKey::new(sk.secret_bytes(), amount_petals);
    let info = wallet.store().as_note_key_store()?.store_payment_request(wallet_secret, key).await?;
    Ok(PaymentRequest { pk: info.pk, amount_petals })
}

/// Pay a payment request (the payer's half of sign-to-fresh-pk, upgraded by
/// FORK-PLAN P7.4 with split planning): select own notes covering `amount + fee`
/// (exact if the held multiset allows, else a covering superset — see
/// [`select_covering`]), produce `decompose(amount)` under the request's `pk` and
/// the overshoot minus the fee as change to own fresh Cold keys — payment, split,
/// change, and fee in one `TransferOp` (POOL-SPEC.md P5.6: "'split then pay' is one
/// `TransferOp`, not two sequential ones").
pub async fn pay_payment_request(
    wallet: &Arc<Wallet>,
    wallet_secret: Secret,
    request: PaymentRequest,
    amount_override: Option<u64>,
) -> Result<TransferResult> {
    let amount = request
        .amount_petals
        .or(amount_override)
        .ok_or_else(|| Error::Custom("this payment request pins no amount — one must be supplied".to_string()))?;
    let payment_denoms = decompose_amount(amount)
        .ok_or_else(|| Error::Custom(format!("{amount} petals is not representable (must be a nonzero multiple of 0.01 MAGLD)")))?;
    let external: Vec<NewNote> = payment_denoms.iter().map(|d| NewNote { d: *d, pk: request.pk }).collect();

    let note_key_store = wallet.store().as_note_key_store()?;
    let active: Vec<Arc<NoteKeyInfo>> = note_key_store
        .iter()
        .await?
        .try_filter(|info| futures::future::ready(info.status == NoteStatus::Active))
        .try_collect()
        .await?;
    let held_total: u64 = active.iter().map(|info| DENOMINATION_PETALS[info.d as usize]).sum();

    let network_id = wallet.network_id()?;
    let mass_calculator = MassCalculator::new_with_consensus_params(&Params::from(network_id));
    let server_info = wallet.rpc_api().get_server_info().await?;
    let freshness = FreshnessAnchor { anchor_daa_score: server_info.virtual_daa_score };
    let feerate = transfer_feerate(wallet).await;

    let mut fee_quanta: u64 = 1;
    for _ in 0..8 {
        let fee_petals = fee_quanta * FEE_QUANTUM_PETALS;
        let target = amount + fee_petals;
        let selection = select_covering(&active, target).ok_or_else(|| {
            Error::Custom(format!(
                "insufficient note balance: {held_total} petals held, {target} needed ({amount} payment + {fee_petals} fee)"
            ))
        })?;
        let selected_total: u64 = selection
            .iter()
            .map(|sn| {
                let info = active.iter().find(|info| info.sn == *sn).expect("selection comes from the active set");
                DENOMINATION_PETALS[info.d as usize]
            })
            .sum();
        let change_denoms = decompose_amount_allow_zero(selected_total - target).expect("change is denomination-quantized");

        let placeholder_groups: Vec<Vec<Hash>> = (0..selection.len()).map(|i| vec![Hash::from_u64_word(i as u64)]).collect();
        let mut placeholder_produced = external.clone();
        placeholder_produced.extend(change_denoms.iter().map(|d| NewNote { d: *d, pk: [0u8; 32] }));
        let mass = estimate_transfer_mass(&mass_calculator, &placeholder_groups, &placeholder_produced, freshness)?;
        let required = required_fee_quanta(mass, feerate);
        if required > fee_quanta {
            fee_quanta = required;
            continue;
        }

        let mut consumed_entries = Vec::with_capacity(selection.len());
        for sn in &selection {
            let entry = note_key_store
                .load_key(&wallet_secret, sn)
                .await?
                .ok_or_else(|| Error::Custom(format!("serial {sn} has no stored key")))?;
            consumed_entries.push(entry);
        }
        let own_fresh = generate_fresh_notes(&change_denoms);
        return submit_transfer(
            wallet,
            &wallet_secret,
            &consumed_entries,
            &external,
            &own_fresh,
            NoteProvenance::Cold,
            freshness,
            fee_quanta * FEE_QUANTUM_PETALS,
        )
        .await;
    }
    Err(Error::Custom("transfer fee sizing did not converge".to_string()))
}

pub struct BearerExportResult {
    pub bearer: BearerNote,
    /// The isolation rotation, when the note's key wasn't solo — `None` means the
    /// note was already on a solo Cold key and was handed over directly. When
    /// `Some`, the exported `bearer` refers to the freshly isolated serial, which
    /// only becomes verifiable by the receiver once this transaction confirms.
    pub isolation: Option<TransferResult>,
}

/// Bearer-export a note (FORK-PLAN P7.4 flow (b), POOL-SPEC.md P5.6's one wallet
/// invariant: **bearer handover requires a solo key** — revealing a shared `sk`
/// hands over every note under that `pk`, not just the one being paid). If the
/// note's key is shared (any other live row under the same `pk` — the
/// landing-pad/POS case) or `Hot` (its history may include a wallet we don't
/// control), it is first auto-isolated onto a fresh solo Cold key via
/// [`rotate_notes`]; the exported payload then carries the isolated serial. The
/// handed-over row is marked [`NoteStatus::HandedOver`] — excluded from balance and
/// selection, flipping to `Superseded` when the receiver's rotation is observed.
pub async fn bearer_export(wallet: &Arc<Wallet>, wallet_secret: Secret, sn: Hash) -> Result<BearerExportResult> {
    let note_key_store = wallet.store().as_note_key_store()?;
    let info = note_key_store.load_info(&sn).await?.ok_or_else(|| Error::Custom(format!("serial {sn} is not in the note key database")))?;
    if info.status != NoteStatus::Active {
        return Err(Error::Custom(format!("serial {sn} is not active ({:?})", info.status)));
    }
    let entry =
        note_key_store.load_key(&wallet_secret, &sn).await?.ok_or_else(|| Error::Custom(format!("serial {sn} has no stored key")))?;

    // Solo check, on plaintext info alone (same sk ⇔ same pk): any other row still
    // live under this pk means the key is shared; Hot provenance means some other
    // wallet may hold it even with no sibling row here.
    let mut shared = info.provenance == NoteProvenance::Hot;
    if !shared {
        let siblings: Vec<Arc<NoteKeyInfo>> = note_key_store
            .iter()
            .await?
            .try_filter(|other| {
                futures::future::ready(other.sn != sn && other.pk == info.pk && other.status != NoteStatus::Superseded)
            })
            .try_collect()
            .await?;
        shared = !siblings.is_empty();
    }

    if !shared {
        note_key_store.mark_status(&sn, NoteStatus::HandedOver).await?;
        return Ok(BearerExportResult { bearer: BearerNote { sn, sk: entry.sk, d: entry.d }, isolation: None });
    }

    let rotation = rotate_notes(wallet, wallet_secret, vec![sn]).await?;
    let isolated = rotation
        .own_notes
        .iter()
        .find(|note| note.d == entry.d)
        .ok_or_else(|| {
            Error::Custom(format!(
                "isolation could not preserve the note's denomination (no spare fee note — the rotation ran in slack mode and \
                 split the value; the rotated notes are safely yours, re-run the export against one of: {})",
                rotation.own_notes.iter().map(|n| n.sn.to_string()).collect::<Vec<_>>().join(", ")
            ))
        })?
        .clone();
    note_key_store.mark_status(&isolated.sn, NoteStatus::HandedOver).await?;
    Ok(BearerExportResult {
        bearer: BearerNote { sn: isolated.sn, sk: isolated.sk, d: isolated.d },
        isolation: Some(rotation),
    })
}

pub struct ClaimedPayment {
    pub notes: Vec<NoteKeyEntry>,
    pub total_petals: u64,
}

/// Await a payment against an outstanding request (the receiver's half of
/// sign-to-fresh-pk): subscribe to `NotesChanged` for the request's `pk`
/// (POOL-SPEC.md P5.6's "the wallet was watching that `pk` since generating it" —
/// notifications fire on *confirmation*, when virtual's pool state changes, so
/// arrival here IS settlement, not mempool presence), claim the landed serials into
/// the note key store as Cold rows, and retire the request. Cold, not Hot: the
/// request key was generated locally and only its *pk* ever left the wallet.
pub async fn await_payment_request(
    wallet: &Arc<Wallet>,
    wallet_secret: &Secret,
    pk: [u8; 32],
    timeout: Duration,
) -> Result<ClaimedPayment> {
    let note_key_store = wallet.store().as_note_key_store()?;
    let request_key = note_key_store
        .load_payment_request_key(wallet_secret, &pk)
        .await?
        .ok_or_else(|| Error::Custom("no outstanding payment request for this pk".to_string()))?;
    let target = request_key.amount_petals;

    let rpc = wallet.rpc_api();
    let notification_channel = Channel::<kaspa_rpc_core::Notification>::unbounded();
    let listener_id = rpc.register_new_listener(ChannelConnection::new(
        "notepool-await-payment",
        notification_channel.sender.clone(),
        ChannelType::Closable,
    ));
    rpc.start_notify(listener_id, Scope::NotesChanged(NotesChangedScope::new(vec![], vec![pk]))).await?;

    let mut landed: Vec<(Hash, DenominationTag)> = Vec::new();
    let mut total: u64 = 0;
    let mut timeout_fut = Box::pin(workflow_core::task::sleep(timeout).fuse());
    let outcome = loop {
        futures::select! {
            notification = notification_channel.receiver.recv().fuse() => {
                match notification {
                    Ok(kaspa_rpc_core::Notification::NotesChanged(notification)) => {
                        for entry in notification.added.iter().filter(|entry| entry.pk == pk) {
                            let d = DenominationTag::try_from(entry.denomination)
                                .map_err(|_| Error::Custom(format!("unknown denomination tag {}", entry.denomination)))?;
                            if !landed.iter().any(|(sn, _)| *sn == entry.sn) {
                                landed.push((entry.sn, d));
                                total += DENOMINATION_PETALS[d as usize];
                            }
                        }
                        match target {
                            Some(amount) if total >= amount => break Ok(()),
                            None if !landed.is_empty() => break Ok(()),
                            _ => {}
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break Err(Error::Custom("notification channel closed while awaiting payment".to_string())),
                }
            }
            _ = timeout_fut => {
                break Err(Error::Custom(format!(
                    "timed out awaiting payment ({} petals of {} arrived) — the request remains active",
                    total,
                    target.map(|t| t.to_string()).unwrap_or_else(|| "unpinned".to_string())
                )));
            }
        }
    };
    let _ = rpc.unregister_listener(listener_id).await;
    outcome?;

    let mut notes = Vec::with_capacity(landed.len());
    for (sn, d) in landed {
        let entry = NoteKeyEntry::new(sn, request_key.sk, d, NoteProvenance::Cold);
        note_key_store.store(wallet_secret, entry.clone()).await?;
        notes.push(entry);
    }
    note_key_store.remove_payment_request(wallet_secret, &pk).await?;
    Ok(ClaimedPayment { notes, total_petals: total })
}

// ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
// POS landing-pad mode (FORK-PLAN P7.5, POOL-SPEC.md P5.6 "POS 'landing pad' flow").

/// Callback type for [`pos_checkout`]'s `on_request` hook — pulled out as a named
/// alias (rather than inlined in each signature) so the anonymous lifetime in
/// `&PaymentRequest` resolves identically wherever the type is used, including
/// across the `#[async_trait]`-generated `Account::pos_checkout` signature.
pub type PosCheckoutRequestHook = Box<dyn FnOnce(&PaymentRequest) + Send>;

pub struct PosCheckoutResult {
    pub request: PaymentRequest,
    pub claimed: ClaimedPayment,
    /// The immediate sweep off the landing-pad `pk` — one `TransferOp`, one
    /// `SignedGroup` (every claimed note shares the checkout key), landing each
    /// output on its own fresh Cold key.
    pub sweep: TransferResult,
}

/// One POS sale (FORK-PLAN P7.5's primary, spec-recommended mode — "fresh `pk` per
/// checkout... gives free payment matching," POOL-SPEC.md P5.6): create a
/// single-use payment request (fresh `pk`, matching one sale to one confirmed
/// rotation), wait for the exact payment, and the instant it confirms, immediately
/// sweep every landed note off the shared checkout `pk` onto its own fresh Cold key
/// in the same call — "sweep per confirmation, not end-of-day" (P5.6): the
/// shared-key exposure window is bounded to this function's own latency, not a
/// business day. The static day-`pk` fallback for printed/no-register QR codes
/// (P5.6, secondary — no live confirmation loop, no per-sale amount) is deliberately
/// not built here; see NOTES.md's P7.5 entry for the scoping call.
///
/// Sweeps by *value*, not by strict per-note identity: the claimed notes' total is
/// re-decomposed into the canonical denomination ladder (P7.3's `rotate_notes`),
/// which may consolidate differently-denominated inputs into a different shape at
/// the same total value — "every note lands under its own key" (P5.6's phrasing)
/// is satisfied either way (nothing stays shared), and canonical reshaping also
/// opportunistically consolidates a merchant's accumulating dust for free.
///
/// `on_request` fires the moment the checkout `pk` exists (before the wait begins)
/// so a caller can display it — the request must be shown to the customer before
/// anything can be paid, but `pos_checkout` only *returns* once the whole sale
/// (payment + sweep) is done.
pub async fn pos_checkout(
    wallet: &Arc<Wallet>,
    wallet_secret: Secret,
    amount_petals: u64,
    timeout: Duration,
    on_request: Option<PosCheckoutRequestHook>,
) -> Result<PosCheckoutResult> {
    let request = create_payment_request(wallet, &wallet_secret, Some(amount_petals)).await?;
    if let Some(on_request) = on_request {
        on_request(&request);
    }
    let claimed = await_payment_request(wallet, &wallet_secret, request.pk, timeout).await?;
    let serials: Vec<Hash> = claimed.notes.iter().map(|n| n.sn).collect();
    let sweep = rotate_notes(wallet, wallet_secret, serials).await?;
    Ok(PosCheckoutResult { request, claimed, sweep })
}

// ~~~ FORK-PLAN P7.6: vault verify, restore-rotation planning, paper QR export ~~~
//
// DECISIONS.md's "Note vault, backup, and restore-rotation policy" / POOL-SPEC.md
// P5.6's "Restore flow" section, implemented here rather than in
// `storage::local::notevault` since every function below needs live chain state
// (`get_notes_by_serial`) via `Account`/`Wallet`, not just storage — the vault
// itself stays a pure storage primitive, reachable only through the existing
// `NoteKeyStore` trait (`iter`/`load_key`), exactly like every other function in
// this module.

/// Result of [`light_verify`]: which of the wallet's believed-`Active` serials the
/// live pool still confirms as unspent under the expected `(pk, denomination)`.
/// Needs no secret at all — a serial's `(denomination, pk)` binding never changes
/// during its life (POOL-SPEC.md P5.6), so this alone distinguishes "still mine"
/// from "already spent/rotated away" without decrypting anything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LightVerifyReport {
    pub live: Vec<Hash>,
    pub stale: Vec<Hash>,
}

/// Check every `Active` row's claimed `(pk, d)` against live `PoolState` — the
/// "confirm a backup's health without restoring" capability from DECISIONS.md.
/// Reads only the plaintext `NoteKeyInfo` index (`iter()`); never touches `sk`.
pub async fn light_verify(wallet: &Arc<Wallet>) -> Result<LightVerifyReport> {
    let note_key_store = wallet.store().as_note_key_store()?;
    let active: Vec<Arc<NoteKeyInfo>> = note_key_store
        .iter()
        .await?
        .try_filter(|info| futures::future::ready(info.status == NoteStatus::Active))
        .try_collect()
        .await?;
    if active.is_empty() {
        return Ok(LightVerifyReport::default());
    }

    let serials: Vec<Hash> = active.iter().map(|info| info.sn).collect();
    let on_chain = wallet.rpc_api().get_notes_by_serial(serials).await?;
    let mut report = LightVerifyReport::default();
    for info in &active {
        match on_chain.iter().find(|entry| entry.sn == info.sn) {
            Some(entry) if entry.pk == info.pk && entry.denomination == info.d as u8 => report.live.push(info.sn),
            _ => report.stale.push(info.sn),
        }
    }
    Ok(report)
}

/// Result of [`deep_verify`]: like [`LightVerifyReport`], but additionally catches
/// entries whose stored ciphertext (or plaintext index) is internally inconsistent
/// — corruption light verify's keyless check cannot see.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeepVerifyReport {
    pub live: Vec<Hash>,
    pub stale: Vec<Hash>,
    pub corrupted: Vec<Hash>,
}

/// Decrypt every `Active` row and re-derive its `pk` fresh from `sk`, comparing
/// against both the stored index (catches corruption) and the live pool (catches
/// spent-elsewhere) — POOL-SPEC.md P5.6's mandatory first step of an actual
/// restore. One `get_notes_by_serial` call total, not one per note.
pub async fn deep_verify(wallet: &Arc<Wallet>, wallet_secret: Secret) -> Result<DeepVerifyReport> {
    let note_key_store = wallet.store().as_note_key_store()?;
    let active: Vec<Arc<NoteKeyInfo>> = note_key_store
        .iter()
        .await?
        .try_filter(|info| futures::future::ready(info.status == NoteStatus::Active))
        .try_collect()
        .await?;
    let mut report = DeepVerifyReport::default();
    if active.is_empty() {
        return Ok(report);
    }

    let serials: Vec<Hash> = active.iter().map(|info| info.sn).collect();
    let on_chain = wallet.rpc_api().get_notes_by_serial(serials).await?;

    for info in &active {
        // A decrypt failure (corrupted ciphertext) surfaces as `Err`, not `Ok(None)`
        // — deep verify's entire purpose is to catch exactly this, so it must be
        // classified as `corrupted` here rather than aborting the whole call via `?`.
        let entry = match note_key_store.load_key(&wallet_secret, &info.sn).await {
            Ok(Some(entry)) => entry,
            Ok(None) | Err(_) => {
                report.corrupted.push(info.sn);
                continue;
            }
        };
        let derived_pk = match entry.derive_pk() {
            Ok(pk) => pk,
            Err(_) => {
                report.corrupted.push(info.sn);
                continue;
            }
        };
        if derived_pk != info.pk {
            report.corrupted.push(info.sn);
            continue;
        }
        match on_chain.iter().find(|chain_entry| chain_entry.sn == info.sn) {
            Some(chain_entry) if chain_entry.pk == derived_pk => report.live.push(info.sn),
            _ => report.stale.push(info.sn),
        }
    }
    Ok(report)
}

/// Like [`light_verify`], but against an arbitrary standalone vault directory —
/// e.g. a `note vault backup` copy — rather than the currently open wallet: "the
/// possibility of someone just checking their backups against the manifest
/// without actually restoring" (DECISIONS.md). Needs an RPC handle but no
/// `Account`/open wallet at all.
pub async fn light_verify_vault(
    vault: &crate::storage::local::notevault::NoteVault,
    rpc: &Arc<DynRpcApi>,
) -> Result<LightVerifyReport> {
    let active: Vec<Arc<NoteKeyInfo>> = vault
        .iter()
        .await?
        .try_filter(|info| futures::future::ready(info.status == NoteStatus::Active))
        .try_collect()
        .await?;
    if active.is_empty() {
        return Ok(LightVerifyReport::default());
    }

    let serials: Vec<Hash> = active.iter().map(|info| info.sn).collect();
    let on_chain = rpc.get_notes_by_serial(serials).await?;
    let mut report = LightVerifyReport::default();
    for info in &active {
        match on_chain.iter().find(|entry| entry.sn == info.sn) {
            Some(entry) if entry.pk == info.pk && entry.denomination == info.d as u8 => report.live.push(info.sn),
            _ => report.stale.push(info.sn),
        }
    }
    Ok(report)
}

/// Every `Active` entry's full `(sn, sk, d, provenance)`, decrypted — the input
/// [`paper_export_encode`] needs. Kept as its own function rather than inlined at
/// call sites since it's the one place that legitimately holds every held `sk` in
/// memory at once (paper export's whole point); everywhere else in this module
/// touches only the notes a single operation actually selects.
pub async fn export_active_entries(wallet: &Arc<Wallet>, wallet_secret: Secret) -> Result<Vec<NoteKeyEntry>> {
    let note_key_store = wallet.store().as_note_key_store()?;
    let active: Vec<Arc<NoteKeyInfo>> = note_key_store
        .iter()
        .await?
        .try_filter(|info| futures::future::ready(info.status == NoteStatus::Active))
        .try_collect()
        .await?;
    let mut entries = Vec::with_capacity(active.len());
    for info in &active {
        let entry = note_key_store
            .load_key(&wallet_secret, &info.sn)
            .await?
            .ok_or_else(|| Error::Custom(format!("serial {} has no stored key", info.sn)))?;
        entries.push(entry);
    }
    Ok(entries)
}

/// Split `serials` into 2-5 randomly-composed batches (POOL-SPEC.md P5.6's
/// restore-time rotation policy) — shuffled first, then dealt round-robin, so
/// batch membership carries no value information (never sorted by denomination,
/// which would leak structure the mixing exists to hide). A single serial (or
/// fewer than two total) can't usefully split — returns one batch. Pure/sync: the
/// caller (CLI) owns actually spacing the batches out in wall-clock time and
/// getting the user's per-batch confirmation, since a library call has no business
/// blocking on either.
pub fn plan_restore_rotation(serials: Vec<Hash>) -> Vec<Vec<Hash>> {
    if serials.is_empty() {
        return vec![];
    }
    let mut serials = serials;
    let mut rng = rand::thread_rng();
    serials.shuffle(&mut rng);

    let max_batches = serials.len().min(5);
    let batch_count = if max_batches < 2 { 1 } else { rng.gen_range(2..=max_batches) };
    let mut batches: Vec<Vec<Hash>> = vec![Vec::new(); batch_count];
    for (i, sn) in serials.into_iter().enumerate() {
        batches[i % batch_count].push(sn);
    }
    batches
}

/// Plaintext page header preceding a paper QR's encrypted payload (POOL-SPEC.md
/// P5.6's "QR payload formats" — 13 bytes on the wire; readable without the paper
/// export's password, so a multi-page restore can detect missing pages up front).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QrPageHeader {
    /// Random per export session — groups a multi-page backup's pages together.
    pub backup_id: [u8; 8],
    /// This page's 0-based index.
    pub chunk_index: u16,
    /// Total pages in this backup.
    pub chunk_count: u16,
    pub format_version: u8,
}

pub const QR_PAGE_FORMAT_VERSION: u8 = 1;
/// ~40 `(serial, sk, d)` entries (65 bytes each) fit comfortably under a QR code's
/// practical capacity at a scannable error-correction level once borsh-equivalent
/// framing and XChaCha20Poly1305 overhead (24-byte nonce + 16-byte tag) are added
/// (POOL-SPEC.md P5.6).
pub const QR_CHUNK_MAX_ENTRIES: usize = 40;

const QR_HEADER_LEN: usize = 8 + 2 + 2 + 1;

impl QrPageHeader {
    pub fn encode(&self) -> [u8; QR_HEADER_LEN] {
        let mut bytes = [0u8; QR_HEADER_LEN];
        bytes[0..8].copy_from_slice(&self.backup_id);
        bytes[8..10].copy_from_slice(&self.chunk_index.to_le_bytes());
        bytes[10..12].copy_from_slice(&self.chunk_count.to_le_bytes());
        bytes[12] = self.format_version;
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < QR_HEADER_LEN {
            return Err(Error::Custom(format!("QR page header must be at least {QR_HEADER_LEN} bytes, got {}", bytes.len())));
        }
        let backup_id: [u8; 8] = bytes[0..8].try_into().unwrap();
        let chunk_index = u16::from_le_bytes(bytes[8..10].try_into().unwrap());
        let chunk_count = u16::from_le_bytes(bytes[10..12].try_into().unwrap());
        let format_version = bytes[12];
        if format_version != QR_PAGE_FORMAT_VERSION {
            return Err(Error::Custom(format!("unsupported paper backup format version {format_version}")));
        }
        Ok(Self { backup_id, chunk_index, chunk_count, format_version })
    }
}

/// One printable page: `QrPageHeader::encode()` `||` `encrypt_xchacha20poly1305(borsh-equivalent
/// entry list, password)`. Whatever renders the QR (the CLI's existing `qrcode`
/// usage from P7.3) takes this blob directly.
pub fn paper_export_encode(entries: &[NoteKeyEntry], password: &Secret) -> Result<Vec<Vec<u8>>> {
    let mut backup_id = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut backup_id);

    let bearer_notes: Vec<BearerNote> = entries.iter().map(|e| BearerNote { sn: e.sn, sk: e.sk, d: e.d }).collect();
    let chunks: Vec<&[BearerNote]> =
        if bearer_notes.is_empty() { vec![&[]] } else { bearer_notes.chunks(QR_CHUNK_MAX_ENTRIES).collect() };
    let chunk_count = chunks.len() as u16;

    let mut pages = Vec::with_capacity(chunks.len());
    for (i, chunk) in chunks.iter().enumerate() {
        let header = QrPageHeader { backup_id, chunk_index: i as u16, chunk_count, format_version: QR_PAGE_FORMAT_VERSION };
        let mut plaintext = Vec::with_capacity(2 + chunk.len() * 65);
        plaintext.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
        for entry in *chunk {
            plaintext.extend_from_slice(&entry.encode());
        }
        // Salted, like the vault key and the wallet file. This used the old
        // path whose Argon2 salt was sha256(password) — deterministic, so one
        // precomputation attacks every paper backup ever made under a given
        // password. A paper backup is a page someone keeps in a drawer for
        // years; it is the last thing that should have the weakest wrapping.
        let ciphertext = crate::encryption::encrypt_salted(&plaintext, password)?;

        let mut page = header.encode().to_vec();
        page.extend_from_slice(&ciphertext);
        pages.push(page);
    }
    Ok(pages)
}

/// Encrypted pages of the given notes, base64 for text transport.
///
/// Same format as the paper backup — encrypted, chunked, each page carrying a
/// backup id and its index — because it is the same problem: a set of note
/// keys that has to survive somewhere other than this machine. Reusing it means
/// one format to get right rather than two.
///
/// Sized for a phone's storage: forty notes a page comes to about 3,600
/// characters encoded, inside Telegram CloudStorage's 4,096-character limit
/// with room to spare.
pub fn mirror_export_pages(entries: &[NoteKeyEntry], password: &Secret) -> Result<Vec<String>> {
    use base64::Engine;
    Ok(paper_export_encode(entries, password)?
        .into_iter()
        .map(|page| base64::engine::general_purpose::STANDARD.encode(page))
        .collect())
}

/// Read back one page produced by [`mirror_export_pages`].
pub fn mirror_import_page(page: &str, password: &Secret) -> Result<(QrPageHeader, Vec<BearerNote>)> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(page.trim())
        .map_err(|err| Error::Custom(format!("that does not look like a mirror page: {err}")))?;
    paper_export_decode_page(&bytes, password)
}

/// Peek a page's header without the password — enough to detect a missing page in
/// a multi-page restore before ever asking for the password.
pub fn paper_export_peek_header(page: &[u8]) -> Result<QrPageHeader> {
    QrPageHeader::decode(page)
}

/// Decrypt one page, returning its header and the `(sn, sk, d)` entries it carries.
pub fn paper_export_decode_page(page: &[u8], password: &Secret) -> Result<(QrPageHeader, Vec<BearerNote>)> {
    let header = QrPageHeader::decode(page)?;
    let ciphertext = &page[QR_HEADER_LEN..];
    // Reads both containers: pages printed before this change still decode.
    let (plaintext, _legacy) = crate::encryption::decrypt_salted_or_legacy(ciphertext, password)?;
    let plaintext = plaintext.as_ref();
    if plaintext.len() < 2 {
        return Err(Error::Custom("paper backup page payload is too short".to_string()));
    }
    let count = u16::from_le_bytes(plaintext[0..2].try_into().unwrap()) as usize;
    let mut entries = Vec::with_capacity(count);
    let mut offset = 2;
    for _ in 0..count {
        if plaintext.len() < offset + 65 {
            return Err(Error::Custom("paper backup page payload truncated mid-entry".to_string()));
        }
        entries.push(BearerNote::decode(&plaintext[offset..offset + 65])?);
        offset += 65;
    }
    Ok((header, entries))
}

/// Given the headers seen so far (from [`paper_export_peek_header`] on every page
/// in hand), report which 0-based page indices are still missing — works before
/// the password is ever entered.
pub fn paper_export_missing_pages(headers: &[QrPageHeader]) -> Vec<u16> {
    let Some(chunk_count) = headers.first().map(|h| h.chunk_count) else {
        return vec![];
    };
    let seen: std::collections::HashSet<u16> = headers.iter().map(|h| h.chunk_index).collect();
    (0..chunk_count).filter(|i| !seen.contains(i)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompose_amount_splits_greedily_largest_first() {
        // 1.11 MAGLD = 111_000_000 petals = 1x D1 + 1x D0_1 + 1x D0_01.
        let notes = decompose_amount(111_000_000).unwrap();
        assert_eq!(notes, vec![DenominationTag::D1, DenominationTag::D0_1, DenominationTag::D0_01]);
    }

    #[test]
    fn decompose_amount_repeats_a_denomination_as_needed() {
        // 0.03 MAGLD = 3x D0_01.
        let notes = decompose_amount(3_000_000).unwrap();
        assert_eq!(notes, vec![DenominationTag::D0_01, DenominationTag::D0_01, DenominationTag::D0_01]);
    }

    #[test]
    fn decompose_amount_covers_every_ladder_rung_at_once() {
        let total: u64 = DENOMINATION_PETALS.iter().sum();
        let notes = decompose_amount(total).unwrap();
        assert_eq!(notes.len(), DENOMINATION_PETALS.len());
        let reconstructed: u64 = notes.iter().map(|d| DENOMINATION_PETALS[*d as usize]).sum();
        assert_eq!(reconstructed, total);
    }

    #[test]
    fn decompose_amount_rejects_sub_denomination_remainder() {
        assert!(decompose_amount(1_000_001).is_none());
    }

    #[test]
    fn decompose_amount_rejects_zero() {
        assert!(decompose_amount(0).is_none());
    }

    #[test]
    fn payment_request_round_trips_both_forms() {
        // The 40-byte pinned-amount form and the 32-byte amount-omitted form
        // (POOL-SPEC.md P5.6's two QR variants), distinguished by length alone.
        let with_amount = PaymentRequest { pk: [0xabu8; 32], amount_petals: Some(111_000_000) };
        assert_eq!(with_amount.encode().len(), 40);
        assert_eq!(PaymentRequest::from_text(&with_amount.to_text()).unwrap(), with_amount);

        let without_amount = PaymentRequest { pk: [0xcdu8; 32], amount_petals: None };
        assert_eq!(without_amount.encode().len(), 32);
        assert_eq!(PaymentRequest::from_text(&without_amount.to_text()).unwrap(), without_amount);

        assert!(PaymentRequest::decode(&[0u8; 33]).is_err());
        assert!(PaymentRequest::from_text("marigoldnote:00").is_err());
    }

    #[test]
    fn bearer_note_round_trips() {
        let bearer = BearerNote { sn: Hash::from_bytes([0x11u8; 32]), sk: [0x22u8; 32], d: DenominationTag::D0_1 };
        assert_eq!(bearer.encode().len(), 65);
        assert_eq!(BearerNote::from_text(&bearer.to_text()).unwrap(), bearer);
        assert!(BearerNote::decode(&[0u8; 64]).is_err());
        // Unknown denomination tag byte is rejected, not coerced.
        let mut bytes = bearer.encode();
        bytes[64] = 200;
        assert!(BearerNote::decode(&bytes).is_err());
    }

    fn info(byte: u8, d: DenominationTag) -> Arc<NoteKeyInfo> {
        Arc::new(NoteKeyInfo::new(Hash::from_bytes([byte; 32]), [byte; 32], d, NoteProvenance::Cold))
    }

    #[test]
    fn select_exact_greedy_over_the_ladder() {
        let available =
            vec![info(1, DenominationTag::D0_1), info(2, DenominationTag::D0_01), info(3, DenominationTag::D0_01), info(4, DenominationTag::D1)];
        // 0.12 = 0.1 + 2x0.01
        let selected = select_exact(&available, 12_000_000).unwrap();
        assert_eq!(selected.len(), 3);
        // 0.05 needs 5x0.01 but only 2 held — and greedy must NOT grab the 0.1 or 1.
        assert!(select_exact(&available, 5_000_000).is_none());
        // The full holdings sum exactly.
        assert!(select_exact(&available, 112_000_000).is_some());
        assert!(select_exact(&available, 113_000_000).is_none());
    }

    #[test]
    fn select_covering_prefers_exact_then_sweeps_smallest_first() {
        let available =
            vec![info(1, DenominationTag::D1), info(2, DenominationTag::D0_1), info(3, DenominationTag::D0_01), info(4, DenominationTag::D0_01)];
        // Exact representation exists: 0.11 = 0.1 + 0.01 (2 notes, no overshoot).
        let exact = select_covering(&available, 11_000_000).unwrap();
        assert_eq!(exact.len(), 2);
        // 0.05 has no exact representation — the smallest-first sweep covers it:
        // 0.01 + 0.01 + 0.1 = 0.12 >= 0.05 (3 notes, overshoot 0.07 becomes change).
        let covering = select_covering(&available, 5_000_000).unwrap();
        assert_eq!(covering.len(), 3);
        assert!(!covering.contains(&Hash::from_bytes([1u8; 32])), "the 1-MAGLD note must stay untouched");
        // More than everything held.
        assert!(select_covering(&available, 200_000_000).is_none());
    }

    #[test]
    fn fee_quanta_sizing() {
        // One quantum covers any fee up to 1,000,000 sompi; never zero quanta.
        assert_eq!(required_fee_quanta(0, 1.0), 1);
        assert_eq!(required_fee_quanta(9_009, 100.0), 1);
        assert_eq!(required_fee_quanta(10_001, 100.0), 2);
    }

    #[test]
    fn restore_rotation_plan_covers_every_serial_in_two_to_five_batches() {
        let serials: Vec<Hash> = (0..17u8).map(|i| Hash::from_bytes([i; 32])).collect();
        let batches = plan_restore_rotation(serials.clone());
        assert!((2..=5).contains(&batches.len()));
        let mut covered: Vec<Hash> = batches.iter().flatten().copied().collect();
        covered.sort_by_key(|h| h.as_bytes());
        let mut expected = serials.clone();
        expected.sort_by_key(|h| h.as_bytes());
        assert_eq!(covered, expected, "every serial appears exactly once across all batches");
        assert!(batches.iter().all(|batch| !batch.is_empty()), "no batch is left empty");
    }

    #[test]
    fn restore_rotation_plan_handles_small_inputs() {
        assert_eq!(plan_restore_rotation(vec![]), Vec::<Vec<Hash>>::new());
        let one = plan_restore_rotation(vec![Hash::from_bytes([1u8; 32])]);
        assert_eq!(one, vec![vec![Hash::from_bytes([1u8; 32])]]);
    }

    #[test]
    fn qr_page_header_round_trips_and_rejects_bad_version() {
        let header = QrPageHeader { backup_id: [0x11u8; 8], chunk_index: 2, chunk_count: 5, format_version: QR_PAGE_FORMAT_VERSION };
        let bytes = header.encode();
        assert_eq!(bytes.len(), 13);
        assert_eq!(QrPageHeader::decode(&bytes).unwrap(), header);

        let mut bad_version = bytes;
        bad_version[12] = 99;
        assert!(QrPageHeader::decode(&bad_version).is_err());
        assert!(QrPageHeader::decode(&[0u8; 5]).is_err());
    }

    #[test]
    fn paper_export_round_trips_and_chunks_correctly() {
        let entries: Vec<NoteKeyEntry> = (0..90u8)
            .map(|i| NoteKeyEntry::new(Hash::from_bytes([i; 32]), [i; 32], DenominationTag::D0_1, NoteProvenance::Cold))
            .collect();
        let password = Secret::from("paper-export-test-password");

        let pages = paper_export_encode(&entries, &password).unwrap();
        // 90 entries / 40 per page = 3 pages (40 + 40 + 10).
        assert_eq!(pages.len(), 3);

        let headers: Vec<QrPageHeader> = pages.iter().map(|p| paper_export_peek_header(p).unwrap()).collect();
        assert!(headers.iter().all(|h| h.chunk_count == 3));
        assert_eq!(headers.iter().map(|h| h.chunk_index).collect::<Vec<_>>(), vec![0, 1, 2]);
        assert!(paper_export_missing_pages(&headers).is_empty());
        // A page missing from the set is reported by index.
        assert_eq!(paper_export_missing_pages(&headers[..2]), vec![2]);

        let mut recovered: Vec<BearerNote> = vec![];
        for page in &pages {
            let (_, page_entries) = paper_export_decode_page(page, &password).unwrap();
            recovered.extend(page_entries);
        }
        assert_eq!(recovered.len(), 90);
        for entry in &entries {
            assert!(recovered.contains(&BearerNote { sn: entry.sn, sk: entry.sk, d: entry.d }));
        }

        // Wrong password fails to decrypt rather than silently returning garbage.
        let wrong = Secret::from("not-the-password");
        assert!(paper_export_decode_page(&pages[0], &wrong).is_err());
    }

    fn petals(magld: f64) -> u64 {
        (magld * 100_000_000.0).round() as u64
    }

    /// Two 1s and a 0.1 must pay 1.1 with 1 + 0.1, not with both 1s.
    #[test]
    fn cover_amount_prefers_the_tight_cover_over_largest_first() {
        // counts by denomination: 0.01, 0.1, 1, 10, ...
        let counts = [1, 1, 2, 0, 0, 0, 0, 0];
        assert_eq!(cover_amount(&counts, petals(1.1)), Some([0, 1, 1, 0, 0, 0, 0, 0]));
        assert_eq!(cover_amount(&counts, petals(1.11)), Some([1, 1, 1, 0, 0, 0, 0, 0]));
        // 1.12 cannot be met to the penny: the least over it is 2.
        assert_eq!(cover_amount(&counts, petals(1.12)), Some([0, 0, 2, 0, 0, 0, 0, 0]));
        // Three 1s beat a 10 for 2.5.
        assert_eq!(cover_amount(&[0, 0, 3, 1, 0, 0, 0, 0], petals(2.5)), Some([0, 0, 3, 0, 0, 0, 0, 0]));
        // Not enough at all.
        assert_eq!(cover_amount(&counts, petals(5.0)), None);
        // Exactly enough.
        assert_eq!(cover_amount(&counts, petals(2.11)), Some([1, 1, 2, 0, 0, 0, 0, 0]));
        // With plenty of 0.01s the cover still takes 1 + 0.1, not 1 + ten
        // stamps (the live run of 2026-09-15 did exactly that).
        assert_eq!(cover_amount(&[18, 5, 8, 0, 0, 0, 0, 0], petals(1.1)), Some([0, 1, 1, 0, 0, 0, 0, 0]));
        assert_eq!(cover_amount(&[18, 5, 8, 0, 0, 0, 0, 0], petals(0.25)), Some([5, 2, 0, 0, 0, 0, 0, 0]));
    }

    /// Past the cap the greedy fallback still covers, and says so by sum.
    #[test]
    fn cover_amount_falls_back_to_greed_when_the_space_is_huge() {
        // one 100,000-MAGLD note and a few small ones
        let counts = [3, 0, 0, 0, 0, 0, 0, 1];
        let take = cover_amount(&counts, petals(0.02)).expect("covered");
        let total: u64 = (0..8).map(|i| take[i] as u64 * DENOMINATION_PETALS[i]).sum();
        assert!(total >= petals(0.02));
    }

    /// The keep line is where storage mass makes a change coin cost more
    /// than it is worth; the 0.04 burned on 2026-09-15 sits below it and a
    /// whole MAGLD sits above it.
    #[test]
    fn change_keep_line_separates_dust_from_change_worth_keeping() {
        let line = change_keep_line(1_000_000_000_000, 105.0);
        assert!((10_000_000..11_000_000).contains(&line), "line was {line}");
        assert!(4_000_000 < line, "0.04 is below the line");
        assert!(100_000_000 > line, "a whole MAGLD is above it");
        assert_eq!(change_keep_line(1_000_000_000_000, 0.0), 0);
    }

    #[test]
    fn paper_export_handles_empty_entry_list() {
        let password = Secret::from("paper-export-test-password");
        let pages = paper_export_encode(&[], &password).unwrap();
        assert_eq!(pages.len(), 1);
        let (header, entries) = paper_export_decode_page(&pages[0], &password).unwrap();
        assert_eq!(header.chunk_count, 1);
        assert!(entries.is_empty());
    }
}

/// How many notes of each denomination to consume to cover `target_petals`
/// with the least value over it — the smallest reachable sum at or above the
/// target. `None` when the notes do not add up to it.
///
/// Largest-first greed is wrong here: asked for 1.1 with two 1s and a 0.1 in
/// hand it took both 1s and never looked at the 0.1 (2026-09-15). On a ledger
/// wallet that only means more change; on a notes-only wallet, where a
/// payout must cover the amount to within one 0.01 (FORK-PLAN P8.0b), it
/// turned a payable amount into a refusal. A bounded knapsack over 0.01
/// units answers exactly; the space is the target plus the largest note held,
/// since any cover further over can drop a note and still cover.
pub fn cover_amount(counts: &[usize; DENOMINATION_PETALS.len()], target_petals: u64) -> Option<[usize; DENOMINATION_PETALS.len()]> {
    const N: usize = DENOMINATION_PETALS.len();
    let quantum = DENOMINATION_PETALS[0];
    let held: u64 = (0..N).map(|i| counts[i] as u64 * DENOMINATION_PETALS[i]).sum();
    if held < target_petals {
        return None;
    }
    let target = target_petals.div_ceil(quantum);
    let largest = (0..N).rev().find(|&i| counts[i] > 0).map(|i| DENOMINATION_PETALS[i] / quantum).unwrap_or(0);
    // Past this the exact answer is not worth the memory; largest-first greed
    // is the old behaviour and still correct, just not tight.
    const CAP: u64 = 4_000_000;
    let limit = target + largest;
    if limit > CAP {
        let mut take = [0usize; N];
        let mut remaining = target_petals;
        for i in (0..N).rev() {
            let d = DENOMINATION_PETALS[i];
            let n = (remaining.div_ceil(d) as usize).min(counts[i]);
            take[i] = n;
            remaining = remaining.saturating_sub(n as u64 * d);
            if remaining == 0 {
                break;
            }
        }
        return Some(take);
    }
    let len = limit as usize + 1;
    let mut reach = vec![false; len];
    reach[0] = true;
    // used[i][s]: how many of denomination i the cover of sum s takes, so the
    // answer can be read back. Denominations go largest first, and a sum the
    // larger ones already reach is kept rather than rebuilt from smaller
    // notes: among equal covers this picks the fewest notes, and in
    // particular leaves the 0.01s alone — they are the fee stamps, and the
    // first cut spent ten of them on a 1.1 that a 1 and a 0.1 covered.
    let mut used: Vec<Vec<u32>> = Vec::with_capacity(N);
    for i in (0..N).rev() {
        let d = (DENOMINATION_PETALS[i] / quantum) as usize;
        let count = counts[i] as u32;
        let mut next = vec![false; len];
        let mut used_i = vec![0u32; len];
        for sum in 0..len {
            if reach[sum] {
                next[sum] = true;
            } else if count > 0 && sum >= d && next[sum - d] && used_i[sum - d] < count {
                next[sum] = true;
                used_i[sum] = used_i[sum - d] + 1;
            }
        }
        reach = next;
        used.push(used_i);
    }
    // `used` is in processing order: used[0] is the largest denomination.
    let mut sum = (target as usize..len).find(|&s| reach[s])?;
    let mut take = [0usize; N];
    for i in 0..N {
        let d = (DENOMINATION_PETALS[i] / quantum) as usize;
        let n = used[N - 1 - i][sum] as usize;
        take[i] = n;
        sum -= n * d;
    }
    debug_assert_eq!(sum, 0);
    Some(take)
}

/// The smallest change output worth keeping. KIP-9 prices an output at
/// `storage_mass_parameter / value` grams, so keeping a change coin of `v`
/// costs about `C · rate / v` in fee; below `sqrt(C · rate)` that is more than
/// the coin itself, and the generator (rightly) gives it to the miner instead
/// of creating it. At C = 10^12 and 105 sompi per gram the line is ~0.102 MAGLD.
pub fn change_keep_line(storage_mass_parameter: u64, fee_rate: f64) -> u64 {
    (storage_mass_parameter as f64 * fee_rate.max(0.0)).sqrt() as u64
}

/// Largest amount [`mint`] can currently fund from the account's mature
/// balance — i.e. mature minus the mint transaction's own fee ("note mint
/// all"). The fee is discovered by dry-running the generator over a sweep of
/// the full mature balance with a same-shape dummy Mint payload (only payload
/// LENGTH affects mass, so zeroed keys suffice), refined once for the final
/// amount's own decomposition, then verified with a real SenderPays estimate.
pub async fn max_mintable_petals(
    account: Arc<dyn Account>,
    fee_rate: Option<f64>,
    abortable: &Abortable,
    progress: Option<NoteProgress>,
) -> Result<u64> {
    let fee_rate = fee_rate.or(Some(POOL_FEE_RATE));
    let quantum = DENOMINATION_PETALS[0];
    // Sum the coins rather than trusting the cached Balance: it is None in
    // the window right after account activation, and reading 0 there made
    // automated minting skip silently on a funded wallet (2026-09-05).
    let (mature_entries, _, _) = account.utxo_context().utxo_entries_snapshot();
    let mature: u64 = mature_entries.iter().map(|entry| entry.amount()).sum();
    if mature < quantum * 2 {
        return Ok(0);
    }

    let dummy_payload = |petals: u64| -> Option<Vec<u8>> {
        let new_notes: Vec<NewNote> = decompose_amount(petals)?.iter().map(|d| NewNote { d: *d, pk: [0u8; 32] }).collect();
        Some(PoolOp::Mint(MintOp { new_notes }).encode_payload())
    };

    // Sweep-style estimate: fees come out of the destination, so this always
    // has funds and yields the fee for consuming the whole mature set.
    let mut candidate = mature - mature % quantum;
    for pass in 1..=3 {
        if let Some(progress) = &progress {
            progress(format!("estimating the sweep fee (pass {pass}, dry-running the transaction generator)..."));
        }
        let Some(payload) = dummy_payload(candidate) else { return Ok(0) };
        let change_address = account.change_address()?;
        let destination = PaymentDestination::PaymentOutputs(PaymentOutputs::from((change_address, mature)));
        let summary =
            account.clone().estimate(destination, fee_rate, Fees::ReceiverPays(0), Some(payload), abortable).await?;
        let next = mature.saturating_sub(summary.aggregate_fees()) / quantum * quantum;
        if next == 0 || next == candidate {
            candidate = next;
            break;
        }
        candidate = next;
    }
    if candidate == 0 {
        return Ok(0);
    }

    // Leave headroom. The estimate above dry-runs a sweep-shaped transaction
    // (one big output, fees deducted from it) while the real mint has no
    // payment outputs, carries the note payload, and produces change — a
    // different shape with a different mass. Spending the last petal means
    // any discrepancy at all comes back as "Insufficient funds", which is
    // exactly what a mining wallet with 140k coins reported (2026-09-05).
    // Two percent, floored at one quantum, costs nothing and never fails —
    // on a ledger big enough for the margin to survive as a change coin,
    // where it is minted on the next pass. Below the KIP-9 line it does not
    // survive: an output that small costs more in storage mass to keep than
    // it is worth, the generator hands it to the miner, and the margin is
    // simply burned. A 1.10 deposit minted 1.06 and paid 0.04 that way
    // (2026-09-15). So the margin is held back only when it can be kept;
    // otherwise the exact-shape verification below is the whole safety net,
    // with more tries.
    let margin = (candidate / 50).max(quantum);
    let keep_line =
        change_keep_line(Params::from(account.wallet().network_id()?).storage_mass_parameter, fee_rate.unwrap_or(POOL_FEE_RATE));
    let tries = if margin >= keep_line {
        candidate = candidate.saturating_sub(margin) / quantum * quantum;
        4
    } else {
        8
    };
    if candidate == 0 {
        return Ok(0);
    }

    // Verify the exact shape mint() will use; back off by one quantum at a
    // time if the refined estimate still lands a hair over.
    for _ in 0..tries {
        if let Some(progress) = &progress {
            progress(format!("verifying the final amount ({} MAGLD)...", crate::utils::sompi_to_kaspa_string(candidate)));
        }
        let Some(payload) = dummy_payload(candidate) else { return Ok(0) };
        let destination = PaymentDestination::PaymentOutputs(PaymentOutputs { outputs: vec![] });
        match account.clone().estimate(destination, fee_rate, Fees::SenderPays(candidate), Some(payload), abortable).await {
            Ok(_) => return Ok(candidate),
            Err(_) => {
                candidate = candidate.saturating_sub(quantum);
                if candidate == 0 {
                    return Ok(0);
                }
            }
        }
    }
    Ok(0)
}

/// Mint as much as the ledger allows, retrying with a larger reserve when the
/// transaction comes out too heavy.
///
/// [`max_mintable_petals`] can only estimate. It dry-runs a sweep — one payment
/// output, `ReceiverPays`, no payload — while the real mint has no outputs at
/// all, pays `SenderPays`, and carries a note payload of several hundred bytes.
/// Different shape, different fees, and on a fragmented ledger the difference
/// runs to more than the 2% margin: a mint of 58.81 out of 60 left 0.03 MAGLD
/// of change, and KIP-9 prices a dust output at 318,000 mass against a 100,000
/// limit (founder report, 2026-09-07). The estimate said yes; the chain said no.
///
/// The old back-off shed one 0.01 quantum per attempt, four attempts — three
/// orders of magnitude short of the MAGLD it needed, and driven by the same
/// estimate that had already been fooled. This reserves real change instead,
/// and quadruples the reserve on each rejection. Leaving a whole MAGLD behind
/// puts the output harmonic at 10,000, comfortably inside the limit, and the
/// remainder is not lost — it is minted on the next pass.
pub async fn mint_max(
    account: Arc<dyn Account>,
    wallet_secret: Secret,
    payment_secret: Option<Secret>,
    fee_rate: Option<f64>,
    abortable: &Abortable,
    progress: Option<NoteProgress>,
) -> Result<Option<(u64, MintResult)>> {
    let quantum = DENOMINATION_PETALS[0];
    let mintable = max_mintable_petals(account.clone(), fee_rate, abortable, progress.clone()).await?;
    if mintable == 0 {
        return Ok(None);
    }
    // One whole MAGLD of change keeps the output harmonic at 10,000 — an order
    // of magnitude inside the standard limit even before the inputs' credit.
    let mut reserve = 0u64;
    for attempt in 0..5 {
        let amount = mintable.saturating_sub(reserve) / quantum * quantum;
        if amount == 0 {
            return Ok(None);
        }
        match mint(account.clone(), wallet_secret.clone(), payment_secret.clone(), amount, fee_rate, abortable).await {
            Ok(result) => return Ok(Some((amount, result))),
            Err(err) if attempt < 4 && is_transaction_sizing_error(&err) => {
                reserve = if reserve == 0 { DENOMINATION_PETALS[2] } else { reserve.saturating_mul(4) };
                if let Some(progress) = &progress {
                    progress(format!(
                        "too heavy — retrying with {} MAGLD left on the ledger...",
                        crate::utils::sompi_to_kaspa_string(reserve)
                    ));
                }
            }
            Err(err) => return Err(err),
        }
    }
    Ok(None)
}

/// Whether an error means "this transaction was shaped wrong" — worth retrying
/// with different numbers — as opposed to a wrong password or a dead node,
/// where retrying just fails again more slowly.
fn is_transaction_sizing_error(err: &Error) -> bool {
    matches!(err, Error::MassCalculationError | Error::MassCalculationFailed(_))
        || {
            let text = err.to_string();
            text.contains("storage mass") || text.contains("mass") && text.contains("limit")
        }
}

/// Pay `amount_petals` to `address` using BOTH sides of the wallet in one
/// transaction: transparent inputs and consumed notes fund the same outputs.
/// This is the case an ordinary holder hits constantly — 300 on the ledger,
/// 300 in notes, wanting to send 500 — where paying from either side alone
/// reports "insufficient funds" despite the money being there.
///
/// Consensus already unifies the two sides: a pool op's consumed value counts
/// alongside transparent input value against outputs plus fee
/// (`tx_validation_in_utxo_context`, POOL-SPEC P5.2/P5.3). Signing order is
/// forced and non-circular: outputs are fixed first, the note signatures cover
/// `tx.outputs`, the payload is then final, and only then are the transparent
/// inputs signed over the whole transaction (which includes that payload).
pub async fn send_combined(
    account: Arc<dyn Account>,
    wallet_secret: Secret,
    payment_secret: Option<Secret>,
    address: Address,
    amount_petals: u64,
) -> Result<(Hash, u64, usize, usize)> {
    let network_id = account.wallet().network_id()?;
    let params: Params = network_id.into();
    let mass_calculator = MassCalculator::new_with_consensus_params(&params);

    // --- select the transparent side (all of it; change returns home) ---
    let (mature, _, _) = account.utxo_context().utxo_entries_snapshot();
    let transparent_total: u64 = mature.iter().map(|entry| entry.amount()).sum();

    // --- select notes to cover the rest, smallest-first so large notes stay whole ---
    let note_key_store = account.wallet().store().as_note_key_store()?;
    let mut available: Vec<Arc<NoteKeyInfo>> = Vec::new();
    let mut stream = note_key_store.iter().await?;
    while let Some(info) = stream.try_next().await? {
        if info.status == NoteStatus::Active {
            available.push(info);
        }
    }
    available.sort_by_key(|info| DENOMINATION_PETALS[info.d as usize]);

    // Cover the shortfall plus one quantum of headroom for the fee.
    let shortfall = amount_petals.saturating_sub(transparent_total).saturating_add(DENOMINATION_PETALS[0]);
    let mut selected_serials = Vec::new();
    let mut selected_value = 0u64;
    for info in available.iter() {
        if selected_value >= shortfall {
            break;
        }
        selected_serials.push(info.sn);
        selected_value += DENOMINATION_PETALS[info.d as usize];
    }
    if transparent_total + selected_value <= amount_petals {
        return Err(Error::Custom(format!(
            "insufficient funds: {} MAGLD on the ledger plus {} MAGLD in selectable notes",
            crate::utils::sompi_to_kaspa_string(transparent_total),
            crate::utils::sompi_to_kaspa_string(selected_value)
        )));
    }

    // --- note signature groups (one signature per shared key) ---
    let mut entries = Vec::with_capacity(selected_serials.len());
    for sn in &selected_serials {
        let entry = note_key_store
            .load_key(&wallet_secret, sn)
            .await?
            .ok_or_else(|| Error::Custom(format!("serial {sn} has no stored key")))?;
        entries.push(entry);
    }
    let mut groups_by_sk: HashMap<[u8; 32], Vec<Hash>> = HashMap::new();
    for entry in &entries {
        groups_by_sk.entry(entry.sk).or_default().push(entry.sn);
    }

    let server_info = account.wallet().rpc_api().get_server_info().await?;
    let freshness = FreshnessAnchor { anchor_daa_score: server_info.virtual_daa_score };
    const REDEEM_OP_TYPE: u8 = 2;

    // --- build inputs, then shape outputs and cost the transaction ---
    let inputs: Vec<kaspa_consensus_core::tx::TransactionInput> = mature
        .iter()
        .map(|entry| {
            kaspa_consensus_core::tx::TransactionInput::new(entry.utxo.outpoint.clone().into(), vec![], 0, 1)
        })
        .collect();
    let utxo_entries: Vec<kaspa_consensus_core::tx::UtxoEntry> = mature.iter().map(|entry| entry.utxo.as_ref().into()).collect();

    let change_script = pay_to_address_script(&account.change_address()?);
    let recipient_script = pay_to_address_script(&address);
    let available_total = transparent_total + selected_value;

    let placeholder_groups: Vec<SignedGroup> =
        groups_by_sk.values().map(|serials| SignedGroup { serials: serials.clone(), signature: [0u8; 64] }).collect();
    let placeholder_payload = PoolOp::Redeem(RedeemOp { consumed: placeholder_groups, freshness }).encode_payload();
    let placeholder_outputs = vec![
        TransactionOutput::new(amount_petals, recipient_script.clone()),
        TransactionOutput::new(available_total - amount_petals, change_script.clone()),
    ];
    let placeholder_tx = Transaction::new(
        TX_VERSION_TOCCATA,
        inputs.clone(),
        placeholder_outputs,
        0,
        SUBNETWORK_ID_NOTE_POOL,
        0,
        placeholder_payload,
    );
    let populated = PopulatedTransaction::new(&placeholder_tx, utxo_entries.clone());
    let masses = mass_calculator
        .calc_contextual_masses(&populated)
        .ok_or_else(|| Error::Custom("combined send: mass calculation failed".to_string()))?;
    let compute_mass = mass_calculator.calc_non_contextual_masses(&placeholder_tx).compute_mass;
    let total_mass = masses.storage_mass.max(compute_mass);

    // The mass here was already right; the feerate was not. The node's
    // estimate reports 1 petal per gram, the mempool relay minimum is 100, and
    // the old 1,000-petal floor was three orders of magnitude too small to
    // paper over the gap.
    let feerate = match account.wallet().rpc_api().get_fee_estimate().await {
        Ok(estimate) => estimate.normal_buckets.first().map(|b| b.feerate).unwrap_or(1.0),
        Err(_) => 1.0,
    }
    .max(POOL_FEE_RATE);
    let fee_petals = (total_mass as f64 * feerate).ceil() as u64;
    if available_total <= amount_petals + fee_petals {
        return Err(Error::Custom("insufficient funds once the fee is included".to_string()));
    }

    let change = available_total - amount_petals - fee_petals;
    let mut outputs = vec![TransactionOutput::new(amount_petals, recipient_script)];
    if change >= DENOMINATION_PETALS[0] {
        outputs.push(TransactionOutput::new(change, change_script));
    }
    let outputs_hash = transparent_outputs_hash(&outputs);

    // --- sign the notes over the final outputs, then finalize the payload ---
    let mut signed_groups = Vec::with_capacity(groups_by_sk.len());
    for (sk_bytes, group_serials) in groups_by_sk {
        let hash = signing_hash(REDEEM_OP_TYPE, &group_serials, &[], outputs_hash, freshness.anchor_daa_score);
        let keypair =
            Keypair::from_seckey_slice(SECP256K1, &sk_bytes).map_err(|e| Error::Custom(format!("invalid note secret key: {e}")))?;
        let signature: [u8; 64] = *keypair.sign_schnorr(Message::from_digest(hash.into())).as_ref();
        signed_groups.push(SignedGroup { serials: group_serials, signature });
    }
    let payload = PoolOp::Redeem(RedeemOp { consumed: signed_groups, freshness }).encode_payload();

    // --- sign the transparent inputs over the now-final transaction ---
    let tx = Transaction::new(TX_VERSION_TOCCATA, inputs, outputs, 0, SUBNETWORK_ID_NOTE_POOL, 0, payload);
    tx.set_storage_mass(masses.storage_mass);
    let signable = kaspa_consensus_core::tx::SignableTransaction::with_entries(tx, utxo_entries);
    let keydata = account.prv_key_data(wallet_secret.clone()).await?;
    let signer = Signer::new(account.clone(), keydata, payment_secret);
    let addresses: Vec<Address> = mature.iter().filter_map(|entry| entry.utxo.address.clone()).collect();
    let signed = crate::tx::generator::signer::SignerT::try_sign(&signer, signable, &addresses)?;

    let rpc_tx: kaspa_rpc_core::RpcTransaction = (&signed.tx).into();
    let transaction_id = account.wallet().rpc_api().submit_transaction(rpc_tx, false).await?;

    for sn in &selected_serials {
        note_key_store.mark_status(sn, NoteStatus::Superseded).await?;
    }

    Ok((transaction_id, fee_petals, mature.len(), selected_serials.len()))
}

/// Groups of ten same-denomination notes that would consolidate into one note
/// of the next size up. Denominations being powers of ten, rotating such a
/// group re-produces it as a single larger note (`decompose_amount` is greedy
/// largest-first), so this needs no new operation — it is [`rotate_notes`]
/// applied deliberately.
///
/// Two denominations are deliberately left alone: the top one (nothing larger
/// to merge into) and the smallest, because 0.01 notes are the fee stamps
/// every pure-pool operation spends — merging them away would force later
/// rotations into slack mode, where the fee comes out of the note's own value
/// and breaks its denomination. A merge costs one fee quantum (0.01), so this
/// also keeps the cost proportionate: consolidating ten 0.1s into 1 MAGLD
/// spends 1% of it, while the same fee against ten 1s is a tenth of a percent.
/// Fee stamps (0.01 notes) to keep on hand. Every pure pool operation spends
/// one, so a wallet needs a working supply — merging the supply away would
/// force later rotations into slack mode, where the fee comes out of a note's
/// own value and breaks its denomination.
/// How far above the fee a fee-source may reach before the operation is
/// refused. A fee is normally paid with one 0.01 stamp; overshooting by a
/// hundredfold means no stamp is available and something much larger is about
/// to be broken up for it. Waiting a minute for stamps to confirm is always
/// the better trade.
pub const FEE_SOURCE_MAX_OVERSHOOT: u64 = 100;

/// How long a note may be missing from the pool before reconciliation calls it
/// a phantom. Covers the gap between submitting a transaction and the chain
/// accepting it, with room for a node that is catching up.
pub const IN_FLIGHT_SECS: u64 = 120;

pub const STAMP_RESERVE: usize = 50;

/// Don't touch the stamps until there are clearly too many. Merging as soon as
/// the count passes the reserve would have the wallet consolidating stamps
/// continuously, since minting makes more whenever the supply runs low — the
/// two would chase each other forever. A gap between the trigger and the
/// reserve is what makes the cycle terminate.
pub const STAMP_MERGE_TRIGGER: usize = 100;

/// Reconcile the vault against the pool: which notes this wallet believes it
/// holds actually exist on chain. Returns (present, phantom).
///
/// The wallet stores a note the moment its creating transaction is submitted,
/// so a transaction that never lands leaves a note in the vault that exists
/// nowhere else. Balance cannot detect that on its own — it reports belief, not
/// fact — so this asks the node directly.
pub async fn verify_held_notes(wallet: &Arc<Wallet>) -> Result<(Vec<Arc<NoteKeyInfo>>, Vec<Arc<NoteKeyInfo>>)> {
    let note_key_store = wallet.store().as_note_key_store()?;

    // Notes written in the last couple of minutes are in flight, not lost. A
    // note is stored the instant its transaction is submitted, so a mint that
    // has just run leaves a vault full of notes the pool has not accepted yet
    // — and reporting those as "NOT on chain" the moment housekeeping finishes
    // is how a healthy wallet was made to look like it had lost two thousand
    // MAGLD (founder report, 2026-09-07). The window is generous on purpose:
    // being slow to notice a real phantom costs nothing, and crying wolf about
    // money costs trust.
    let in_flight: HashSet<Hash> = note_key_store.recently_written(IN_FLIGHT_SECS).await?.into_iter().collect();

    let mut held: Vec<Arc<NoteKeyInfo>> = Vec::new();
    let mut stream = note_key_store.iter().await?;
    while let Some(info) = stream.try_next().await? {
        if matches!(info.status, NoteStatus::Active | NoteStatus::Mirrored) && !in_flight.contains(&info.sn) {
            held.push(info);
        }
    }
    let mut present = Vec::new();
    let mut phantom = Vec::new();
    // Chunked: a mining wallet can hold thousands of notes, and one query
    // carrying every serial is a needlessly large request to build and parse.
    for chunk in held.chunks(500) {
        let confirmed = pool_confirmed(wallet, chunk.iter().map(|i| i.sn).collect()).await?;
        for info in chunk {
            if confirmed.contains(&info.sn) {
                present.push(info.clone());
            } else {
                phantom.push(info.clone());
            }
        }
    }
    Ok((present, phantom))
}

/// Strikes before a note stops being counted as money.
///
/// Three, and they must be consecutive: a serial that turns up once resets to
/// zero. Each strike costs a separate check against a node that declared
/// itself caught up, so three is three independent statements that the pool
/// does not have it.
pub const MISSING_STRIKES_BEFORE_UNKNOWN: u32 = 3;

/// What a reconciliation pass did.
pub struct Reconciliation {
    /// Notes the pool confirms, which is the normal case.
    pub present: usize,
    /// Missing, but not yet out of strikes — still counted as money.
    pub pending: usize,
    /// Moved to [`NoteStatus::Unknown`] by this pass.
    pub moved_to_unknown: Vec<Arc<NoteKeyInfo>>,
    /// Value of the notes still pending, in petals.
    pub pending_petals: u64,
}

/// Check held notes against the pool and act on what is missing.
///
/// Does nothing unless the node says it is caught up. An unsynced node has not
/// finished reading the chain, so it reports every note as absent — counting
/// those would move a wallet's entire holdings to `Unknown` within three
/// checks of a fresh sync starting.
///
/// A note that is missing is not yet a note that is gone. It gets a strike;
/// three consecutive strikes and it stops being counted as money and moves to
/// [`NoteStatus::Unknown`], where an archival lookup can later say whether the
/// transaction that would have created it ever landed — and therefore whether
/// this was money spent or money that never moved.
pub async fn reconcile_held_notes(wallet: &Arc<Wallet>) -> Result<Option<Reconciliation>> {
    if !wallet.utxo_processor().is_synced() {
        return Ok(None);
    }
    let note_key_store = wallet.store().as_note_key_store()?;
    let (present, phantom) = verify_held_notes(wallet).await?;

    // A serial that turned up cancels whatever it had accumulated.
    for info in &present {
        note_key_store.clear_missing(&info.sn).await?;
    }

    let mut moved_to_unknown = Vec::new();
    let mut pending = 0usize;
    let mut pending_petals = 0u64;
    for info in &phantom {
        let strikes = note_key_store.record_missing(&info.sn).await?;
        if strikes >= MISSING_STRIKES_BEFORE_UNKNOWN {
            note_key_store.mark_status(&info.sn, NoteStatus::Unknown).await?;
            moved_to_unknown.push(info.clone());
        } else {
            pending += 1;
            pending_petals += DENOMINATION_PETALS[info.d as usize];
        }
    }

    Ok(Some(Reconciliation { present: present.len(), pending, moved_to_unknown, pending_petals }))
}

/// Which of `serials` the node actually holds in the pool. A wallet stores a
/// note as `Active` the moment its creating transaction is submitted — the
/// serial is derived from the transaction id, so it is known before the chain
/// has accepted it — and consuming one that has not landed yet is rejected
/// outright. A failed query is an error, never an empty set: silently reading
/// as "you hold nothing" would turn a dropped connection into a no-op that
/// looks like tidy housekeeping.
async fn pool_confirmed(wallet: &Arc<Wallet>, serials: Vec<Hash>) -> Result<HashSet<Hash>> {
    if serials.is_empty() {
        return Ok(HashSet::new());
    }
    let entries = wallet.rpc_api().get_notes_by_serial(serials).await?;
    Ok(entries.into_iter().map(|entry| entry.sn).collect())
}

pub async fn plan_merges(wallet: &Arc<Wallet>) -> Result<Vec<Vec<Hash>>> {
    let note_key_store = wallet.store().as_note_key_store()?;
    let mut candidates: Vec<(usize, Hash)> = Vec::new();
    let mut stream = note_key_store.iter().await?;
    while let Some(info) = stream.try_next().await? {
        if info.status == NoteStatus::Active {
            candidates.push((info.d as usize, info.sn));
        }
    }

    // Local `Active` is not the same as "exists in the pool". A note is stored
    // Active the moment its creating transaction is SUBMITTED — its serial is
    // derived from the transaction id, so it is known before the chain has
    // accepted it. Planning against that set means proposing to consume notes
    // that do not exist yet, which the node rejects outright: "consumed serial
    // ... does not exist in the pool" (founder report, 2026-09-06). Ask the
    // node what is really there and plan only over that.
    let serials: Vec<Hash> = candidates.iter().map(|(_, sn)| *sn).collect();
    let confirmed: HashSet<Hash> = pool_confirmed(wallet, serials).await?;

    let mut by_denomination: HashMap<usize, Vec<Hash>> = HashMap::new();
    for (d, sn) in candidates {
        if confirmed.contains(&sn) {
            by_denomination.entry(d).or_default().push(sn);
        }
    }

    let mut plans = Vec::new();
    // Fee stamps (index 0) are held back up to a working reserve — every pure
    // pool operation spends one, and merging the supply away would force later
    // rotations into slack mode. Above the reserve they are just dust that can
    // never merge on its own, which is how a wallet ended up holding 125 of
    // them (founder report, 2026-09-06). Once past the trigger, consolidate
    // down toward the reserve and leave the rest alone.
    if let Some(serials) = by_denomination.get(&0) {
        if serials.len() > STAMP_MERGE_TRIGGER {
            let excess = &serials[STAMP_RESERVE..];
            for group in excess.chunks(10) {
                if group.len() == 10 {
                    plans.push(group.to_vec());
                }
            }
        }
    }
    // Skip the top denomination — nothing above it to merge into.
    for d in 1..DENOMINATION_PETALS.len() - 1 {
        let Some(serials) = by_denomination.get(&d) else { continue };
        for group in serials.chunks(10) {
            if group.len() == 10 {
                plans.push(group.to_vec());
            }
        }
    }
    Ok(plans)
}

/// Consolidate held notes into the fewest possible, ten at a time, carrying up
/// the ladder until nothing more can merge. Returns how many groups were merged
/// and the reason it stopped, if it stopped early.
///
/// `limit` is a safety bound on one call, not a work quota: the caller that
/// wants the vault actually tidy passes a number large enough to reach the top.
/// A small one leaves a backlog — capping a run at twelve merged eleven groups
/// of fee stamps and one of 0.1s, and left 122 notes of 10 MAGLD sitting
/// exactly where they were (founder report, 2026-09-06).
pub async fn merge_held_notes(wallet: &Arc<Wallet>, wallet_secret: Secret, limit: usize) -> Result<(usize, Option<String>)> {
    let mut merged = 0usize;
    // Serials this call has already spent. A plan is computed up front, but
    // `rotate_notes` sources its fee from the smallest spare it can find —
    // and that spare is very often a 0.01 stamp belonging to a group later in
    // the SAME plan. Merging the 10s would quietly eat a stamp out of the
    // stamp group, and by the time the plan reached it the chain had confirmed
    // the spend, so the node answered "consumed serial ... does not exist in
    // the pool" (founder report, 2026-09-06). Fee sourcing is deliberately
    // free to take any spare; the planner is what has to keep up.
    let mut spent: HashSet<Hash> = HashSet::new();
    // Re-plan after each pass so the merge carries up the ladder in one call:
    // ten 0.1s become a 1, and that new 1 may complete a group of ten 1s that
    // becomes a 10. Planning once would climb a single rung per run.
    while merged < limit {
        let plans: Vec<Vec<Hash>> = plan_merges(wallet)
            .await?
            .into_iter()
            .filter(|group| group.iter().all(|sn| !spent.contains(sn)))
            .collect();
        if plans.is_empty() {
            break;
        }
        let before = merged;
        // The reason a merge stopped is reported, not swallowed. A silent
        // `Ok(0)` is indistinguishable from "nothing to do", and that is
        // precisely how a wallet sat with 22 notes of 1 MAGLD unmerged
        // without ever saying why.
        for group in plans.into_iter().take(limit - merged) {
            if group.iter().any(|sn| spent.contains(sn)) {
                continue;
            }
            match rotate_notes(wallet, wallet_secret.clone(), group).await {
                Ok(result) => {
                    merged += 1;
                    // Both the group and whatever spare paid its fee.
                    spent.extend(result.consumed_serials.iter().copied());
                }
                Err(err) => return Ok((merged, Some(err.to_string()))),
            }
        }
        if merged == before {
            break;
        }
    }
    Ok((merged, None))
}

#[cfg(test)]
mod fee_sizing {
    use super::*;

    /// A pool op's fee must cover what the node will charge for it. The node
    /// prices payload bytes at `mass_per_tx_byte.max(2)` grams each and demands
    /// at least 100 sompi per gram, so the fee has to track payload size — it
    /// cannot be a constant. It was one, because the sizing read storage mass,
    /// which is structurally zero for a transaction with no outputs.
    #[test]
    fn the_fee_covers_what_the_node_charges_at_every_size() {
        const RELAY_SOMPI_PER_GRAM: u64 = 100;
        let calculator = MassCalculator::new_with_consensus_params(&Params::from(NetworkId::with_suffix(
            kaspa_consensus_core::network::NetworkType::Testnet,
            10,
        )));
        for n in [1usize, 10, 25, 50, 100, 200] {
            let groups: Vec<Vec<Hash>> = (0..n).map(|i| vec![Hash::from_u64_word(i as u64)]).collect();
            let produced: Vec<NewNote> = (0..n).map(|_| NewNote { d: DenominationTag::D1, pk: [0u8; 32] }).collect();
            let freshness = FreshnessAnchor { anchor_daa_score: 1 };
            let mass = estimate_transfer_mass(&calculator, &groups, &produced, freshness).unwrap();
            let quanta = required_fee_quanta(mass, POOL_FEE_RATE);
            let we_pay = quanta * FEE_QUANTUM_PETALS;
            let node_wants = mass * RELAY_SOMPI_PER_GRAM;
            assert!(
                we_pay >= node_wants,
                "{n} notes: mass {mass}, node wants {node_wants} petals, we offer {we_pay} ({quanta} quanta)"
            );
        }
    }

    /// The everyday case stays one penny. A ten-note merge is what housekeeping
    /// runs constantly, and it must not creep up to two quanta.
    #[test]
    fn a_ten_note_merge_still_costs_one_quantum() {
        let calculator = MassCalculator::new_with_consensus_params(&Params::from(NetworkId::with_suffix(
            kaspa_consensus_core::network::NetworkType::Testnet,
            10,
        )));
        let groups: Vec<Vec<Hash>> = (0..10).map(|i| vec![Hash::from_u64_word(i as u64)]).collect();
        let produced = vec![NewNote { d: DenominationTag::D10, pk: [0u8; 32] }];
        let mass = estimate_transfer_mass(&calculator, &groups, &produced, FreshnessAnchor { anchor_daa_score: 1 }).unwrap();
        assert_eq!(required_fee_quanta(mass, POOL_FEE_RATE), 1, "a ten-note merge should cost 0.01, mass was {mass}");
    }
}

#[cfg(test)]
mod paper_export_salt_tests {
    use super::*;

    /// Two backups of the same notes under the same password must not produce
    /// the same bytes. Before the salt fix they did, because the Argon2 salt
    /// was derived from the password itself.
    #[test]
    fn two_backups_under_one_password_differ() {
        let entries = vec![NoteKeyEntry::new(
            Hash::from_bytes([0x11u8; 32]),
            [0x22u8; 32],
            DenominationTag::D1,
            NoteProvenance::Cold,
        )];
        let password = Secret::from("the same password both times");
        let a = paper_export_encode(&entries, &password).unwrap();
        let b = paper_export_encode(&entries, &password).unwrap();
        // The backup id differs by design; compare only the encrypted bodies.
        assert_ne!(a[0][QR_HEADER_LEN..], b[0][QR_HEADER_LEN..], "same password must not give identical ciphertext");

        for pages in [&a, &b] {
            let (_, notes) = paper_export_decode_page(&pages[0], &password).unwrap();
            assert_eq!(notes.len(), 1);
            assert_eq!(notes[0].sn, entries[0].sn);
        }
        assert!(paper_export_decode_page(&a[0], &Secret::from("wrong")).is_err());
    }

    /// A page printed by older software must still decode, or someone's drawer
    /// full of paper becomes worthless.
    #[test]
    fn pages_written_before_the_salt_still_decode() {
        let password = Secret::from("old password");
        let note = BearerNote { sn: Hash::from_bytes([0x33u8; 32]), sk: [0x44u8; 32], d: DenominationTag::D10 };
        let mut plaintext = Vec::new();
        plaintext.extend_from_slice(&1u16.to_le_bytes());
        plaintext.extend_from_slice(&note.encode());
        let legacy = crate::encryption::encrypt_xchacha20poly1305(&plaintext, &password).unwrap();

        let header = QrPageHeader { backup_id: [7u8; 8], chunk_index: 0, chunk_count: 1, format_version: QR_PAGE_FORMAT_VERSION };
        let mut page = header.encode().to_vec();
        page.extend_from_slice(&legacy);

        let (_, notes) = paper_export_decode_page(&page, &password).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].sn, note.sn);
        assert_eq!(notes[0].sk, note.sk);
    }
}

#[cfg(test)]
mod mirror_export_tests {
    use super::*;

    #[test]
    fn a_page_round_trips_and_fits_a_phone() {
        let entries: Vec<NoteKeyEntry> = (0..40u8)
            .map(|i| NoteKeyEntry::new(Hash::from_bytes([i; 32]), [i.wrapping_add(1); 32], DenominationTag::D1, NoteProvenance::Cold))
            .collect();
        let password = Secret::from("the phone passphrase");
        let pages = mirror_export_pages(&entries, &password).unwrap();
        assert_eq!(pages.len(), 1, "forty notes should be one page");

        // Telegram CloudStorage caps a value at 4096 characters. A page that
        // does not fit cannot be stored at all, so this is a hard limit, not a
        // preference.
        assert!(pages[0].len() <= 4096, "page is {} chars, over the 4096 limit", pages[0].len());

        let (header, notes) = mirror_import_page(&pages[0], &password).unwrap();
        assert_eq!(header.chunk_count, 1);
        assert_eq!(notes.len(), 40);
        assert_eq!(notes[7].sn, entries[7].sn);
        assert_eq!(notes[7].sk, entries[7].sk);

        assert!(mirror_import_page(&pages[0], &Secret::from("wrong")).is_err());
        assert!(mirror_import_page("not base64 at all !!", &password).is_err());
    }

    #[test]
    fn more_than_a_page_of_notes_chunks() {
        let entries: Vec<NoteKeyEntry> = (0..90u8)
            .map(|i| NoteKeyEntry::new(Hash::from_bytes([i; 32]), [i.wrapping_add(1); 32], DenominationTag::D0_1, NoteProvenance::Cold))
            .collect();
        let password = Secret::from("pw");
        let pages = mirror_export_pages(&entries, &password).unwrap();
        assert_eq!(pages.len(), 3);
        let mut total = 0;
        for page in &pages {
            assert!(page.len() <= 4096);
            let (header, notes) = mirror_import_page(page, &password).unwrap();
            assert_eq!(header.chunk_count, 3);
            total += notes.len();
        }
        assert_eq!(total, 90);
    }
}
