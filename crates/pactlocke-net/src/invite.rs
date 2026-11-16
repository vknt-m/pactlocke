//! Invite links: the Host's key, a single-use 128-bit Run secret and the
//! Host's app version, plus the relay to reach them on when it isn't one of
//! n0's default relays. Never an IP address.
//!
//! A link is `pactlocke:` and one unbroken block of 68 URL-safe characters
//! (the key, the secret and the version, in base64url), then `@relay` for a
//! custom relay. Links from before it, `pactlocke://join?host=…`, still read.

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
    /// The Host's key, which is also where to reach them.
    pub host: PeerKey,
    /// The relay the Host is reachable on; `None` for a local test.
    pub relay: Option<String>,
    /// The Run secret that proves this invite came from the Host.
    pub secret: RunSecret,
    /// The Host's Pactlocke version; a joiner must have the same one.
    pub app_version: String,
}

/// Why a link a Player pasted isn't an invite they can join with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InviteError {
    /// Not a Pactlocke link at all.
    NotAnInvite,
    /// A Pactlocke link, but cut short, added to or changed on the way.
    Damaged,
}

impl fmt::Display for InviteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InviteError::NotAnInvite => f.write_str("That isn't a Pactlocke invite"),
            InviteError::Damaged => f.write_str("That invite is damaged: ask your friend to copy it again"),
        }
    }
}

impl std::error::Error for InviteError {}

const SCHEME: &str = "pactlocke:";
/// The links from before the block, which still read.
const OLD_SCHEME: &str = "pactlocke://join?";
/// Where each part of the invite is in the block: the Host's key, the Run
/// secret, then the version's three numbers. 51 bytes are 68 characters.
const HOST: std::ops::Range<usize> = 0..32;
const SECRET: std::ops::Range<usize> = 32..48;
const VERSION: std::ops::Range<usize> = 48..51;
const BLOCK_BYTES: usize = VERSION.end;
/// A relay's scheme when the link leaves it out.
const HTTPS: &str = "https://";

impl Invite {
    /// The link a Host shares: `pactlocke:` and one block of 68 characters,
    /// then `@relay` if the Host is on a custom relay.
    pub fn link(&self) -> String {
        let Some(version) = version_bytes(&self.app_version) else {
            return self.old_link();
        };
        let mut bytes = Vec::with_capacity(BLOCK_BYTES);
        bytes.extend_from_slice(&self.host.0);
        bytes.extend_from_slice(&self.secret.0);
        bytes.extend_from_slice(&version);
        let mut link = format!("{SCHEME}{}", base64url(&bytes));
        if let Some(relay) = self.relay.as_deref().filter(|r| !is_default_relay(r)) {
            link.push('@');
            match relay.strip_prefix(HTTPS) {
                Some(rest) => link.push_str(rest.strip_suffix('/').unwrap_or(rest)),
                None => link.push_str(relay),
            }
        }
        link
    }

    /// The link as it was before the block, for a version the block can't hold.
    fn old_link(&self) -> String {
        let mut link = format!(
            "{OLD_SCHEME}host={}&secret={}&v={}",
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

    /// Reads a link a friend pasted. Whitespace anywhere in it is ignored, as
    /// chat apps add spaces and line breaks around and inside long words.
    pub fn parse(link: &str) -> Result<Self, InviteError> {
        let link: String = link.chars().filter(|c| !c.is_whitespace()).collect();
        if let Some(query) = link.strip_prefix(OLD_SCHEME) {
            return Self::parse_old(query).ok_or(InviteError::Damaged);
        }
        let rest = link.strip_prefix(SCHEME).ok_or(InviteError::NotAnInvite)?;
        Self::parse_block(rest).ok_or(InviteError::Damaged)
    }

    fn parse_block(rest: &str) -> Option<Self> {
        let (block, relay) = match rest.split_once('@') {
            Some((block, relay)) => (block, Some(relay)),
            None => (rest, None),
        };
        let bytes = unbase64url(block)?;
        if bytes.len() != BLOCK_BYTES {
            return None;
        }
        let relay = match relay {
            None => None,
            Some("") => return None,
            Some(relay) if relay.contains("://") => Some(relay.to_owned()),
            Some(relay) => Some(format!("{HTTPS}{relay}/")),
        };
        let [major, minor, patch] = bytes[VERSION].try_into().ok()?;
        Some(Self {
            host: PeerKey(bytes[HOST].try_into().ok()?),
            relay,
            secret: RunSecret(bytes[SECRET].try_into().ok()?),
            app_version: format!("{major}.{minor}.{patch}"),
        })
    }

    fn parse_old(query: &str) -> Option<Self> {
        let (mut host, mut secret, mut version, mut relay) = (None, None, None, None);
        for pair in query.split('&') {
            let (key, value) = pair.split_once('=')?;
            match key {
                "host" => host = Some(unhex::<32>(value)?),
                "secret" => secret = Some(unhex::<16>(value)?),
                "v" => version = Some(unescape(value)?),
                "relay" => relay = Some(unescape(value)?),
                _ => {}
            }
        }
        Some(Self {
            host: PeerKey(host?),
            relay,
            secret: RunSecret(secret?),
            app_version: version?,
        })
    }
}

/// A version such as `0.1.0` as three bytes, if it is three numbers that fit.
fn version_bytes(version: &str) -> Option<[u8; 3]> {
    let mut numbers = version
        .split('.')
        .map(|n| n.parse::<u8>().ok().filter(|b| b.to_string() == n));
    let bytes = [numbers.next()??, numbers.next()??, numbers.next()??];
    numbers.next().is_none().then_some(bytes)
}

/// Whether a relay is one of n0's, which a joiner finds the Host on by key.
fn is_default_relay(relay: &str) -> bool {
    relay
        .parse::<iroh::RelayUrl>()
        .is_ok_and(|url| iroh::defaults::prod::default_relay_map().contains(&url))
}

const BASE64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Base64url without padding.
fn base64url(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(char::from(BASE64URL[(n >> (18 - 6 * i) & 63) as usize]));
        }
    }
    out
}

/// The bytes of unpadded base64url, if `text` is that.
fn unbase64url(text: &str) -> Option<Vec<u8>> {
    if text.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3 + 2);
    for chunk in text.as_bytes().chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            let value = BASE64URL.iter().position(|b| b == c)? as u32;
            n |= value << (18 - 6 * i);
        }
        let bytes = n.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len()]);
    }
    Some(out)
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
