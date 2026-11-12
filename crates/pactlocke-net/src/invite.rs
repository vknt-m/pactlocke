//! Invite links: the Host's key, the relay to reach them on, a single-use
//! 128-bit Run secret and the Host's app version. Never an IP address.

use std::fmt;

/// An app's identity on the network: its endpoint's public key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PeerKey(pub [u8; 32]);

impl fmt::Display for PeerKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex(&self.0))
    }
}

impl fmt::Debug for PeerKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PeerKey({})", &hex(&self.0)[..8])
    }
}

/// The single-use secret an invite carries: whoever brings it may join, once.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RunSecret(pub [u8; 16]);

impl RunSecret {
    /// 128 bits from the operating system's random source.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).expect("the operating system's random source works");
        Self(bytes)
    }

    /// Compares without stopping at the first difference.
    pub fn matches(&self, other: &[u8; 16]) -> bool {
        self.0.iter().zip(other).fold(0u8, |diff, (a, b)| diff | (a ^ b)) == 0
    }
}

impl fmt::Debug for RunSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RunSecret(…)")
    }
}

/// What a friend needs to join a Run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    pub host: PeerKey,
    /// The relay the Host is reachable on; `None` for a local test.
    pub relay: Option<String>,
    pub secret: RunSecret,
    /// The Host's Pactlocke version; a joiner must have the same one.
    pub app_version: String,
}

/// Why a link isn't an invite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InviteError {
    NotAnInvite,
    Missing(&'static str),
    Bad(&'static str),
}

impl fmt::Display for InviteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InviteError::NotAnInvite => write!(f, "that isn't a Pactlocke invite link"),
            InviteError::Missing(what) => write!(f, "the invite link has no {what}"),
            InviteError::Bad(what) => write!(f, "the invite link's {what} is damaged"),
        }
    }
}

impl std::error::Error for InviteError {}

const SCHEME: &str = "pactlocke://join?";

impl Invite {
    /// The link a Host shares, such as
    /// `pactlocke://join?host=…&secret=…&v=0.1.0&relay=https%3A%2F%2F…`.
    pub fn link(&self) -> String {
        let mut link = format!(
            "{SCHEME}host={}&secret={}&v={}",
            self.host,
            hex(&self.secret.0),
            escape(&self.app_version)
        );
        if let Some(relay) = &self.relay {
            link.push_str("&relay=");
            link.push_str(&escape(relay));
        }
        link
    }

    /// Reads a link a friend pasted (whitespace around it is ignored).
    pub fn parse(link: &str) -> Result<Self, InviteError> {
        let query = link.trim().strip_prefix(SCHEME).ok_or(InviteError::NotAnInvite)?;
        let mut host = None;
        let mut secret = None;
        let mut version = None;
        let mut relay = None;
        for pair in query.split('&') {
            let (key, value) = pair.split_once('=').ok_or(InviteError::NotAnInvite)?;
            match key {
                "host" => host = Some(value),
                "secret" => secret = Some(value),
                "v" => version = Some(unescape(value).ok_or(InviteError::Bad("version"))?),
                "relay" => relay = Some(unescape(value).ok_or(InviteError::Bad("relay"))?),
                _ => {}
            }
        }
        let host = unhex::<32>(host.ok_or(InviteError::Missing("Host key"))?).ok_or(InviteError::Bad("Host key"))?;
        let secret =
            unhex::<16>(secret.ok_or(InviteError::Missing("Run secret"))?).ok_or(InviteError::Bad("Run secret"))?;
        Ok(Self {
            host: PeerKey(host),
            relay,
            secret: RunSecret(secret),
            app_version: version.ok_or(InviteError::Missing("version"))?,
        })
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N * 2 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// Percent-encodes everything but letters, digits and `-._~`.
fn escape(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn unescape(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let pair = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(pair, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}
