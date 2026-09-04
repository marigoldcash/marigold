//!
//! Compatibility layer for legacy wallets — **emptied by FORK-PLAN P7.0**.
//!
//! The legacy-Kaspa import surfaces that lived here (`gen0.rs`, the KDX keydata
//! import; `gen1.rs`, the Go-`kaspawallet` file import) were removed: on a
//! fair-launch chain they could never find funds, and their only possible
//! real-world effect was inviting users to type real Kaspa wallet passwords into
//! Marigold software — a key-reuse hazard that normalizes exactly the behavior
//! wallet phishing depends on. See docs/marigold/DECISIONS.md (P7.0).
//!
//! The module itself is retained as a tombstone so the removal is discoverable in
//! place. Pre-existing wallet *storage* containing legacy accounts still opens
//! (the account variants and derivation code live elsewhere); only the ways to
//! import legacy-Kaspa key material are gone.
//!
