//! The wallet password policy, one for the terminal, the desktop and the phone.

/// The wallet password policy (founder, 2026-10-10): a password, or at least a
/// PIN, of a reasonable length is required of a regular user; a wallet without
/// a password is a deliberate advanced choice — `advanced on` in the terminal,
/// the advanced box on the desktop's Create screen, or
/// `MARIGOLD_ALLOW_EMPTY_PASSWORD=1` for scripts — whatever the reason. Nothing
/// here changes at mainnet; the advanced choice stays.
pub const MIN_WALLET_PASSWORD_CHARS: usize = 4;

/// Scripts and tests say so in the environment rather than in a terminal mode.
pub fn empty_password_allowed_by_env() -> bool {
    std::env::var("MARIGOLD_ALLOW_EMPTY_PASSWORD").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// What a wallet password typed at creation gets: the one place the policy
/// lives, so the terminal wizard and the desktop agree, and so a test can
/// hold the policy to the requirement (GitHub #20). `allow_empty` is the
/// advanced choice made for this creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordVerdict {
    /// Empty, by an advanced choice: accepted, with a note.
    EmptyAllowed,
    /// Empty without that choice: ask again, and say where the choice is.
    EmptyRefused,
    /// Shorter than `MIN_WALLET_PASSWORD_CHARS`: ask again.
    TooShort,
    Fine,
}

pub fn password_verdict(password: &str, allow_empty: bool) -> PasswordVerdict {
    let chars = password.chars().count();
    if chars == 0 {
        if allow_empty { PasswordVerdict::EmptyAllowed } else { PasswordVerdict::EmptyRefused }
    } else if chars < MIN_WALLET_PASSWORD_CHARS {
        PasswordVerdict::TooShort
    } else {
        PasswordVerdict::Fine
    }
}

#[cfg(test)]
mod password_policy_tests {
    use super::*;

    /// The requirement (founder, 2026-10-10): a regular user needs a password
    /// or a PIN of reasonable length; no password is an advanced choice only.
    #[test]
    fn no_password_is_an_advanced_choice_only() {
        assert_eq!(password_verdict("", false), PasswordVerdict::EmptyRefused);
        assert_eq!(password_verdict("", true), PasswordVerdict::EmptyAllowed);
    }

    #[test]
    fn a_pin_of_four_is_enough_and_three_is_not() {
        assert_eq!(password_verdict("123", false), PasswordVerdict::TooShort);
        assert_eq!(password_verdict("123", true), PasswordVerdict::TooShort);
        assert_eq!(password_verdict("1234", false), PasswordVerdict::Fine);
        assert_eq!(password_verdict("héhé", false), PasswordVerdict::Fine, "characters, not bytes");
        assert_eq!(password_verdict("correct horse battery staple", false), PasswordVerdict::Fine);
    }

    #[test]
    fn the_environment_switch_is_off_unless_set() {
        // SAFETY: the test owns this variable; nothing else in the suite reads it.
        unsafe { std::env::remove_var("MARIGOLD_ALLOW_EMPTY_PASSWORD") };
        assert!(!empty_password_allowed_by_env());
        unsafe { std::env::set_var("MARIGOLD_ALLOW_EMPTY_PASSWORD", "1") };
        assert!(empty_password_allowed_by_env());
        unsafe { std::env::remove_var("MARIGOLD_ALLOW_EMPTY_PASSWORD") };
    }
}
