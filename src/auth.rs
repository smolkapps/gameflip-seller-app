//! Gameflip request authorization.
//!
//! Gameflip's REST API authenticates each request with a header of the form:
//!
//! ```text
//! Authorization: GFAPI <api_key>:<totp>
//! ```
//!
//! where `<totp>` is an RFC 6238 time-based one-time password derived from the
//! account's OTP secret. The reference Node bindings generate it with Speakeasy
//! using `{ encoding: "base32", algorithm: "sha1", digits: 6, period: 30 }`, so
//! this module implements exactly those parameters: a base32-decoded secret,
//! HMAC-SHA1, 6 digits, and a 30-second time step.

use hmac::{Hmac, Mac};
use sha1::Sha1;

use crate::error::{Error, Result};

type HmacSha1 = Hmac<Sha1>;

/// Number of digits in the generated code. Gameflip uses 6.
const TOTP_DIGITS: u32 = 6;
/// Time step in seconds. Gameflip uses 30.
const TOTP_PERIOD: u64 = 30;

/// Holds the seller's API credentials and produces signed `Authorization`
/// headers on demand.
#[derive(Clone)]
pub struct Credentials {
    api_key: String,
    /// Raw bytes of the OTP shared secret (already base32-decoded).
    secret: Vec<u8>,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the secret or full key.
        f.debug_struct("Credentials")
            .field("api_key", &masked(&self.api_key))
            .field("secret", &"<redacted>")
            .finish()
    }
}

fn masked(key: &str) -> String {
    if key.len() <= 6 {
        "***".to_string()
    } else {
        format!("{}…", &key[..6])
    }
}

impl Credentials {
    /// Build credentials from an API key and a base32-encoded OTP secret
    /// (the value Gameflip shows on the Settings page). Surrounding
    /// whitespace and any internal spaces in the secret are tolerated, and
    /// base32 padding is optional.
    pub fn new(api_key: impl Into<String>, base32_secret: &str) -> Result<Self> {
        let secret = decode_base32_secret(base32_secret)?;
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(Error::Config("API key is empty".into()));
        }
        Ok(Self {
            api_key: api_key.trim().to_string(),
            secret,
        })
    }

    /// The API key, used to select the API base URL by its environment prefix.
    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// Produce the value for the `Authorization` header at the given unix time
    /// (seconds). Splitting time out as a parameter keeps this deterministic and
    /// testable; [`Self::authorization_header`] uses the real clock.
    pub fn authorization_header_at(&self, unix_seconds: u64) -> String {
        let code = totp_at(&self.secret, unix_seconds);
        format!("GFAPI {}:{}", self.api_key, code)
    }

    /// Produce the `Authorization` header value using the current wall clock.
    pub fn authorization_header(&self) -> String {
        let now = time::OffsetDateTime::now_utc().unix_timestamp().max(0) as u64;
        self.authorization_header_at(now)
    }
}

/// Decode a user-supplied base32 OTP secret into raw bytes.
///
/// Gameflip secrets are RFC 4648 base32 (Speakeasy's default). We accept the
/// secret with or without `=` padding and ignore spaces, since the Settings
/// page sometimes groups the characters for readability.
pub fn decode_base32_secret(input: &str) -> Result<Vec<u8>> {
    let cleaned: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.is_empty() {
        return Err(Error::Config("OTP secret is empty".into()));
    }
    let upper = cleaned.to_ascii_uppercase();
    // Pad to a multiple of 8 so callers can paste an unpadded secret.
    let padded = pad_base32(&upper);
    data_encoding::BASE32
        .decode(padded.as_bytes())
        .map_err(|e| Error::Config(format!("OTP secret is not valid base32: {e}")))
}

fn pad_base32(s: &str) -> String {
    let rem = s.len() % 8;
    if rem == 0 {
        s.to_string()
    } else {
        let mut out = String::with_capacity(s.len() + (8 - rem));
        out.push_str(s);
        for _ in 0..(8 - rem) {
            out.push('=');
        }
        out
    }
}

/// Compute the RFC 6238 TOTP for the given secret at `unix_seconds`, with
/// Gameflip's parameters (SHA1, 6 digits, 30s step).
pub fn totp_at(secret: &[u8], unix_seconds: u64) -> String {
    let counter = unix_seconds / TOTP_PERIOD;
    hotp(secret, counter, TOTP_DIGITS)
}

/// RFC 4226 HOTP. Factored out so it can be tested directly against the RFC
/// reference vectors.
pub fn hotp(secret: &[u8], counter: u64, digits: u32) -> String {
    let msg = counter.to_be_bytes();
    let mut mac = HmacSha1::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(&msg);
    let hash = mac.finalize().into_bytes();

    // Dynamic truncation (RFC 4226 §5.3).
    let offset = (hash[hash.len() - 1] & 0x0f) as usize;
    let bin_code = ((hash[offset] as u32 & 0x7f) << 24)
        | ((hash[offset + 1] as u32) << 16)
        | ((hash[offset + 2] as u32) << 8)
        | (hash[offset + 3] as u32);

    let modulo = 10u32.pow(digits);
    let code = bin_code % modulo;
    format!("{code:0width$}", width = digits as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 4226 Appendix D reference vectors. Secret is the ASCII string
    // "12345678901234567890". The first ten HOTP values are well known.
    const RFC4226_SECRET: &[u8] = b"12345678901234567890";
    const RFC4226_HOTP: [&str; 10] = [
        "755224", "287082", "359152", "969429", "338314", "254676", "287922", "162583", "399871",
        "520489",
    ];

    #[test]
    fn hotp_matches_rfc4226_vectors() {
        for (counter, expected) in RFC4226_HOTP.iter().enumerate() {
            let got = hotp(RFC4226_SECRET, counter as u64, 6);
            assert_eq!(&got, expected, "HOTP mismatch at counter {counter}");
        }
    }

    #[test]
    fn totp_matches_rfc6238_sha1_vector() {
        // RFC 6238 Appendix B: with the SHA1 seed (same 20-byte ASCII secret)
        // and T = 59s, the 8-digit TOTP is 94287082. The trailing 6 digits are
        // what a 6-digit configuration yields.
        let code = totp_at(RFC4226_SECRET, 59);
        assert_eq!(code, "287082");

        // T = 1111111109 -> 8-digit 07081804 -> 6-digit 081804.
        let code = totp_at(RFC4226_SECRET, 1_111_111_109);
        assert_eq!(code, "081804");
    }

    #[test]
    fn totp_is_stable_within_a_period_and_changes_across_periods() {
        let secret = decode_base32_secret("JBSWY3DPEHPK3PXP").unwrap();
        // Use a period-aligned base so both instants fall in the same window.
        let base = (1_700_000_000u64 / TOTP_PERIOD) * TOTP_PERIOD; // window start
        let a = totp_at(&secret, base);
        let b = totp_at(&secret, base + TOTP_PERIOD - 1); // last second of window
        assert_eq!(a, b, "codes must be identical within a 30s window");
        // Crossing into the next window changes the code.
        let c = totp_at(&secret, base + TOTP_PERIOD);
        assert_ne!(a, c, "code must change at the window boundary");
    }

    #[test]
    fn authorization_header_has_expected_shape() {
        let creds = Credentials::new("test-myapikey123", "JBSWY3DPEHPK3PXP").unwrap();
        let header = creds.authorization_header_at(1_700_000_000);
        assert!(header.starts_with("GFAPI test-myapikey123:"));
        let code = header.rsplit(':').next().unwrap();
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn base32_secret_tolerates_spaces_lowercase_and_missing_padding() {
        // Same secret three ways must decode identically.
        let canonical = decode_base32_secret("JBSWY3DPEHPK3PXP").unwrap();
        let spaced = decode_base32_secret("jbsw y3dp ehpk 3pxp").unwrap();
        assert_eq!(canonical, spaced);
    }

    #[test]
    fn empty_secret_is_rejected() {
        assert!(decode_base32_secret("   ").is_err());
    }

    #[test]
    fn invalid_base32_is_rejected() {
        // '1', '8', '9', '0' are not in the RFC 4648 base32 alphabet.
        assert!(decode_base32_secret("11110000").is_err());
    }

    #[test]
    fn debug_does_not_leak_secret_or_full_key() {
        let creds = Credentials::new("test-supersecretkey", "JBSWY3DPEHPK3PXP").unwrap();
        let dbg = format!("{creds:?}");
        assert!(dbg.contains("redacted"));
        assert!(!dbg.contains("supersecretkey"));
        assert!(!dbg.contains("JBSWY3DPEHPK3PXP"));
    }
}
