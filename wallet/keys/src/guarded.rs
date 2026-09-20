//!
//! A secret kept for longer than one command.
//!
//! [`Secret`] wipes itself when dropped, and for a password typed at a prompt
//! and used once that is the whole of its life. The wallet also keeps a few
//! secrets for a session: the password the housekeeping signs with, the one
//! `serve` reads from a file. Those sat in memory as plain bytes, where a swap
//! file, a crash dump or another process of the same user could find them by
//! looking for readable text.
//!
//! [`Guarded`] stores such a secret as two halves, a random mask and the secret
//! XORed with it. Each half alone is random; the secret exists only for the
//! moment a caller [`reveal`](Guarded::reveal)s it, in a [`Secret`] that wipes
//! itself, and the halves are re-drawn on every reveal. Both halves are locked
//! in memory so they never reach swap.
//!
//! What this does not do, said plainly: an attacker who can read this
//! process's memory *and* run code can find both halves and combine them,
//! because the wallet has to. It defeats the cheap attacks — scanning a dump
//! or swap for text — and, with [`harden_process`], the common one: another
//! process of the same user attaching to read memory at all.
//!

use crate::secret::Secret;
use rand::RngCore;
use zeroize::Zeroize;

/// A secret held as two random-looking halves. See the module doc.
pub struct Guarded {
    mask: Vec<u8>,
    masked: Vec<u8>,
}

impl Guarded {
    /// Guard a copy of `secret`. The caller still owns the original and should
    /// let it drop as soon as it can.
    pub fn new(secret: &[u8]) -> Self {
        let mut mask = vec![0u8; secret.len()];
        rand::thread_rng().fill_bytes(&mut mask);
        let masked: Vec<u8> = secret.iter().zip(&mask).map(|(s, m)| s ^ m).collect();
        let guarded = Self { mask, masked };
        guarded.lock_pages();
        guarded
    }

    /// Guard a [`Secret`] and consume it, so the plain copy is wiped here and
    /// now rather than whenever the caller gets round to dropping it.
    pub fn from_secret(secret: Secret) -> Self {
        let guarded = Self::new(secret.as_ref());
        drop(secret);
        guarded
    }

    /// The secret, for now: a fresh [`Secret`] that wipes itself when dropped.
    /// Both halves are re-drawn, so two reveals leave nothing in common in
    /// memory.
    pub fn reveal(&mut self) -> Secret {
        let plain: Vec<u8> = self.mask.iter().zip(&self.masked).map(|(m, x)| m ^ x).collect();
        self.rotate();
        Secret::new(plain)
    }

    pub fn len(&self) -> usize {
        self.mask.len()
    }

    pub fn is_empty(&self) -> bool {
        self.mask.is_empty()
    }

    /// New mask, same secret. Done a byte at a time so the whole secret never
    /// sits contiguous anywhere during the change.
    fn rotate(&mut self) {
        let mut fresh = vec![0u8; self.mask.len()];
        rand::thread_rng().fill_bytes(&mut fresh);
        for ((m, x), f) in self.mask.iter_mut().zip(self.masked.iter_mut()).zip(&fresh) {
            let byte = *m ^ *x;
            *m = *f;
            *x = byte ^ *f;
        }
        fresh.zeroize();
    }

    #[cfg(all(unix, not(target_arch = "wasm32")))]
    fn lock_pages(&self) {
        // Best effort: a locked-memory limit that is too small fails the call,
        // and the wallet must keep working, so the result is not checked.
        // SAFETY: both slices are live for as long as `self`, and mlock only
        // pins their pages.
        unsafe {
            libc::mlock(self.mask.as_ptr() as *const libc::c_void, self.mask.len());
            libc::mlock(self.masked.as_ptr() as *const libc::c_void, self.masked.len());
        }
    }

    #[cfg(not(all(unix, not(target_arch = "wasm32"))))]
    fn lock_pages(&self) {}
}

impl Drop for Guarded {
    fn drop(&mut self) {
        self.mask.zeroize();
        self.masked.zeroize();
        #[cfg(all(unix, not(target_arch = "wasm32")))]
        // SAFETY: the same pages lock_pages pinned; wiping them first means
        // whatever is paged out afterwards is zeros.
        unsafe {
            libc::munlock(self.mask.as_ptr() as *const libc::c_void, self.mask.len());
            libc::munlock(self.masked.as_ptr() as *const libc::c_void, self.masked.len());
        }
    }
}

impl std::fmt::Debug for Guarded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Guarded").field("bytes", &self.mask.len()).finish()
    }
}

/// Make this process's memory harder to read from outside it.
///
/// Linux: not dumpable, which empties core dumps and refuses `ptrace` from
/// any other process of the same user; root is unaffected. macOS: deny
/// debugger attachment. Everywhere Unix: a zero core-file limit. Call once,
/// early. Set `MARIGOLD_DEBUGGABLE=1` to skip it, for a session under a
/// debugger.
pub fn harden_process() {
    if std::env::var_os("MARIGOLD_DEBUGGABLE").is_some() {
        return;
    }
    #[cfg(all(unix, not(target_arch = "wasm32")))]
    // SAFETY: plain libc calls with constant arguments; none touches memory
    // we own.
    unsafe {
        let none = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        libc::setrlimit(libc::RLIMIT_CORE, &none);
        #[cfg(target_os = "linux")]
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
        #[cfg(target_os = "macos")]
        libc::ptrace(libc::PT_DENY_ATTACH, 0, std::ptr::null_mut(), 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn random_secret(len: usize) -> Vec<u8> {
        let mut v = vec![0u8; len];
        rand::thread_rng().fill_bytes(&mut v);
        v
    }

    #[test]
    fn neither_half_holds_the_secret_and_together_they_do() {
        let plain = random_secret(32);
        let mut guarded = Guarded::new(&plain);
        assert_ne!(guarded.mask, plain);
        assert_ne!(guarded.masked, plain);
        assert!(!contains(&guarded.mask, &plain[..8]) && !contains(&guarded.masked, &plain[..8]));
        assert_eq!(guarded.reveal().as_ref(), &plain[..]);
    }

    #[test]
    fn every_reveal_redraws_both_halves() {
        let plain = random_secret(32);
        let mut guarded = Guarded::new(&plain);
        let (m1, x1) = (guarded.mask.clone(), guarded.masked.clone());
        assert_eq!(guarded.reveal().as_ref(), &plain[..]);
        assert_ne!(guarded.mask, m1);
        assert_ne!(guarded.masked, x1);
        assert_eq!(guarded.reveal().as_ref(), &plain[..]);
    }

    #[test]
    fn an_empty_secret_is_fine() {
        let mut guarded = Guarded::new(&[]);
        assert!(guarded.is_empty());
        assert!(guarded.reveal().as_ref().is_empty());
    }

    /// The claim that matters, checked against the process itself: after a
    /// secret is guarded and its plain copy dropped, a scan of this process's
    /// readable memory finds no run of bytes equal to it, while the same scan
    /// does find a plain [`Secret`] holding another random value (so the scan
    /// is known to work). The needle is never materialised either: each
    /// window is compared against mask XOR masked byte by byte.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_memory_scan_finds_the_plain_secret_but_not_the_guarded_one() {
        let guarded = {
            // Not the RNG's bytes as they are: the thread RNG keeps its last
            // output block in its own buffer, and a secret equal to it would
            // be found there, which says nothing about Guarded. A password
            // does not come from the RNG.
            let mut plain: Vec<u8> =
                random_secret(48).into_iter().enumerate().map(|(i, b)| b.wrapping_mul(3).wrapping_add(i as u8)).collect();
            let g = Guarded::new(&plain);
            plain.zeroize();
            g
        };
        let control = {
            let plain: Vec<u8> =
                random_secret(48).into_iter().enumerate().map(|(i, b)| b.wrapping_mul(5).wrapping_add(i as u8)).collect();
            Secret::new(plain)
        };

        let guarded_hits = scan_process_memory(|window| {
            window.len() == guarded.len() && window.iter().zip(guarded.mask.iter().zip(&guarded.masked)).all(|(w, (m, x))| *w == m ^ x)
        });
        let control_hits = scan_process_memory(|window| window == control.as_ref());
        assert!(control_hits >= 1, "the scan must be able to find a plain secret");
        assert_eq!(guarded_hits, 0, "the guarded secret was found contiguous in memory");
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    /// Walk every readable, writable, private mapping of this process and
    /// count the 48-byte windows `matches` accepts. Heap and stack included;
    /// file-backed and shared mappings skipped, since a secret cannot be there.
    #[cfg(target_os = "linux")]
    fn scan_process_memory(matches: impl Fn(&[u8]) -> bool) -> usize {
        use std::io::{Read, Seek, SeekFrom};
        let maps = std::fs::read_to_string("/proc/self/maps").unwrap();
        let mut mem = std::fs::File::open("/proc/self/mem").unwrap();
        let mut hits = 0;
        for line in maps.lines() {
            let mut parts = line.split_whitespace();
            let (Some(range), Some(perms), Some(_offset), Some(_dev), Some(inode)) =
                (parts.next(), parts.next(), parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            if !perms.starts_with("rw")
                || !perms.ends_with('p')
                || inode != "0"
                || line.contains("[vvar]")
                || line.contains("[vsyscall]")
            {
                continue;
            }
            let (start, end) = range.split_once('-').unwrap();
            let (start, end) = (u64::from_str_radix(start, 16).unwrap(), u64::from_str_radix(end, 16).unwrap());
            let mut buf = vec![0u8; (end - start) as usize];
            if mem.seek(SeekFrom::Start(start)).is_err() || mem.read_exact(&mut buf).is_err() {
                continue;
            }
            hits += buf.windows(48).filter(|w| matches(w)).count();
            buf.zeroize();
        }
        hits
    }
}
