//! Answers apps looking for servers on the local network: they send
//! `who is JellyfinServer?` to UDP port 7359 and list whoever replies.

use std::net::{IpAddr, SocketAddr};

use serde_json::json;
use tokio::net::UdpSocket;

use crate::state::AppState;

pub const PORT: u16 = 7359;

/// The reply: where the app reaches the server, as seen from the app.
pub fn reply(local: IpAddr, http_port: u16, id: &str) -> String {
    let address = match local {
        IpAddr::V6(v6) => format!("http://[{v6}]:{http_port}/jellyfin"),
        IpAddr::V4(v4) => format!("http://{v4}:{http_port}/jellyfin"),
    };
    json!({ "Address": address, "Id": id, "Name": "Moka", "EndpointAddress": null }).to_string()
}

/// The address of this machine on the way to `peer`.
async fn local_ip_towards(peer: SocketAddr) -> Option<IpAddr> {
    let probe = UdpSocket::bind(if peer.is_ipv6() { "[::]:0" } else { "0.0.0.0:0" }).await.ok()?;
    probe.connect(peer).await.ok()?;
    Some(probe.local_addr().ok()?.ip())
}

/// Runs forever. When the port is taken (another media server on this
/// machine), apps still reach Moka by its address.
pub async fn run(state: AppState) {
    let socket = match UdpSocket::bind(("0.0.0.0", PORT)).await {
        Ok(s) => s,
        Err(e) => {
            tracing::info!(error = %e, "app discovery is off: UDP port {PORT} is in use");
            return;
        }
    };
    let mut buf = [0u8; 256];
    loop {
        let Ok((n, peer)) = socket.recv_from(&mut buf).await else { continue };
        let asked = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();
        if !asked.contains("who is jellyfinserver") {
            continue;
        }
        let Some(ip) = local_ip_towards(peer).await else { continue };
        let Ok(id) = super::server_id(&state).await else { continue };
        let _ = socket.send_to(reply(ip, state.config.listen.port(), &id).as_bytes(), peer).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_with_the_app_address() {
        let r: serde_json::Value =
            serde_json::from_str(&reply("192.168.1.5".parse().unwrap(), 8484, "abc")).unwrap();
        assert_eq!(r["Address"], "http://192.168.1.5:8484/jellyfin");
        assert_eq!((r["Id"].as_str(), r["Name"].as_str()), (Some("abc"), Some("Moka")));
    }
}
