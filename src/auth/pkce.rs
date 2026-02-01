//! PKCE (Proof Key for Code Exchange) implementation.
//!
//! PKCE is a security extension to OAuth 2.0 that protects against
//! authorization code interception attacks.
//!
//! # Example
//!
//! ```
//! use patina::auth::pkce::PkceChallenge;
//!
//! let pkce = PkceChallenge::generate();
//! println!("Code verifier: {}", pkce.verifier());
//! println!("Code challenge: {}", pkce.challenge());
//! ```

use base64::engine::{general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::prelude::*;
use sha2::{Digest, Sha256};

/// Length of the code verifier in bytes (before base64 encoding).
///
/// RFC 7636 recommends 32 octets of random data.
const VERIFIER_LENGTH: usize = 32;

/// PKCE challenge for OAuth 2.0 authorization.
///
/// Contains both the code verifier (kept secret) and the code challenge
/// (sent to the authorization server).
#[derive(Debug, Clone)]
pub struct PkceChallenge {
    /// The code verifier - a random string kept by the client.
    verifier: String,
    /// The code challenge - SHA256 hash of the verifier, base64url encoded.
    challenge: String,
}

impl PkceChallenge {
    /// Generates a new PKCE challenge.
    ///
    /// Creates a cryptographically random code verifier and computes
    /// the corresponding code challenge using SHA256 + base64url.
    #[must_use]
    pub fn generate() -> Self {
        let verifier = generate_verifier();
        let challenge = compute_challenge(&verifier);
        Self {
            verifier,
            challenge,
        }
    }

    /// Creates a PKCE challenge from an existing verifier.
    ///
    /// Useful for testing or when the verifier is stored externally.
    #[must_use]
    pub fn from_verifier(verifier: String) -> Self {
        let challenge = compute_challenge(&verifier);
        Self {
            verifier,
            challenge,
        }
    }

    /// Returns the code verifier.
    ///
    /// This should be kept secret and sent only during the token exchange.
    #[must_use]
    pub fn verifier(&self) -> &str {
        &self.verifier
    }

    /// Returns the code challenge.
    ///
    /// This is sent to the authorization server during the authorization request.
    #[must_use]
    pub fn challenge(&self) -> &str {
        &self.challenge
    }

    /// Returns the challenge method ("S256").
    #[must_use]
    pub fn challenge_method(&self) -> &'static str {
        "S256"
    }
}

/// Generates a cryptographically random code verifier.
///
/// The verifier is base64url encoded to ensure it contains only
/// unreserved URI characters as required by RFC 7636.
fn generate_verifier() -> String {
    let mut rng = rand::thread_rng();
    let random_bytes: Vec<u8> = (0..VERIFIER_LENGTH).map(|_| rng.gen()).collect();
    URL_SAFE_NO_PAD.encode(random_bytes)
}

/// Computes the code challenge from a verifier.
///
/// Uses the S256 method: BASE64URL(SHA256(code_verifier))
fn compute_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    URL_SAFE_NO_PAD.encode(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pkce_generate() {
        let pkce = PkceChallenge::generate();

        // Verifier should be base64url encoded (43 chars for 32 bytes)
        assert_eq!(pkce.verifier().len(), 43);

        // Challenge should be base64url encoded SHA256 (43 chars)
        assert_eq!(pkce.challenge().len(), 43);

        // Method should be S256
        assert_eq!(pkce.challenge_method(), "S256");
    }

    #[test]
    fn test_pkce_uniqueness() {
        let pkce1 = PkceChallenge::generate();
        let pkce2 = PkceChallenge::generate();

        // Each generation should produce different values
        assert_ne!(pkce1.verifier(), pkce2.verifier());
        assert_ne!(pkce1.challenge(), pkce2.challenge());
    }

    #[test]
    fn test_pkce_from_verifier() {
        let verifier = "test-verifier-123456789012345678901234567890".to_string();
        let pkce = PkceChallenge::from_verifier(verifier.clone());

        assert_eq!(pkce.verifier(), verifier);
        // Challenge should be consistent for the same verifier
        let pkce2 = PkceChallenge::from_verifier(verifier);
        assert_eq!(pkce.challenge(), pkce2.challenge());
    }

    #[test]
    fn test_pkce_challenge_computation() {
        // Test with a known verifier to verify the challenge computation
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".to_string();
        let pkce = PkceChallenge::from_verifier(verifier);

        // The challenge should be a valid base64url string
        let challenge = pkce.challenge();
        assert!(!challenge.is_empty());
        assert!(!challenge.contains('+')); // No + in URL-safe base64
        assert!(!challenge.contains('/')); // No / in URL-safe base64
        assert!(!challenge.contains('=')); // No padding in URL-safe base64 (no pad)
    }

    #[test]
    fn test_verifier_character_set() {
        let pkce = PkceChallenge::generate();
        let verifier = pkce.verifier();

        // Verifier should only contain URL-safe characters
        for c in verifier.chars() {
            assert!(
                c.is_ascii_alphanumeric() || c == '-' || c == '_',
                "Invalid character in verifier: {c}"
            );
        }
    }

    #[test]
    fn test_challenge_is_deterministic() {
        // Same verifier should always produce the same challenge
        let verifier = "my-test-verifier-that-is-long-enough-for-testing".to_string();

        let challenge1 = compute_challenge(&verifier);
        let challenge2 = compute_challenge(&verifier);

        assert_eq!(challenge1, challenge2);
    }
}
