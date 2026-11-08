//! Free TCP port selection: prefer the build default, bump upward if busy.

use std::net::{Ipv4Addr, TcpListener};

/// Probe-bind on 127.0.0.1; if closed for any reason, scan upward.
pub fn find_free_port(preferred: u16) -> u16 {
    let mut port = preferred;
    for _ in 0..200 {
        if TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok() {
            return port;
        }
        // Saturating bump; wrap high ports start at 8642/3142 so overflow is
        // unlikely, but guarantee forward motion.
        port = port.saturating_add(1).max(1024);
        if port == 0 {
            port = 1024;
        }
    }
    preferred // last resort; the listen will fail loudly downstream
}
