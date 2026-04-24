use std::net::{IpAddr, Ipv4Addr};

use url::Url;

/// Validate that a URL is safe to fetch. Mirrors
/// `zunel/security/network.py::validate_url_target`.
pub fn validate_url_target(url: &str, allow_loopback: bool) -> Result<Url, String> {
    let parsed = Url::parse(url).map_err(|e| format!("invalid url: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!(
            "scheme must be http or https, got {}",
            parsed.scheme()
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "url missing host".to_string())?
        .to_string();
    if !allow_loopback {
        if let Ok(ip) = host.parse::<IpAddr>() {
            if is_blocked_ip(&ip) {
                return Err(format!("ssrf blocked ip: {ip}"));
            }
        } else if host.eq_ignore_ascii_case("localhost") {
            return Err("ssrf blocked: localhost".to_string());
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
