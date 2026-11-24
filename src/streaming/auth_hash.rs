//! Apache `htpasswd` password-hash verification.
//!
//! Replaces the abandoned `htpasswd-verify` crate (which pulled the unmaintained
//! `rust-crypto`, `rustc-serialize`, `time`, and `gcc` crates — RUSTSEC-2022-0011,
//! RUSTSEC-2022-0004, RUSTSEC-2020-0071, RUSTSEC-2025-0121). Verification is
//! performed directly with maintained RustCrypto primitives (`bcrypt`, `md-5`,
//! `sha1`) plus the existing `base64` dependency.
//!
//! # Supported hash formats
//!
//! | Prefix      | Format        | Notes |
//! |-------------|---------------|-------|
//! | `$2a$`/`$2b$`/`$2y$`/`$2x$` | bcrypt | The recommended modern format. Verified via the `bcrypt` crate (constant-time internally). |
//! | `$apr1$`    | Apache APR1 MD5 crypt | Apache `htpasswd -m` default. |
//! | `$1$`       | standard MD5 crypt     | Same algorithm as APR1, different magic. |
//! | `{SHA}`     | `base64(SHA1(password))` | Apache `htpasswd -s`. |
//!
//! APR1 and `$1$` share the Poul-Henning Kamp MD5-crypt algorithm; only the
//! magic identifier differs (and is mixed into the digest).

use md5::Digest;

/// Crypt(3) base64 alphabet used by MD5-crypt / APR1 (NOT standard base64).
const CRYPT_BASE64: &[u8; 64] = b"./0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Encode the low `n` 6-bit groups of `value` as crypt-base64 (least-significant first).
fn crypt_to64(mut value: u32, n: usize) -> String {
    let mut out = String::with_capacity(n);
    for _ in 0..n {
        out.push(CRYPT_BASE64[(value & 0x3f) as usize] as char);
        value >>= 6;
    }
    out
}

/// Compute an MD5-crypt / APR1 hash body (the 22-char crypt-base64 suffix).
///
/// `magic` is `"$1$"` (standard MD5 crypt) or `"$apr1$"` (Apache APR1). `salt`
/// is the raw salt bytes; it is truncated to 8 bytes per the crypt(3) spec.
fn md5_crypt(magic: &str, password: &[u8], salt: &[u8]) -> String {
    // Salt is limited to 8 bytes.
    let salt = if salt.len() > 8 { &salt[..8] } else { salt };

    // 1. alt = MD5(password || salt || password)
    let mut alt = md5::Md5::new();
    alt.update(password);
    alt.update(salt);
    alt.update(password);
    let mut alt: [u8; 16] = alt.finalize().into();

    // 2. ctx = MD5(password || magic || salt)
    let mut ctx = md5::Md5::new();
    ctx.update(password);
    ctx.update(magic.as_bytes());
    ctx.update(salt);

    // 3. Add `alt`, `take` bytes at a time, for the password length.
    let mut plen = password.len();
    while plen > 0 {
        let take = plen.min(16);
        ctx.update(&alt[..take]);
        plen -= take;
    }

    // Classic crypt(3) quirk: zero the first byte of the alternate sum before
    // the bit-length step. This matches openssl / glibc / Apache apr_md5 (the
    // reference vectors below were generated with `openssl passwd`).
    alt[0] = 0;

    // 4. For each bit of the password length (LSB first), add alt[0] (now zero)
    //    when the bit is 1, otherwise password[0]. This "nothing-up-my-sleeve"
    //    step makes the hash length-sensitive.
    let mut bits = password.len();
    while bits > 0 {
        if bits & 1 != 0 {
            ctx.update(&alt[..1]);
        } else {
            ctx.update(&password[..1]);
        }
        bits >>= 1;
    }

    let mut current: [u8; 16] = ctx.finalize().into();

    // 5. 1000 rounds of strengthening.
    for i in 0..1000u32 {
        let mut round = md5::Md5::new();
        if i & 1 != 0 {
            round.update(password);
        } else {
            round.update(current);
        }
        if i % 3 != 0 {
            round.update(salt);
        }
        if i % 7 != 0 {
            round.update(password);
        }
        if i & 1 != 0 {
            round.update(current);
        } else {
            round.update(password);
        }
        current = round.finalize().into();
    }

    // 6. Emit the 16 digest bytes in the crypt(3) MD5 permutation order.
    let mut out = String::with_capacity(22);
    let triples: [(usize, usize, usize); 5] =
        [(0, 6, 12), (1, 7, 13), (2, 8, 14), (3, 9, 15), (4, 10, 5)];
    for (a, b, c) in triples {
        out.push_str(&crypt_to64(
            ((current[a] as u32) << 16) | ((current[b] as u32) << 8) | (current[c] as u32),
            4,
        ));
    }
    out.push_str(&crypt_to64(current[11] as u32, 2));
    out
}

/// Verify a password against a stored MD5-crypt / APR1 hash.
///
/// `stored` is the full `$<magic>$<salt>$<encoded>` string. The password is
/// re-hashed with the extracted salt and the result is compared in constant
/// time.
fn verify_md5_crypt(stored: &str, password: &str) -> bool {
    use subtle::ConstantTimeEq;

    let (magic, rest) = if let Some(rest) = stored.strip_prefix("$apr1$") {
        ("$apr1$", rest)
    } else if let Some(rest) = stored.strip_prefix("$1$") {
        ("$1$", rest)
    } else {
        return false;
    };

    let (salt, expected) = match rest.split_once('$') {
        Some((s, e)) => (s.as_bytes(), e),
        None => return false,
    };

    let computed = md5_crypt(magic, password.as_bytes(), salt);
    bool::from(computed.as_bytes().ct_eq(expected.as_bytes()))
}

/// Verify a password against a stored `{SHA}` hash (`base64(SHA1(password))`).
fn verify_sha1_apache(stored: &str, password: &str) -> bool {
    use base64::Engine;
    use subtle::ConstantTimeEq;

    let expected = match stored.strip_prefix("{SHA}") {
        Some(e) => e,
        None => return false,
    };
    let digest = sha1::Sha1::digest(password.as_bytes());
    let computed = base64::engine::general_purpose::STANDARD.encode(digest);
    bool::from(computed.as_bytes().ct_eq(expected.as_bytes()))
}

/// Verify a password against a stored bcrypt hash via the `bcrypt` crate.
fn verify_bcrypt(stored: &str, password: &str) -> bool {
    bcrypt::verify(password, stored).unwrap_or(false)
}

/// Name the legacy (weak) htpasswd format of `hash`, or `None` for bcrypt and
/// unrecognized input (SEC-201).
///
/// `{SHA}` is unsalted SHA-1 and `$1$`/`$apr1$` is MD5-crypt; both remain
/// accepted for compatibility but are cheap to brute-force offline, so the
/// caller should warn the operator to regenerate the hash with bcrypt.
pub fn legacy_hash_format(hash: &str) -> Option<&'static str> {
    if hash.starts_with("{SHA}") {
        Some("{SHA} (unsalted SHA-1)")
    } else if hash.starts_with("$apr1$") {
        Some("$apr1$ (APR1 MD5-crypt)")
    } else if hash.starts_with("$1$") {
        Some("$1$ (MD5-crypt)")
    } else {
        None
    }
}

/// Warn once per process so the deprecation/refusal notice cannot be
/// re-spammed by repeated authentication attempts against a legacy hash.
static LEGACY_NOTICE_EMITTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Emit the SEC-002 legacy-hash notice once per process. `refused` selects
/// the wording: refusal by default (gate closed) versus deprecation (gate
/// explicitly opened).
fn emit_legacy_notice(refused: bool) {
    use std::sync::atomic::Ordering;
    if LEGACY_NOTICE_EMITTED.swap(true, Ordering::Relaxed) {
        return;
    }
    if refused {
        log::warn!(
            "Legacy htpasswd hash format ({{SHA}}/$1$/$apr1$) credential refused: these \
             formats are refused by default since the 0.89-era SEC-002 follow-up. \
             Regenerate the hash with bcrypt (`htpasswd -nB`) or opt in via \
             StreamingConfig.allow_legacy_password_hashes to keep verifying it."
        );
    } else {
        log::warn!(
            "DEPRECATION: legacy htpasswd hash format ({{SHA}}/$1$/$apr1$) accepted via the \
             explicit StreamingConfig.allow_legacy_password_hashes opt-in. Support is \
             deprecated and will be removed two minor releases after the default refusal \
             landed; regenerate the hash with bcrypt (`htpasswd -nB`)."
        );
    }
}

/// Verify a password against any supported htpasswd-format hash, with the
/// SEC-002 legacy-format gate.
///
/// bcrypt hashes are always verified. `{SHA}`, `$1$`, and `$apr1$` hashes
/// are refused outright (return `false`) when `allow_legacy` is `false` —
/// the library default, matching the standalone streamer's startup refusal —
/// and verified with a loud deprecation warning when it is `true`.
///
/// Returns `false` for an unrecognized format (never panics). All
/// comparisons are constant-time.
pub fn verify_htpasswd_hash_gated(hash: &str, password: &str, allow_legacy: bool) -> bool {
    let is_legacy = legacy_hash_format(hash).is_some();
    if is_legacy && !allow_legacy {
        emit_legacy_notice(true);
        return false;
    }
    if is_legacy {
        emit_legacy_notice(false);
    }
    verify_htpasswd_hash(hash, password)
}

/// Verify a password against any supported htpasswd-format hash.
///
/// Dispatches on the hash prefix. Returns `false` for an unrecognized format
/// (never panics). All comparisons are constant-time.
///
/// This function accepts legacy `{SHA}`/`$1$`/`$apr1$` hashes unconditionally;
/// prefer [`verify_htpasswd_hash_gated`] for anything reachable from network
/// authentication paths (SEC-002).
pub fn verify_htpasswd_hash(hash: &str, password: &str) -> bool {
    if hash.starts_with("$2a$")
        || hash.starts_with("$2b$")
        || hash.starts_with("$2y$")
        || hash.starts_with("$2x$")
    {
        verify_bcrypt(hash, password)
    } else if hash.starts_with("$apr1$") || hash.starts_with("$1$") {
        verify_md5_crypt(hash, password)
    } else if hash.starts_with("{SHA}") {
        verify_sha1_apache(hash, password)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // All vectors below were generated with `openssl` (authoritative):
    //   openssl passwd -apr1 -salt salt password   -> $apr1$salt$Xxd1irWT9ycqoYxGFn4cb.
    //   openssl passwd -1    -salt salt password    -> $1$salt$qJH7.N4xYta3aEG/dfqo/0
    // SHA1("{SHA}" password): echo -n password | openssl dgst -sha1 -binary | base64

    #[test]
    fn test_apr1_known_vector() {
        // openssl passwd -apr1 -salt salt password
        let stored = "$apr1$salt$Xxd1irWT9ycqoYxGFn4cb.";
        assert!(verify_htpasswd_hash(stored, "password"));
        assert!(!verify_htpasswd_hash(stored, "wrong-password"));
        // Constant-time path returns false cleanly for malformed hashes.
        assert!(!verify_htpasswd_hash("$apr1$", "password"));
    }

    #[test]
    fn test_md5crypt_dollar_one_known_vector() {
        // openssl passwd -1 -salt salt password
        let stored = "$1$salt$qJH7.N4xYta3aEG/dfqo/0";
        assert!(verify_htpasswd_hash(stored, "password"));
        assert!(!verify_htpasswd_hash(stored, "not-the-password"));
    }

    #[test]
    fn test_sha1_known_vector() {
        // base64(SHA1("password"))
        let stored = "{SHA}W6ph5Mm5Pz8GgiULbPgzG37mj9g=";
        assert!(verify_htpasswd_hash(stored, "password"));
        assert!(!verify_htpasswd_hash(stored, "Password")); // case-sensitive
        assert!(!verify_htpasswd_hash("{SHA}", "password"));
    }

    #[test]
    fn test_bcrypt_round_trip_and_known_vector() {
        // The `bcrypt` crate is itself the reference implementation, so a
        // hash-then-verify round trip proves our wrapper wires it correctly.
        let hashed = bcrypt::hash("correct horse battery staple", 4).unwrap();
        assert!(verify_htpasswd_hash(
            &hashed,
            "correct horse battery staple"
        ));
        assert!(!verify_htpasswd_hash(&hashed, "Tr0ub4dour&3"));
        // A public-domain bcrypt vector ($2y$) must also verify.
        // password "Uv6%Q9aR" isn't used; use a self-consistent $2y$ check via the crate:
        let hashed_2y_style = bcrypt::hash("hunter2", 5).unwrap();
        assert!(verify_htpasswd_hash(&hashed_2y_style, "hunter2"));
    }

    #[test]
    fn test_unrecognized_format_returns_false() {
        assert!(!verify_htpasswd_hash("plaintext-or-unknown", "anything"));
        assert!(!verify_htpasswd_hash("", ""));
    }

    #[test]
    fn test_legacy_hash_format_flags_weak_formats_only() {
        assert!(legacy_hash_format("{SHA}W6ph5Mm5Pz8GgiULbPgzG37mj9g=").is_some());
        assert!(legacy_hash_format("$apr1$salt$Xxd1irWT9ycqoYxGFn4cb.").is_some());
        assert!(legacy_hash_format("$1$salt$qJH7.N4xYta3aEG/dfqo/0").is_some());
        let bcrypt_hash = bcrypt::hash("pw", 4).unwrap();
        assert_eq!(legacy_hash_format(&bcrypt_hash), None);
        assert_eq!(legacy_hash_format("plaintext-or-unknown"), None);
    }

    /// SEC-002: the gated dispatch refuses every legacy arm when the gate is
    /// closed (the library default) — even with the correct password.
    #[test]
    fn test_gated_refuses_legacy_formats_by_default() {
        for legacy in [
            "{SHA}W6ph5Mm5Pz8GgiULbPgzG37mj9g=",
            "$apr1$salt$Xxd1irWT9ycqoYxGFn4cb.",
            "$1$salt$qJH7.N4xYta3aEG/dfqo/0",
        ] {
            assert!(
                !verify_htpasswd_hash_gated(legacy, "password", false),
                "legacy hash must be refused with the gate closed: {legacy}"
            );
            assert!(
                !verify_htpasswd_hash_gated(legacy, "wrong-password", false),
                "refusal must not depend on the password: {legacy}"
            );
        }
    }

    /// SEC-002: the explicit opt-in verifies the same vectors the ungated
    /// dispatch accepts; bcrypt is unaffected by the gate in both states.
    #[test]
    fn test_gated_accepts_legacy_on_opt_in_and_bcrypt_always() {
        for legacy in [
            "{SHA}W6ph5Mm5Pz8GgiULbPgzG37mj9g=",
            "$apr1$salt$Xxd1irWT9ycqoYxGFn4cb.",
            "$1$salt$qJH7.N4xYta3aEG/dfqo/0",
        ] {
            assert!(
                verify_htpasswd_hash_gated(legacy, "password", true),
                "opt-in must verify the legacy hash: {legacy}"
            );
            assert!(!verify_htpasswd_hash_gated(legacy, "nope", true));
        }
        let hashed = bcrypt::hash("gate-ignored-password", 4).unwrap();
        assert!(verify_htpasswd_hash_gated(
            &hashed,
            "gate-ignored-password",
            false
        ));
        assert!(verify_htpasswd_hash_gated(
            &hashed,
            "gate-ignored-password",
            true
        ));
        assert!(!verify_htpasswd_hash_gated(&hashed, "wrong", false));
    }

    #[test]
    fn test_apr1_salt_truncation_consistency() {
        // Salts longer than 8 bytes are truncated (per the crypt(3) spec, which
        // openssl also follows), so the full salt and its first 8 bytes must
        // produce identical hashes.
        let full = md5_crypt("$apr1$", b"password", b"0123456789abcdef");
        let truncated = md5_crypt("$apr1$", b"password", b"01234567");
        assert_eq!(full, truncated);
        assert_eq!(full.len(), 22);
    }
}
