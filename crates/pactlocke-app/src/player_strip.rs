//! How the Player strip says another Player's app is reached: straight or
//! through a relay with the ping, through the Host, or offline.

/// How this app reaches another Player's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    Direct {
        ping_ms: u32,
    },
    Relayed {
        ping_ms: u32,
    },
    /// A joiner reaches the other joiners through the Host.
    ViaHost,
    /// Not heard from for a while: their game is frozen here until they are back.
    Offline,
}

/// The connection as the strip shows it; nothing on this Player's own row.
pub fn reach_text(reach: Option<Reach>, you: bool) -> String {
    match reach {
        _ if you => String::new(),
        Some(Reach::Direct { ping_ms }) => format!("direct · {ping_ms} ms"),
        Some(Reach::Relayed { ping_ms }) => format!("relayed · {ping_ms} ms"),
        Some(Reach::ViaHost) => "via the Host".to_owned(),
        Some(Reach::Offline) => "offline".to_owned(),
        None => "connecting…".to_owned(),
    }
}
