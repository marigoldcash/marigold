//! Time-based one-time passwords (RFC 6238), used to protect an open session.
//!
//! # What this actually defends
//!
//! Be clear about it, because a security feature that is oversold is worse
//! than none. The secret below lives inside the wallet payload, encrypted
//! under the wallet password. Anyone holding both the wallet file and the
//! password can therefore read the secret and generate codes at will — so
//! this is **not** a second factor on the file.
//!
//! What it defends is the *session*: a wallet already open, on a machine
//! somebody else has walked up to, or an SSH session left connected. There
//! the attacker has neither the password nor the file — they have a prompt.
//! Asking for a code off a phone before every spend is a real barrier to
//! that, and it is the barrier people actually need day to day.
//!
//! It is a barrier in the interface, not in the mathematics: anyone able to
//! run their own code as this user can go around it. Binding a second factor
//! into the key derivation needs a factor that does not change every thirty
//! seconds — a hardware key's `hmac-secret`, not a rotating code.

use crate::imports::*;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha1::Sha1;

/// RFC 4226 recommends at least 128 bits and says 160 is preferred; 160 is
/// also exactly one SHA-1 block's worth, which is what every authenticator
/// app expects.
pub const SECRET_BYTES: usize = 20;

/// Six digits over thirty seconds. Not configurable: every authenticator app
/// supports this and a fair number quietly ignore anything else, which would
/// hand somebody a QR code that scans and then never matches.
pub const DIGITS: u32 = 6;
pub const PERIOD_SECS: u64 = 30;

/// How far either side of now a code is accepted. One step covers the phone
/// and the machine disagreeing by up to thirty seconds, and covers typing a
/// code that expires while it is being typed. Widening this widens the window
/// an onlooker has.
pub const SKEW_STEPS: u64 = 1;

/// How long a verified code is allowed to stand for before another is asked
/// for. Zero — a code every time — is the default, because it is the only
/// setting with no hole in it and because the moments it guards are the
/// moments that already stop to ask for a password. Somebody minting in bulk
/// can widen it knowingly; nobody should have it widened for them.
pub const DEFAULT_GRACE_SECS: u64 = 0;

/// The widest grace worth offering. Beyond about this, an unattended terminal
/// is unprotected for long enough that the authenticator is decoration.
pub const MAX_GRACE_SECS: u64 = 3600;

/// One enrolled authenticator.
#[derive(Clone, Debug, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Otp {
    /// The shared secret. Never leaves the encrypted payload except to be
    /// shown once, at enrolment, to the person setting it up.
    pub secret: Vec<u8>,
    /// Unix seconds. Shown by `otp status` so a stale enrolment from a phone
    /// that no longer exists is recognisable as one.
    pub enrolled_at: u64,
    /// How long a verified code stands before the next one is asked for.
    /// 0 means every time, which is the default.
    pub grace_secs: u64,
}

impl Otp {
    pub fn generate() -> Self {
        let mut secret = vec![0u8; SECRET_BYTES];
        rand::thread_rng().fill_bytes(&mut secret);
        Self { secret, enrolled_at: unix_now_secs(), grace_secs: DEFAULT_GRACE_SECS }
    }

    /// The secret in the form a person types into an authenticator by hand,
    /// grouped into fours because twenty-six unbroken characters get
    /// mistyped.
    pub fn manual_entry_key(&self) -> String {
        base32_encode(&self.secret).as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join(" ")
    }

    /// The `otpauth://` URI an authenticator app scans.
    ///
    /// `label` is what the app shows in its list. The issuer is repeated as a
    /// query parameter because that is where most apps read it from, and left
    /// in the label as well because a few older ones only read it there.
    pub fn provisioning_uri(&self, label: &str) -> String {
        let secret = base32_encode(&self.secret);
        let label = percent_encode(label);
        format!("otpauth://totp/Marigold:{label}?secret={secret}&issuer=Marigold&algorithm=SHA1&digits={DIGITS}&period={PERIOD_SECS}")
    }

    /// Is `entered` a valid code for `at` (unix seconds)?
    ///
    /// Non-digits are ignored, so a code pasted as "123 456" works.
    pub fn verify_at(&self, entered: &str, at: u64) -> bool {
        let entered: String = entered.chars().filter(|c| c.is_ascii_digit()).collect();
        if entered.len() != DIGITS as usize {
            return false;
        }
        let step = at / PERIOD_SECS;
        (step.saturating_sub(SKEW_STEPS)..=step.saturating_add(SKEW_STEPS))
            .any(|s| constant_time_eq(entered.as_bytes(), self.code_for_step(s).as_bytes()))
    }

    pub fn verify(&self, entered: &str) -> bool {
        self.verify_at(entered, unix_now_secs())
    }

    /// Which time step a code belongs to, or `None` if it matches no step in
    /// the window. Callers use this to refuse a code that has already been
    /// used once — a shoulder-surfed code is otherwise good for a full period.
    pub fn step_of(&self, entered: &str, at: u64) -> Option<u64> {
        let entered: String = entered.chars().filter(|c| c.is_ascii_digit()).collect();
        if entered.len() != DIGITS as usize {
            return None;
        }
        let step = at / PERIOD_SECS;
        (step.saturating_sub(SKEW_STEPS)..=step.saturating_add(SKEW_STEPS))
            .find(|s| constant_time_eq(entered.as_bytes(), self.code_for_step(*s).as_bytes()))
    }

    /// Seconds until the current code is replaced. Shown at the prompt so
    /// nobody types a code with one second left on it.
    pub fn seconds_remaining(at: u64) -> u64 {
        PERIOD_SECS - (at % PERIOD_SECS)
    }

    /// RFC 6238 §4 / RFC 4226 §5.3: HMAC-SHA1 over the counter, dynamic
    /// truncation, modulo.
    pub fn code_for_step(&self, step: u64) -> String {
        let mut mac = <Hmac<Sha1> as Mac>::new_from_slice(&self.secret).expect("HMAC accepts a key of any length");
        mac.update(&step.to_be_bytes());
        let digest = mac.finalize().into_bytes();

        let offset = (digest[digest.len() - 1] & 0x0f) as usize;
        let binary = ((digest[offset] & 0x7f) as u32) << 24
            | (digest[offset + 1] as u32) << 16
            | (digest[offset + 2] as u32) << 8
            | (digest[offset + 3] as u32);

        format!("{:0width$}", binary % 10u32.pow(DIGITS), width = DIGITS as usize)
    }
}

/// Comparison whose duration does not depend on where the first difference
/// is. Overkill for a six-digit code an attacker can only guess once per
/// prompt, but the cost is nothing and the habit is worth keeping.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

const BASE32_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// RFC 4648 base32, unpadded — what `otpauth://` URIs carry.
pub fn base32_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(5) * 8);
    let mut buffer = 0u16;
    let mut bits = 0u32;
    for &byte in data {
        buffer = (buffer << 8) | byte as u16;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(BASE32_ALPHABET[((buffer >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(BASE32_ALPHABET[((buffer << (5 - bits)) & 0x1f) as usize] as char);
    }
    out
}

/// The inverse, tolerant of spaces, lowercase and padding — people paste
/// these out of password managers in every shape.
pub fn base32_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buffer = 0u16;
    let mut bits = 0u32;
    for c in text.chars() {
        if c == '=' || c.is_whitespace() || c == '-' {
            continue;
        }
        let value = BASE32_ALPHABET.iter().position(|&a| a == c.to_ascii_uppercase() as u8)? as u16;
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

/// Only the characters that would break the URI. Authenticator labels read
/// better with spaces left alone than turned into `%20`.
fn percent_encode(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '?' | '#' | '&' | '=' | '/' | '%' | ':' => format!("%{:02X}", c as u8),
            _ => c.to_string(),
        })
        .collect()
}

pub fn unix_now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238 Appendix B, the SHA-1 rows. If these pass, every
    /// authenticator app agrees with us.
    fn rfc_vector_otp() -> Otp {
        Otp { secret: b"12345678901234567890".to_vec(), enrolled_at: 0, grace_secs: 0 }
    }

    #[test]
    fn rfc6238_test_vectors() {
        let otp = rfc_vector_otp();
        for (time, expected) in [
            (59u64, "287082"),
            (1111111109, "081804"),
            (1111111111, "050471"),
            (1234567890, "005924"),
            (2000000000, "279037"),
            (20000000000, "353130"),
        ] {
            assert_eq!(otp.code_for_step(time / PERIOD_SECS), expected, "RFC 6238 vector at t={time}");
        }
    }

    #[test]
    fn a_code_is_accepted_a_step_either_side_and_no_further() {
        let otp = rfc_vector_otp();
        let now = 1111111111;
        assert!(otp.verify_at(&otp.code_for_step(now / PERIOD_SECS), now));
        assert!(otp.verify_at(&otp.code_for_step(now / PERIOD_SECS - 1), now), "a phone thirty seconds slow still works");
        assert!(otp.verify_at(&otp.code_for_step(now / PERIOD_SECS + 1), now), "and thirty seconds fast");
        assert!(!otp.verify_at(&otp.code_for_step(now / PERIOD_SECS - 2), now), "but a minute stale does not");
        assert!(!otp.verify_at(&otp.code_for_step(now / PERIOD_SECS + 2), now));
    }

    #[test]
    fn junk_is_refused_without_pretending_it_is_a_code() {
        let otp = rfc_vector_otp();
        let now = 1111111111;
        assert!(!otp.verify_at("", now));
        assert!(!otp.verify_at("12345", now), "five digits is not a code");
        assert!(!otp.verify_at("1234567", now));
        assert!(!otp.verify_at("abcdef", now));
        assert_eq!(otp.step_of("nonsense", now), None);
    }

    /// People paste codes with the space their phone displays.
    #[test]
    fn a_pasted_code_with_a_space_in_it_works() {
        let otp = rfc_vector_otp();
        let now = 1111111111;
        let code = otp.code_for_step(now / PERIOD_SECS);
        let spaced = format!("{} {}", &code[..3], &code[3..]);
        assert!(otp.verify_at(&spaced, now));
    }

    /// The step a code belongs to is what stops it being used twice.
    #[test]
    fn a_code_reports_the_step_it_came_from() {
        let otp = rfc_vector_otp();
        let now = 1111111111;
        let step = now / PERIOD_SECS;
        assert_eq!(otp.step_of(&otp.code_for_step(step), now), Some(step));
        assert_eq!(otp.step_of(&otp.code_for_step(step - 1), now), Some(step - 1));
    }

    /// Against the RFC 4648 §10 vectors, and round-trips.
    #[test]
    fn base32_matches_the_rfc_and_round_trips() {
        for (input, encoded) in [("", ""), ("f", "MY"), ("fo", "MZXQ"), ("foo", "MZXW6"), ("foob", "MZXW6YQ"), ("fooba", "MZXW6YTB")] {
            assert_eq!(base32_encode(input.as_bytes()), encoded, "encoding {input:?}");
            assert_eq!(base32_decode(encoded).unwrap(), input.as_bytes(), "decoding {encoded:?}");
        }

        let otp = Otp::generate();
        assert_eq!(base32_decode(&base32_encode(&otp.secret)).unwrap(), otp.secret);
        assert_eq!(base32_decode(&otp.manual_entry_key()).unwrap(), otp.secret, "typed back with the spaces in");
    }

    /// The URI has to carry the secret an app can actually use, and survive a
    /// wallet name with a character that would otherwise end the query.
    #[test]
    fn the_provisioning_uri_carries_a_usable_secret() {
        let otp = rfc_vector_otp();
        let uri = otp.provisioning_uri("holiday fund&x");
        assert!(uri.starts_with("otpauth://totp/Marigold:holiday fund%26x?"), "{uri}");
        let secret = uri.split("secret=").nth(1).unwrap().split('&').next().unwrap();
        assert_eq!(base32_decode(secret).unwrap(), otp.secret);
        assert!(uri.contains("algorithm=SHA1") && uri.contains("digits=6") && uri.contains("period=30"));
    }

    #[test]
    fn two_enrolments_do_not_share_a_secret() {
        assert_ne!(Otp::generate().secret, Otp::generate().secret);
        assert_eq!(Otp::generate().secret.len(), SECRET_BYTES);
    }

    #[test]
    fn the_countdown_runs_down_and_resets() {
        assert_eq!(Otp::seconds_remaining(0), 30);
        assert_eq!(Otp::seconds_remaining(29), 1);
        assert_eq!(Otp::seconds_remaining(30), 30);
    }
}
