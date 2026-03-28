//! URL validation utilities for SSRF protection.
//!
//! Provides functions to validate URLs against security threats such as:
//! - Local file access via `file://` URLs
//! - SSRF attacks via localhost or private IP addresses
//! - DNS rebinding attacks via post-connect IP validation
//!
//! These functions are extracted as shared utilities so that any module
//! performing outbound HTTP requests can reuse the same validation logic.

use anyhow::{bail, Result};
use std::net::IpAddr;

/// Validates a URL for security requirements.
///
/// Ensures the URL uses an allowed scheme (`http` or `https`), and that the
/// host is not localhost or a private/reserved IP address.
///
/// When `allow_localhost` is `true`, localhost and private-IP checks are
/// bypassed. This should **only** be used in test configurations.
///
/// # Errors
///
/// Returns an error if:
/// - The URL cannot be parsed
/// - The URL uses a disallowed scheme (e.g. `file://`)
/// - The URL points to localhost or a private IP range (when `allow_localhost` is `false`)
///
/// # Examples
///
/// ```
/// use patina::util::url_validation::validate_url;
///
/// // Public HTTPS URL is accepted
/// let url = validate_url("https://example.com", false);
/// assert!(url.is_ok());
///
/// // Localhost is rejected by default
/// let url = validate_url("http://127.0.0.1/path", false);
/// assert!(url.is_err());
/// ```
#[must_use = "the validated URL should be used for the subsequent request"]
pub fn validate_url(url: &str, allow_localhost: bool) -> Result<reqwest::Url> {
    let parsed = reqwest::Url::parse(url)?;

    // Check scheme - only allow http and https
    match parsed.scheme() {
        "http" | "https" => {}
        "file" => bail!("file:// URLs are not allowed for security reasons"),
        scheme => bail!("URL scheme '{}' is not allowed", scheme),
    }

    // Check for localhost and private IPs
    if let Some(host) = parsed.host_str() {
        if is_localhost(host) && !allow_localhost {
            bail!("Localhost URLs are not allowed for security reasons");
        }

        // Check for private IP addresses (always blocked unless allow_localhost)
        if is_private_ip(host) && !allow_localhost {
            bail!("Private IP addresses are not allowed for security reasons");
        }

        // Warn when using unencrypted HTTP for non-localhost URLs.
        // Auth tokens transmitted over HTTP can be intercepted.
        if parsed.scheme() == "http" && !is_localhost(host) {
            tracing::warn!(
                url = %url,
                "MCP connection using unencrypted HTTP \u{2014} consider using HTTPS"
            );
        }
    }

    Ok(parsed)
}

/// Checks if a host string refers to localhost.
///
/// Matches `localhost`, `127.x.x.x`, `::1`, and `[::1]` (case-insensitive).
///
/// # Examples
///
/// ```
/// use patina::util::url_validation::is_localhost;
///
/// assert!(is_localhost("localhost"));
/// assert!(is_localhost("127.0.0.1"));
/// assert!(!is_localhost("example.com"));
/// ```
#[must_use]
pub fn is_localhost(host: &str) -> bool {
    let host_lower = host.to_lowercase();
    host_lower == "localhost"
        || host_lower == "127.0.0.1"
        || host_lower == "::1"
        || host_lower == "[::1]"
        || host_lower.starts_with("127.")
}

/// Checks if a host string is a private IP address.
///
/// Detects RFC 1918 private ranges (`10.0.0.0/8`, `172.16.0.0/12`,
/// `192.168.0.0/16`), link-local (`169.254.0.0/16`), and IPv6 private
/// ranges (loopback, link-local `fe80::/10`, unique local `fc00::/7`).
///
/// Non-IP hostnames (e.g. domain names) return `false`.
///
/// # Examples
///
/// ```
/// use patina::util::url_validation::is_private_ip;
///
/// assert!(is_private_ip("10.0.0.1"));
/// assert!(is_private_ip("192.168.1.1"));
/// assert!(!is_private_ip("8.8.8.8"));
/// assert!(!is_private_ip("example.com"));
/// ```
#[must_use]
pub fn is_private_ip(host: &str) -> bool {
    // Try to parse as IP address
    let ip: Option<IpAddr> = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .ok();

    match ip {
        Some(IpAddr::V4(ipv4)) => {
            // Private IPv4 ranges:
            // 10.0.0.0/8
            // 172.16.0.0/12
            // 192.168.0.0/16
            // 169.254.0.0/16 (link-local)
            let octets = ipv4.octets();
            octets[0] == 10
                || (octets[0] == 172 && (16..=31).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 168)
                || (octets[0] == 169 && octets[1] == 254)
        }
        Some(IpAddr::V6(ipv6)) => {
            // Private/local IPv6:
            // ::1 (loopback)
            // fe80::/10 (link-local)
            // fc00::/7 (unique local)
            ipv6.is_loopback()
                || ((ipv6.segments()[0] & 0xffc0) == 0xfe80) // link-local
                || ((ipv6.segments()[0] & 0xfe00) == 0xfc00) // unique local
        }
        None => false,
    }
}

/// Checks if a resolved IP address is in a private or reserved range.
///
/// Operates on [`std::net::IpAddr`] directly (unlike [`is_private_ip`] which
/// parses a host string). This is intended for post-connect DNS rebinding
/// validation, where the resolved IP address is already available.
///
/// # Examples
///
/// ```
/// use std::net::{IpAddr, Ipv4Addr};
/// use patina::util::url_validation::is_private_ip_addr;
///
/// assert!(is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
/// assert!(!is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
/// ```
#[must_use]
pub fn is_private_ip_addr(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            octets[0] == 10
                || (octets[0] == 172 && (16..=31).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 168)
                || (octets[0] == 169 && octets[1] == 254)
        }
        IpAddr::V6(ipv6) => {
            ipv6.is_loopback()
                || ((ipv6.segments()[0] & 0xffc0) == 0xfe80) // link-local
                || ((ipv6.segments()[0] & 0xfe00) == 0xfc00) // unique local
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn test_validate_url_rejects_private_ip() {
        // AWS metadata endpoint (169.254.169.254) must be rejected
        let result = validate_url("http://169.254.169.254/latest/meta-data/", false);
        assert!(result.is_err(), "169.254.169.254 should be rejected");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Private IP"),
            "Error should mention private IP: {err}"
        );
    }

    #[test]
    fn test_validate_url_rejects_localhost() {
        // 127.0.0.1
        let result = validate_url("http://127.0.0.1/secret", false);
        assert!(result.is_err(), "127.0.0.1 should be rejected");

        // localhost
        let result = validate_url("http://localhost/secret", false);
        assert!(result.is_err(), "localhost should be rejected");
    }

    #[test]
    fn test_validate_url_rejects_file_scheme() {
        let result = validate_url("file:///etc/passwd", false);
        assert!(result.is_err(), "file:// scheme should be rejected");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("file://"),
            "Error should mention file:// scheme: {err}"
        );
    }

    #[test]
    fn test_validate_url_accepts_public_https() {
        let result = validate_url("https://api.example.com", false);
        assert!(result.is_ok(), "Public HTTPS URL should be accepted");
        let url = result.expect("URL should parse successfully");
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("api.example.com"));
    }

    #[test]
    fn test_validate_url_rejects_ipv6_loopback() {
        let result = validate_url("http://[::1]:8080/path", false);
        assert!(result.is_err(), "IPv6 loopback [::1] should be rejected");
    }

    #[test]
    fn test_is_private_ip_rejects_rfc1918() {
        // 10.0.0.0/8
        assert!(is_private_ip("10.0.0.1"), "10.0.0.1 is RFC 1918 private");

        // 172.16.0.0/12
        assert!(
            is_private_ip("172.16.0.1"),
            "172.16.0.1 is RFC 1918 private"
        );

        // 192.168.0.0/16
        assert!(
            is_private_ip("192.168.1.1"),
            "192.168.1.1 is RFC 1918 private"
        );
    }

    #[test]
    fn test_is_localhost_various() {
        assert!(is_localhost("localhost"));
        assert!(is_localhost("127.0.0.1"));
        assert!(is_localhost("127.0.0.2"));
        assert!(is_localhost("::1"));
        assert!(is_localhost("[::1]"));
        assert!(!is_localhost("example.com"));
        assert!(!is_localhost("8.8.8.8"));
    }

    #[test]
    fn test_is_private_ip_various() {
        assert!(is_private_ip("10.0.0.1"));
        assert!(is_private_ip("10.255.255.255"));
        assert!(is_private_ip("172.16.0.1"));
        assert!(is_private_ip("172.31.255.255"));
        assert!(is_private_ip("192.168.1.1"));
        assert!(is_private_ip("192.168.0.1"));
        assert!(is_private_ip("169.254.1.1"));

        // Not private
        assert!(!is_private_ip("8.8.8.8"));
        assert!(!is_private_ip("172.32.0.1")); // Just outside 172.16-31 range
        assert!(!is_private_ip("example.com"));
    }

    #[test]
    fn test_is_private_ip_addr_v4_ranges() {
        assert!(is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));
        assert!(is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(
            192, 168, 1, 1
        ))));
        assert!(is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(
            169, 254, 0, 1
        ))));

        // Public
        assert!(!is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_private_ip_addr(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
    }

    #[test]
    fn test_is_private_ip_addr_v6_ranges() {
        // Loopback
        assert!(is_private_ip_addr(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        // Link-local (fe80::/10)
        assert!(is_private_ip_addr(IpAddr::V6(Ipv6Addr::new(
            0xfe80, 0, 0, 0, 0, 0, 0, 1
        ))));
        // Unique local (fc00::/7)
        assert!(is_private_ip_addr(IpAddr::V6(Ipv6Addr::new(
            0xfc00, 0, 0, 0, 0, 0, 0, 1
        ))));

        // Public
        assert!(!is_private_ip_addr(IpAddr::V6(Ipv6Addr::new(
            0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888
        ))));
    }

    #[test]
    fn test_validate_url_allows_localhost_when_configured() {
        let result = validate_url("http://127.0.0.1/test", true);
        assert!(
            result.is_ok(),
            "Localhost should be allowed when allow_localhost is true"
        );
    }

    #[test]
    fn test_validate_url_rejects_unknown_scheme() {
        let result = validate_url("ftp://example.com/file", false);
        assert!(result.is_err(), "ftp:// scheme should be rejected");
    }

    #[test]
    fn test_validate_url_accepts_http() {
        let result = validate_url("http://example.com", false);
        assert!(result.is_ok(), "Plain HTTP should be accepted");
    }

    #[test]
    fn test_validate_url_http_non_localhost_triggers_warning_path() {
        // HTTP to a non-localhost URL should succeed but hits the warning path.
        // We verify the URL is accepted (the tracing::warn fires in production).
        let result = validate_url("http://remote-server.example.com/mcp", false);
        assert!(
            result.is_ok(),
            "HTTP non-localhost should be accepted (with warning)"
        );
        let url = result.unwrap();
        assert_eq!(url.scheme(), "http");
    }

    #[test]
    fn test_validate_url_https_does_not_trigger_warning_path() {
        // HTTPS should not trigger the HTTP warning path
        let result = validate_url("https://secure-server.example.com/mcp", false);
        assert!(result.is_ok(), "HTTPS should be accepted without warning");
        let url = result.unwrap();
        assert_eq!(url.scheme(), "https");
    }

    #[test]
    fn test_validate_url_http_localhost_no_warning() {
        // HTTP to localhost is fine (no auth token exposure risk on loopback)
        let result = validate_url("http://localhost:8080/mcp", true);
        assert!(
            result.is_ok(),
            "HTTP to localhost should be accepted without warning"
        );
    }
}
