//! Outgoing HTTP for user-supplied addresses (indexers, download clients,
//! webhooks, links inside indexer feeds).
//!
//! Home-network and loopback addresses are allowed: a self-hosted server
//! normally talks to qBittorrent on the same box or LAN. What is refused is
//! what an attacker would aim at through a crafted address or redirect:
//! cloud metadata services, link-local, multicast and "this host" ranges.
//! The check runs after DNS resolution and on every redirect, so a hostname
//! or redirect pointing at those ranges fails too.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::redirect::Policy;

/// True for addresses no outgoing request should reach.
pub fn is_forbidden(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => forbidden_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return forbidden_v4(v4);
            }
            let s = v6.segments();
            v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xffc0) == 0xfe80 // link-local
                || v6 == Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254) // AWS metadata over IPv6
        }
    }
}

fn forbidden_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 0 // "this network"
        || ip.is_link_local() // 169.254.0.0/16, incl. 169.254.169.254 metadata
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip == Ipv4Addr::new(100, 100, 100, 200) // Alibaba Cloud metadata
}

/// Error for an address that resolves only to forbidden ranges.
#[derive(Debug)]
pub struct Blocked(pub String);

impl std::fmt::Display for Blocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} points to an address the server will not contact", self.0)
    }
}

impl std::error::Error for Blocked {}

/// DNS resolver that drops forbidden addresses from every lookup.
struct SafeResolver;

impl Resolve for SafeResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            let allowed: Vec<SocketAddr> = addrs.into_iter().filter(|a| !is_forbidden(a.ip())).collect();
            if allowed.is_empty() {
                return Err(Box::new(Blocked(host)) as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(allowed.into_iter()) as Addrs)
        })
    }
}

/// Checks the host of a URL when it is a literal IP (DNS is not consulted for those).
pub fn check_url(url: &reqwest::Url) -> Result<(), Blocked> {
    let host = url.host_str().unwrap_or("").trim_start_matches('[').trim_end_matches(']');
    match host.parse::<IpAddr>() {
        Ok(ip) if is_forbidden(ip) => Err(Blocked(ip.to_string())),
        _ => Ok(()),
    }
}

/// Parses and checks an address typed by the user.
pub fn check_str(s: &str) -> Result<(), Blocked> {
    match reqwest::Url::parse(s) {
        Ok(u) => check_url(&u),
        Err(_) => Ok(()),
    }
}

/// Client builder with the resolver and a redirect policy that re-checks every hop.
pub fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().dns_resolver(Arc::new(SafeResolver)).redirect(Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 {
            attempt.error("too many redirects")
        } else if check_url(attempt.url()).is_err() {
            attempt.error("redirect to a refused address")
        } else {
            attempt.follow()
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        let bad = [
            "169.254.169.254",
            "0.0.0.0",
            "224.0.0.1",
            "255.255.255.255",
            "100.100.100.200",
            "fe80::1",
            "::",
            "::ffff:169.254.169.254",
            "fd00:ec2::254",
        ];
        for b in bad {
            assert!(is_forbidden(b.parse().unwrap()), "{b} should be refused");
        }
        let ok = ["127.0.0.1", "192.168.1.10", "10.0.0.5", "172.17.0.2", "8.8.8.8", "::1", "fd12::1"];
        for g in ok {
            assert!(!is_forbidden(g.parse().unwrap()), "{g} should be allowed");
        }
    }

    #[test]
    fn literal_urls() {
        assert!(check_str("http://169.254.169.254/latest/meta-data").is_err());
        assert!(check_str("http://[fe80::1]:8080/").is_err());
        assert!(check_str("http://192.168.1.2:8080").is_ok());
        assert!(check_str("http://qbittorrent:8080").is_ok());
    }

    #[tokio::test]
    async fn redirects_to_refused_addresses_fail() {
        use axum::{Router, http::StatusCode, routing::get};
        let app = Router::new().route(
            "/go",
            get(|| async { (StatusCode::FOUND, [("location", "http://169.254.169.254/latest/meta-data")]) }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = client_builder().build().unwrap();
        let err = client.get(format!("http://{addr}/go")).send().await.unwrap_err();
        assert!(err.is_redirect(), "{err:?}");
    }
}
