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
use crate::storage::{NoteKeyEntry, NoteProvenance, NoteStatus};
use crate::tx::{Fees, Generator, GeneratorSettings, PaymentDestination, PaymentOutputs, Signer};
use kaspa_consensus_core::config::params::Params;
use kaspa_consensus_core::constants::TX_VERSION_TOCCATA;
use kaspa_consensus_core::mass::MassCalculator;
use kaspa_consensus_core::notepool::{
    DENOMINATION_PETALS, DenominationTag, FreshnessAnchor, MintOp, NewNote, PoolOp, RedeemOp, SignedGroup,
    hashing::{serial_hash, signing_hash, transparent_outputs_hash},
};
use kaspa_consensus_core::subnets::SUBNETWORK_ID_NOTE_POOL;
use kaspa_consensus_core::tx::{PopulatedTransaction, Transaction, TransactionOutput};
use kaspa_hashes::Hash;
use kaspa_txscript::pay_to_address_script;
use secp256k1::{Keypair, Message, SECP256K1, SecretKey};
use workflow_core::abortable::Abortable;

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
    pub fee_sompi: u64,
    pub serials: Vec<Hash>,
}

/// Redeem notes back to transparent balance (FORK-PLAN P7.2). Builds a zero-transparent-
/// input `RedeemOp` transaction directly (see this module's doc comment for why),
/// signs each `SignedGroup` with its notes' own key(s), and submits over RPC.
pub async fn redeem(account: Arc<dyn Account>, wallet_secret: Secret, selection: RedeemSelection) -> Result<RedeemResult> {
    let note_key_store = account.wallet().store().as_note_key_store()?;

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
            let mut candidates: Vec<_> = note_key_store
                .iter()
                .await?
                .try_filter(|info| futures::future::ready(info.status == NoteStatus::Active))
                .try_collect()
                .await?;
            candidates.sort_by_key(|info| std::cmp::Reverse(DENOMINATION_PETALS[info.d as usize]));

            let mut selected = Vec::new();
            let mut total = 0u64;
            for info in candidates {
                if total >= target_petals {
                    break;
                }
                total += DENOMINATION_PETALS[info.d as usize];
                selected.push(info.sn);
            }
            if total < target_petals {
                return Err(Error::Custom(format!(
                    "insufficient note balance: {total} petals available, {target_petals} requested"
                )));
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

    let network_id = account.utxo_context().processor().network_id()?;
    let params = Params::from(network_id);
    let mass_calculator = MassCalculator::new_with_consensus_params(&params);

    let server_info = account.wallet().rpc_api().get_server_info().await?;
    let freshness = FreshnessAnchor { anchor_daa_score: server_info.virtual_daa_score };

    let change_address = account.change_address()?;
    let script_public_key = pay_to_address_script(&change_address);

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
    let placeholder_output = TransactionOutput::new(redeemed_value_petals, script_public_key.clone());
    let placeholder_tx =
        Transaction::new(TX_VERSION_TOCCATA, vec![], vec![placeholder_output], 0, SUBNETWORK_ID_NOTE_POOL, 0, placeholder_payload);
    let populated = PopulatedTransaction::new(&placeholder_tx, vec![]);
    let storage_mass = mass_calculator
        .calc_contextual_masses(&populated)
        .ok_or_else(|| Error::Custom("redeem: mass calculation failed".to_string()))?
        .storage_mass;

    let feerate = match account.wallet().rpc_api().get_fee_estimate().await {
        Ok(estimate) => estimate.normal_buckets.first().map(|b| b.feerate).unwrap_or(1.0),
        Err(_) => 1.0,
    };
    let fee_sompi = (storage_mass as f64 * feerate).ceil() as u64;

    if redeemed_value_petals <= fee_sompi {
        return Err(Error::Custom(format!(
            "redeemed value ({redeemed_value_petals} petals) does not cover the estimated fee ({fee_sompi} sompi)"
        )));
    }
    let output_value = redeemed_value_petals - fee_sompi;
    let output = TransactionOutput::new(output_value, script_public_key);
    let outputs_hash = transparent_outputs_hash(std::slice::from_ref(&output));

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
    let tx = Transaction::new(
        TX_VERSION_TOCCATA,
        vec![],
        vec![output],
        0,
        SUBNETWORK_ID_NOTE_POOL,
        0,
        redeem_payload,
    );
    tx.set_storage_mass(storage_mass);

    let rpc_tx: kaspa_rpc_core::RpcTransaction = (&tx).into();
    let transaction_id = account.wallet().rpc_api().submit_transaction(rpc_tx, false).await?;

    // The redeemed serials are gone from the pool the instant this transaction
    // confirms; mark them superseded now (plaintext-only, matches how the P7.1
    // `NotesChanged` listener would eventually observe the same removal).
    for sn in &serials {
        note_key_store.mark_status(sn, NoteStatus::Superseded).await?;
    }

    Ok(RedeemResult { transaction_id, redeemed_value_petals, fee_sompi, serials })
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
}
