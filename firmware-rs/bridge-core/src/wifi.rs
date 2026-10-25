//! Hotspot credential validation (WPA2/WPA3-Personal).

use std::fmt;

pub const MAX_SSID_BYTES: usize = 32;
pub const MIN_PASS_LEN: usize = 8;
pub const MAX_PASS_LEN: usize = 63;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialError {
    SsidEmpty,
    SsidTooLong,
    PassLength,
    PassCharacters,
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SsidEmpty => "network name is empty",
            Self::SsidTooLong => "network name is longer than 32 bytes",
            Self::PassLength => "password must be 8 to 63 characters",
            Self::PassCharacters => "password may only contain printable ASCII characters",
        })
    }
}

impl std::error::Error for CredentialError {}

/// SSID: 1–32 bytes (UTF-8 allowed). Passphrase: 8–63 printable ASCII characters, per WPA2.
pub fn validate(ssid: &str, pass: &str) -> Result<(), CredentialError> {
    if ssid.is_empty() {
        return Err(CredentialError::SsidEmpty);
    }
    if ssid.len() > MAX_SSID_BYTES {
        return Err(CredentialError::SsidTooLong);
    }
    if !(MIN_PASS_LEN..=MAX_PASS_LEN).contains(&pass.len()) {
        return Err(CredentialError::PassLength);
    }
    if !pass.bytes().all(|b| (0x20..=0x7E).contains(&b)) {
        return Err(CredentialError::PassCharacters);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_credentials() {
        assert_eq!(validate("5G-Bridge", "correct horse"), Ok(()));
        assert_eq!(validate("Café ☕", &"x".repeat(63)), Ok(()));
    }

    #[test]
    fn rejects_invalid_credentials() {
        assert_eq!(validate("", "password1"), Err(CredentialError::SsidEmpty));
        assert_eq!(validate(&"s".repeat(33), "password1"), Err(CredentialError::SsidTooLong));
        assert_eq!(validate("Bridge", "short"), Err(CredentialError::PassLength));
        assert_eq!(validate("Bridge", &"x".repeat(64)), Err(CredentialError::PassLength));
        assert_eq!(validate("Bridge", "pässword1"), Err(CredentialError::PassCharacters));
        assert_eq!(validate("Bridge", "tab\there!"), Err(CredentialError::PassCharacters));
    }
}
