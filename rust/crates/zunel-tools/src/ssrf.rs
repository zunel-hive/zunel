use std::net::{IpAddr, Ipv4Addr};

use url::Url;

use crate::error::{Error, Result};

/// Validate that a URL is safe to fetch. Mirrors
/// `zunel/security/network.py::validate_url_target`.
///
/// `tool` is captured into the resulting error so a single shared validator
/// reports a useful provenance ("web_fetch", "web_search", …) on the
/// diagnostic line.
pub fn validate_url_target(url: &str, allow_loopback: bool, tool: &str) -> Result<Url> {
    let parsed = Url::parse(url).map_err(|e| Error::InvalidArgs {
        tool: tool.to_string(),
        message: format!("invalid url: {e}"),
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(Error::PolicyViolation {
            tool: tool.to_string(),
            reason: format!("scheme must be http or https, got {}", parsed.scheme()),
        });
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| Error::InvalidArgs {
            tool: tool.to_string(),
            message: "url missing host".to_string(),
        })?
        .to_string();
    if !allow_loopback {
        if let Ok(ip) = host.parse::<IpAddr>() {
            if is_blocked_ip(&ip) {
                return Err(Error::SsrfBlocked {
                    tool: tool.to_string(),
                    url: url.to_string(),
                    reason: format!("blocked ip: {ip}"),
                });
            }
        } else if host.eq_ignore_ascii_case("localhost") {
            return Err(Error::SsrfBlocked {
                tool: tool.to_string(),
                url: url.to_string(),
                reason: "localhost".to_string(),
            });
        }
    }
    Ok(parsed)
}

fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || *v4 == Ipv4Addr::new(169, 254, 169, 254)
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified() || is_unique_local_v6(v6),
    }
}

// Unique Local Addresses (fc00::/7) per RFC 4193. Kept as a manual
// check because `Ipv6Addr::is_unique_local` was stabilised in 1.84
// while our workspace MSRV is 1.82.
fn is_unique_local_v6(v6: &std::net::Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xfe00) == 0xfc00
}
