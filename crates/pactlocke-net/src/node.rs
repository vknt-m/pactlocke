//! The lobby over iroh. Each app's endpoint is identified by its key; the
//! Host's lobby and a joiner's run on a background thread with its own
//! runtime, and the UI reads a [`LobbyView`] from them.
//!
//! On a connection the joiner first sends its invite's 16-byte Run secret,
//! the one thing the Host reads before deciding: a key outside the roster
//! without an unused secret is closed then. After that each message is a
//! two-byte length and the bytes, never longer than [`MAX_MESSAGE`].

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use iroh::endpoint::{Connection, SendStream, presets};
use iroh::{Endpoint, EndpointAddr, PublicKey, RelayMode, RelayUrl, SecretKey};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::{Semaphore, mpsc};

use crate::invite::{Invite, PeerKey};
use crate::lobby::{Admission, HostAction, HostLobby, JoinLobby, JoinState};
use crate::wire::{LobbyPlayer, MAX_MESSAGE, Message, RunAdvert, encode};

/// The protocol Pactlocke's apps speak to each other on.
const ALPN: &[u8] = b"pactlocke/1";
/// How long a joiner has to open its stream and send its Run secret.
const SECRET_WITHIN: Duration = Duration::from_secs(5);
/// Connections the Host is deciding on at once; more are refused unread.
const HANDSHAKES_AT_ONCE: usize = 8;
/// Messages waiting to go to one Player; a Player this far behind is closed.
const OUTBOX: usize = 512;
/// How long the Host waits for a relay before giving out an invite without one.
const ONLINE_WITHIN: Duration = Duration::from_secs(15);
/// How long a joiner tries to reach the Host.
const CONNECT_WITHIN: Duration = Duration::from_secs(30);

/// Which relays an endpoint uses to be reached and to punch through to a
/// direct connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Relays {
    /// n0's encrypted public relays: the default.
    N0,
    /// Relays the Player listed, such as a self-hosted `iroh-relay`.
    Custom(Vec<String>),
    /// None: direct connections on this machine or network only (tests).
    Off,
}

impl Relays {
    /// Relay URLs as a Player types them, separated by spaces, commas or
    /// lines; none means n0's.
    pub fn parse(text: &str) -> Self {
        let urls: Vec<String> = text
            .split([',', ' ', '\n'])
            .map(str::trim)
            .filter(|u| !u.is_empty())
            .map(str::to_owned)
            .collect();
        if urls.is_empty() {
            Relays::N0
        } else {
            Relays::Custom(urls)
        }
    }
}

/// The lobby as the UI shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LobbyView {
    /// The Host's current invite link, once the endpoint is up.
    pub invite_link: Option<String>,
    /// What the Run is, once known.
    pub run: Option<RunAdvert>,
    /// Every Player in the lobby.
    pub players: Vec<LobbyPlayer>,
    /// The Host's: every Player who has said hello, by key and name.
    pub roster: Vec<(PeerKey, String)>,
    /// A joiner's own ROM against the Run's game, once both are known.
    pub rom_matches: Option<bool>,
    /// Something went wrong: what to tell the Player.
    pub error: Option<String>,
}

/// How a connection to another Player's app goes now: straight to them, or
/// through a relay, and how long a message takes there and back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerLink {
    /// Whether messages go straight to the other Player, not through a relay.
    pub direct: bool,
    /// How long a message takes there and back, in milliseconds.
    pub ping_ms: u32,
}

/// The connection's selected path, if it has one yet.
fn link_of(connection: &Connection) -> Option<PeerLink> {
    let paths = connection.paths();
    let path = paths.into_iter().find(|p| p.is_selected())?;
    Some(PeerLink {
        direct: path.is_ip(),
        ping_ms: u32::try_from(path.rtt().as_millis()).unwrap_or(u32::MAX),
    })
}

/// A fresh endpoint key (a throwaway one, as in tests).
pub fn new_key() -> [u8; 32] {
    SecretKey::generate().to_bytes()
}

/// This install's key, kept in `path`: its identity on the network. It is
/// made on first use, written whole or not at all. A file that is there but
/// can't be read is an error, never replaced: a new key would be a new
/// identity the Run's roster doesn't know.
pub fn load_or_create_key(path: &Path) -> std::io::Result<[u8; 32]> {
    match std::fs::read(path) {
        Ok(bytes) => <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{} isn't a key: it has {} bytes, not 32", path.display(), bytes.len()),
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let key = new_key();
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let part = path.with_extension("key.part");
            std::fs::write(&part, key)?;
            std::fs::rename(&part, path)?;
            Ok(key)
        }
        Err(e) => Err(e),
    }
}

/// A lock that carries on after another thread panicked holding it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

async fn bind(key: [u8; 32], relays: &Relays) -> Result<Endpoint, String> {
    let mode = match relays {
        Relays::N0 => RelayMode::Default,
        Relays::Off => RelayMode::Disabled,
        Relays::Custom(urls) => {
            let urls: Vec<RelayUrl> = urls
                .iter()
                .map(|u| u.parse().map_err(|_| format!("{u:?} isn't a relay URL")))
                .collect::<Result<_, _>>()?;
            RelayMode::custom(urls)
        }
    };
    Endpoint::builder(presets::N0)
        .secret_key(SecretKey::from_bytes(&key))
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(mode)
        .bind()
        .await
        .map_err(|e| format!("the network couldn't be started: {e}"))
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a runtime for the network")
}

/// One length-prefixed message.
async fn write_frame(send: &mut SendStream, bytes: &[u8]) -> Result<(), String> {
    let len = u16::try_from(bytes.len()).map_err(|_| "a message too long to send".to_owned())?;
    send.write_all(&len.to_le_bytes()).await.map_err(|e| e.to_string())?;
    send.write_all(bytes).await.map_err(|e| e.to_string())
}

/// Reads messages from `recv` on their own task, onto the channel returned.
/// A read is never cut short: one cut short inside a `select!` loses the
/// bytes it read, and every length after them is read from inside a message.
fn frames_of<R>(mut recv: R) -> (mpsc::Receiver<Vec<u8>>, tokio::task::JoinHandle<()>)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let (frames_in, frames) = mpsc::channel(OUTBOX);
    let reader = tokio::spawn(async move {
        while let Some(bytes) = read_frame(&mut recv).await {
            if frames_in.send(bytes).await.is_err() {
                break;
            }
        }
    });
    (frames, reader)
}

/// The next message, or `None` when the stream ends or breaks its bounds.
async fn read_frame<R: AsyncRead + Unpin>(recv: &mut R) -> Option<Vec<u8>> {
    let mut len = [0u8; 2];
    recv.read_exact(&mut len).await.ok()?;
    let len = usize::from(u16::from_le_bytes(len));
    if len > MAX_MESSAGE {
        log::warn!("A message of {len} bytes arrived, over the {MAX_MESSAGE}-byte limit");
        return None;
    }
    let mut bytes = vec![0u8; len];
    recv.read_exact(&mut bytes).await.ok()?;
    Some(bytes)
}

fn publish(view: &Mutex<LobbyView>, change: impl FnOnce(&mut LobbyView)) {
    change(&mut lock(view));
}

/// A network thread and how to stop it; stopping waits for the thread.
struct Worker<C> {
    view: Arc<Mutex<LobbyView>>,
    commands: mpsc::UnboundedSender<C>,
    stop: fn() -> C,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl<C> Worker<C> {
    fn view(&self) -> LobbyView {
        lock(&self.view).clone()
    }

    fn send(&self, command: C) {
        let _ = self.commands.send(command);
    }
}

impl<C> Drop for Worker<C> {
    fn drop(&mut self) {
        let _ = self.commands.send((self.stop)());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// This app's nodes that a new one on the same key replaces, and any of
/// its nodes still shutting down. The new node's thread waits for them all
/// to close before its own network starts: a relay that sees two
/// connections on one key drops one of them. The caller never waits.
#[derive(Default)]
pub struct Replaced {
    host: Option<HostNode>,
    join: Option<JoinNode>,
    /// Earlier nodes shutting down on threads of their own.
    shutting_down: Vec<std::thread::JoinHandle<()>>,
}

impl Replaced {
    /// Nothing to shut down first.
    pub fn none() -> Self {
        Self::default()
    }

    /// These nodes and `other`'s, which start shutting down now.
    pub fn and(mut self, other: Replaced) -> Self {
        self.shutting_down.extend(other.in_background().shutting_down);
        self
    }

    /// Starts shutting the nodes down on a thread of their own, as when the
    /// Player leaves: what is returned stands for that shutdown, for a node
    /// started later on the same key to wait for.
    pub fn in_background(self) -> Self {
        let thread = std::thread::spawn(move || self.shut_down());
        Self {
            shutting_down: vec![thread],
            ..Self::default()
        }
    }

    /// Shuts the nodes down, waiting for their networks to close.
    fn shut_down(self) {
        let Self {
            host,
            join,
            shutting_down,
        } = self;
        drop(host);
        drop(join);
        for thread in shutting_down {
            let _ = thread.join();
        }
    }
}

impl From<HostNode> for Replaced {
    fn from(host: HostNode) -> Self {
        Self {
            host: Some(host),
            ..Self::default()
        }
    }
}

impl From<JoinNode> for Replaced {
    fn from(join: JoinNode) -> Self {
        Self {
            join: Some(join),
            ..Self::default()
        }
    }
}

impl From<(Option<HostNode>, Option<JoinNode>)> for Replaced {
    fn from((host, join): (Option<HostNode>, Option<JoinNode>)) -> Self {
        Self {
            host,
            join,
            ..Self::default()
        }
    }
}

enum HostCommand {
    NewInvite,
    Send(Option<PeerKey>, Message),
    Lock,
    Stop,
}

/// The Host's lobby, running on its own thread. Dropping it closes the
/// lobby and waits for the network to shut down.
pub struct HostNode {
    worker: Worker<HostCommand>,
    addrs: Arc<Mutex<Vec<SocketAddr>>>,
    inbox: Arc<Mutex<Vec<(PeerKey, Message)>>>,
    peers: Arc<Mutex<BTreeMap<PeerKey, Peer>>>,
}

/// What the Host's connections share.
#[derive(Clone)]
struct Shared {
    lobby: Arc<Mutex<HostLobby>>,
    view: Arc<Mutex<LobbyView>>,
    peers: Arc<Mutex<BTreeMap<PeerKey, Peer>>>,
    /// Setup and log messages from Players, for the Host's app to read.
    inbox: Arc<Mutex<Vec<(PeerKey, Message)>>>,
    started: Instant,
}

/// A Player's live connection: which one it is (a key may reconnect while
/// its old connection lingers), its queue of messages and the connection.
struct Peer {
    connection_id: u64,
    outbox: mpsc::Sender<Outgoing>,
    connection: Connection,
}

enum Outgoing {
    Send(Vec<u8>),
    /// Send this, then end the stream.
    SendAndFinish(Vec<u8>),
}

impl HostNode {
    /// Starts the Host's endpoint with `key` and makes the first invite.
    pub fn start(key: [u8; 32], lobby: HostLobby, relays: Relays) -> Self {
        Self::start_replacing(key, lobby, relays, Replaced::none())
    }

    /// The same, once `replaced`, this app's nodes on `key` until now, have
    /// shut down; this returns at once.
    pub fn start_replacing(key: [u8; 32], lobby: HostLobby, relays: Relays, replaced: Replaced) -> Self {
        let view = Arc::new(Mutex::new(LobbyView {
            run: Some(lobby.run().clone()),
            players: lobby.players(),
            roster: lobby.roster(),
            ..LobbyView::default()
        }));
        let addrs = Arc::new(Mutex::new(Vec::new()));
        let (commands, receiver) = mpsc::unbounded_channel();
        let inbox = Arc::new(Mutex::new(Vec::new()));
        let peers: Arc<Mutex<BTreeMap<PeerKey, Peer>>> = Arc::default();
        let shared = Shared {
            lobby: Arc::new(Mutex::new(lobby)),
            view: view.clone(),
            peers: peers.clone(),
            inbox: inbox.clone(),
            started: Instant::now(),
        };
        let known = addrs.clone();
        let thread = std::thread::Builder::new()
            .name("lobby host".into())
            .spawn(move || {
                replaced.shut_down();
                runtime().block_on(host(key, relays, shared, known, receiver));
            })
            .expect("a thread for the lobby");
        Self {
            worker: Worker {
                view,
                commands,
                stop: || HostCommand::Stop,
                thread: Some(thread),
            },
            addrs,
            inbox,
            peers,
        }
    }

    /// How each connected Player's connection goes now.
    pub fn links(&self) -> Vec<(PeerKey, PeerLink)> {
        lock(&self.peers)
            .iter()
            .filter_map(|(key, peer)| Some((*key, link_of(&peer.connection)?)))
            .collect()
    }

    /// Setup and log messages from Players since the last call, oldest first.
    pub fn take_inbox(&self) -> Vec<(PeerKey, Message)> {
        std::mem::take(&mut lock(&self.inbox))
    }

    /// A setup or log message to one Player, or to everyone in the lobby.
    pub fn send(&self, to: Option<PeerKey>, message: Message) {
        self.worker.send(HostCommand::Send(to, message));
    }

    /// The Ruleset and roster locked: from now on only the roster connects.
    pub fn lock_roster(&self) {
        self.worker.send(HostCommand::Lock);
    }

    /// The lobby as the UI shows it now.
    pub fn view(&self) -> LobbyView {
        self.worker.view()
    }

    /// A fresh invite link for another friend; each one admits one Player.
    pub fn new_invite(&self) {
        self.worker.send(HostCommand::NewInvite);
    }

    /// Where the Host can be reached directly, for joiners on the same machine (tests).
    pub fn direct_addrs(&self) -> Vec<SocketAddr> {
        lock(&self.addrs).clone()
    }
}

async fn host(
    key: [u8; 32],
    relays: Relays,
    shared: Shared,
    addrs: Arc<Mutex<Vec<SocketAddr>>>,
    mut commands: mpsc::UnboundedReceiver<HostCommand>,
) {
    let endpoint = match bind(key, &relays).await {
        Ok(endpoint) => endpoint,
        Err(e) => return publish(&shared.view, |v| v.error = Some(e)),
    };
    if relays != Relays::Off {
        let _ = tokio::time::timeout(ONLINE_WITHIN, endpoint.online()).await;
    }
    let address = endpoint.addr();
    *lock(&addrs) = address.ip_addrs().copied().collect();
    let relay = address.relay_urls().next().map(|url| url.to_string());
    let me = PeerKey(*endpoint.id().as_bytes());
    // A lobby taken over after the lock has no invite: the roster dials by key.
    let invite = |shared: &Shared| {
        let mut lobby = lock(&shared.lobby);
        (!lobby.is_locked()).then(|| lobby.invite(me, relay.clone()).link())
    };
    let link = invite(&shared);
    publish(&shared.view, |v| v.invite_link = link);
    let handshakes = Arc::new(Semaphore::new(HANDSHAKES_AT_ONCE));
    let mut next_connection = 0u64;

    loop {
        tokio::select! {
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break };
                // Too many undecided connections at once: refuse this one unread.
                let Ok(permit) = handshakes.clone().try_acquire_owned() else {
                    incoming.refuse();
                    continue;
                };
                next_connection += 1;
                let (shared, connection_id) = (shared.clone(), next_connection);
                tokio::spawn(async move {
                    let Ok(accepting) = incoming.accept() else { return };
                    let Ok(connection) = accepting.await else { return };
                    serve_player(connection, connection_id, shared, permit).await;
                });
            }
            command = commands.recv() => match command {
                Some(HostCommand::NewInvite) => {
                    let link = invite(&shared);
                    publish(&shared.view, |v| v.invite_link = link);
                }
                Some(HostCommand::Send(to, message)) => {
                    let actions = lock(&shared.lobby).send(to, &message);
                    carry_out(actions, &shared);
                }
                Some(HostCommand::Lock) => {
                    lock(&shared.lobby).lock();
                    publish(&shared.view, |v| v.invite_link = None);
                }
                Some(HostCommand::Stop) | None => break,
            },
        }
    }
    endpoint.close().await;
}

/// Carries out the lobby's actions and shows the lobby as it now is. A
/// Player whose queue is full is too far behind and is closed.
fn carry_out(actions: Vec<HostAction>, shared: &Shared) {
    {
        let peers = lock(&shared.peers);
        for action in actions {
            let (to, outgoing) = match action {
                HostAction::Deliver { from, message } => {
                    lock(&shared.inbox).push((from, message));
                    continue;
                }
                HostAction::Send { to, bytes } => (to, Outgoing::Send(bytes)),
                HostAction::Close { peer, bytes, refusal } => {
                    log::warn!("Host: closing {}: {refusal}", short(&peer));
                    (peer, Outgoing::SendAndFinish(bytes))
                }
            };
            let Some(peer) = peers.get(&to) else {
                continue;
            };
            let input = matches!(&outgoing, Outgoing::Send(bytes) if crate::wire::is_input(bytes));
            if peer.outbox.try_send(outgoing).is_err() {
                // A stream chunk can be asked for again: drop it, not the Player.
                if input {
                    log::debug!(
                        "Host: {} is behind; an input stream message waits for a resend",
                        short(&to)
                    );
                } else {
                    log::warn!(
                        "Host: {} is too far behind for setup and log messages; closing",
                        short(&to)
                    );
                    peer.connection.close(3u8.into(), b"too far behind");
                }
            }
        }
    }
    let (players, roster) = {
        let lobby = lock(&shared.lobby);
        (lobby.players(), lobby.roster())
    };
    publish(&shared.view, |v| {
        v.players = players;
        v.roster = roster;
    });
}

async fn serve_player(
    connection: Connection,
    connection_id: u64,
    shared: Shared,
    handshake: tokio::sync::OwnedSemaphorePermit,
) {
    let peer = PeerKey(*connection.remote_id().as_bytes());
    let Ok(Ok((mut send, mut recv))) = tokio::time::timeout(SECRET_WITHIN, connection.accept_bi()).await else {
        connection.close(1u8.into(), b"no Run secret");
        return;
    };
    // The Run secret, and nothing else, before the Host decides.
    let mut secret = [0u8; 16];
    let proof = match tokio::time::timeout(SECRET_WITHIN, recv.read_exact(&mut secret)).await {
        Ok(Ok(())) => Some(secret),
        _ => None,
    };
    let admission = lock(&shared.lobby).admit(peer, proof.as_ref());
    if let Admission::Refuse(refusal) = &admission {
        log::info!("Host: refused {}: {refusal}", short(&peer));
    } else {
        log::info!("Host: {} connected", short(&peer));
    }
    if let Admission::Refuse(refusal) = admission {
        let _ = write_frame(&mut send, &encode(&Message::Refused(refusal))).await;
        let _ = send.finish();
        drop(handshake);
        tokio::time::sleep(Duration::from_millis(200)).await;
        connection.close(2u8.into(), b"refused");
        return;
    }
    drop(handshake);

    let (outbox, mut queued) = mpsc::channel(OUTBOX);
    let replaced = lock(&shared.peers).insert(
        peer,
        Peer {
            connection_id,
            outbox,
            connection: connection.clone(),
        },
    );
    // The same key on a new connection: the old one is done.
    if let Some(old) = replaced {
        old.connection.close(4u8.into(), b"reconnected");
    }
    let writer = tokio::spawn(async move {
        while let Some(outgoing) = queued.recv().await {
            match outgoing {
                Outgoing::Send(bytes) => {
                    if let Err(e) = write_frame(&mut send, &bytes).await {
                        log::warn!("Host: writing to {} failed: {e}", short(&peer));
                        break;
                    }
                }
                Outgoing::SendAndFinish(bytes) => {
                    let _ = write_frame(&mut send, &bytes).await;
                    let _ = send.finish();
                    break;
                }
            }
        }
    });
    while let Some(bytes) = read_frame(&mut recv).await {
        let now_ms = shared.started.elapsed().as_millis() as i64;
        let actions = lock(&shared.lobby).receive(peer, &bytes, now_ms);
        let closing = actions
            .iter()
            .any(|a| matches!(a, HostAction::Close { peer: p, .. } if *p == peer));
        carry_out(actions, &shared);
        if closing {
            break;
        }
    }
    // Only the Player's current connection ending takes them out of the lobby.
    let current = {
        let mut peers = lock(&shared.peers);
        let current = peers.get(&peer).is_some_and(|p| p.connection_id == connection_id);
        if current {
            peers.remove(&peer);
        }
        current
    };
    if current {
        log::info!("Host: {} disconnected", short(&peer));
        let actions = lock(&shared.lobby).disconnected(peer);
        carry_out(actions, &shared);
    }
    let _ = tokio::time::timeout(Duration::from_secs(1), writer).await;
    connection.close(0u8.into(), b"bye");
}

enum JoinCommand {
    RomPicked(String),
    Send(Message),
    Stop,
}

/// A joiner's lobby, running on its own thread. Dropping it leaves the
/// lobby and waits for the network to shut down.
pub struct JoinNode {
    worker: Worker<JoinCommand>,
    inbox: Arc<Mutex<Vec<Message>>>,
    /// The connection to the Host, while there is one.
    connection: Arc<Mutex<Option<Connection>>>,
}

impl JoinNode {
    /// Joins the Run `lobby`'s invite is for, reaching the Host through the
    /// invite's relay.
    pub fn start(key: [u8; 32], lobby: JoinLobby, relays: Relays) -> Self {
        Self::start_with(key, lobby, relays, Vec::new())
    }

    /// The same, also trying these direct addresses (a Host on this machine, in tests).
    pub fn start_with(key: [u8; 32], lobby: JoinLobby, relays: Relays, direct: Vec<SocketAddr>) -> Self {
        Self::start_replacing(key, lobby, relays, direct, Replaced::none())
    }

    /// The same, once `replaced`, this app's nodes on `key` until now, have
    /// shut down; this returns at once.
    pub fn start_replacing(
        key: [u8; 32],
        lobby: JoinLobby,
        relays: Relays,
        direct: Vec<SocketAddr>,
        replaced: Replaced,
    ) -> Self {
        let view = Arc::new(Mutex::new(LobbyView::default()));
        let inbox = Arc::new(Mutex::new(Vec::new()));
        let (commands, receiver) = mpsc::unbounded_channel();
        let (shown, received) = (view.clone(), inbox.clone());
        let connection: Arc<Mutex<Option<Connection>>> = Arc::default();
        let held = connection.clone();
        let thread = std::thread::Builder::new()
            .name("lobby join".into())
            .spawn(move || {
                replaced.shut_down();
                runtime().block_on(join(key, lobby, relays, direct, shown, received, receiver, held));
            })
            .expect("a thread for the lobby");
        Self {
            worker: Worker {
                view,
                commands,
                stop: || JoinCommand::Stop,
                thread: Some(thread),
            },
            inbox,
            connection,
        }
    }

    /// How the connection to the Host goes now, while there is one.
    pub fn link(&self) -> Option<PeerLink> {
        lock(&self.connection).as_ref().and_then(link_of)
    }

    /// The Host's setup and log messages since the last call, oldest first.
    pub fn take_inbox(&self) -> Vec<Message> {
        std::mem::take(&mut lock(&self.inbox))
    }

    /// A message to the Host. Input sent while there is no connection is
    /// dropped: the Host asks again for what it misses, and a game's stream
    /// queued while offline would arrive all at once, a flood.
    pub fn send(&self, message: Message) {
        if matches!(message, Message::Input(_)) && lock(&self.connection).is_none() {
            return;
        }
        self.worker.send(JoinCommand::Send(message));
    }

    /// The lobby as the UI shows it now.
    pub fn view(&self) -> LobbyView {
        self.worker.view()
    }

    /// The Player picked their ROM; the Host is told its SHA-1.
    pub fn rom_picked(&self, sha1: &str) {
        self.worker.send(JoinCommand::RomPicked(sha1.to_owned()));
    }
}

/// Shows a joiner's lobby as it now is.
fn publish_join(view: &Mutex<LobbyView>, lobby: &JoinLobby) {
    publish(view, |v| match lobby.state() {
        JoinState::Joining => {}
        JoinState::InLobby { run, players } => {
            v.run = Some(run.clone());
            v.players = players.clone();
            v.rom_matches = lobby.rom_matches();
        }
        JoinState::Refused(why) => v.error = Some(why.clone()),
    });
}

#[allow(clippy::too_many_arguments)]
async fn join(
    key: [u8; 32],
    lobby: JoinLobby,
    relays: Relays,
    direct: Vec<SocketAddr>,
    view: Arc<Mutex<LobbyView>>,
    inbox: Arc<Mutex<Vec<Message>>>,
    commands: mpsc::UnboundedReceiver<JoinCommand>,
    held: Arc<Mutex<Option<Connection>>>,
) {
    let fail = |why: String| {
        log::warn!("Joiner: {why}");
        publish(&view, |v| v.error = Some(why));
    };
    let endpoint = match bind(key, &relays).await {
        Ok(endpoint) => endpoint,
        Err(e) => return fail(e),
    };
    if let Err(why) = join_on(&endpoint, lobby, direct, &view, inbox, commands, held).await {
        fail(why);
    }
    // Closed, never just dropped, on every way out: the relay is told this
    // key's connection is gone before a node replacing this one connects.
    endpoint.close().await;
}

/// The joiner's connection to the Host on `endpoint`, until it ends or the
/// node is stopped. A stop cuts reaching the Host short too.
async fn join_on(
    endpoint: &Endpoint,
    mut lobby: JoinLobby,
    direct: Vec<SocketAddr>,
    view: &Mutex<LobbyView>,
    inbox: Arc<Mutex<Vec<Message>>>,
    mut commands: mpsc::UnboundedReceiver<JoinCommand>,
    held: Arc<Mutex<Option<Connection>>>,
) -> Result<(), String> {
    let invite: &Invite = lobby.invite();
    let Ok(host) = PublicKey::from_bytes(&invite.host.0) else {
        return Err("The invite's Host key is damaged.".into());
    };
    let mut address = EndpointAddr::new(host);
    if let Some(relay) = invite.relay.as_ref().and_then(|r| r.parse::<RelayUrl>().ok()) {
        address = address.with_relay_url(relay);
    }
    for addr in direct {
        address = address.with_ip_addr(addr);
    }
    let reaching = tokio::time::timeout(CONNECT_WITHIN, endpoint.connect(address, ALPN));
    tokio::pin!(reaching);
    // What the Player does meanwhile is carried out once connected.
    let mut early = Vec::new();
    let connection = loop {
        tokio::select! {
            reached = &mut reaching => match reached {
                Ok(Ok(connection)) => break connection,
                Ok(Err(e)) => return Err(format!("The Host couldn't be reached: {e}.")),
                Err(_) => {
                    return Err(format!(
                        "The Host couldn't be reached: no answer in {} seconds.",
                        CONNECT_WITHIN.as_secs()
                    ));
                }
            },
            command = commands.recv() => match command {
                Some(JoinCommand::Stop) | None => return Ok(()),
                Some(command) => early.push(command),
            },
        }
    };
    let Ok((mut send, recv)) = connection.open_bi().await else {
        return Err("The connection to the Host closed at once.".into());
    };
    if send.write_all(&lobby.proof()).await.is_err() || write_frame(&mut send, &lobby.hello()).await.is_err() {
        return Err("The connection to the Host closed at once.".into());
    }
    *lock(&held) = Some(connection.clone());
    // Frames are read on their own task, so the select below can't cut one short.
    let (mut frames, reader) = frames_of(recv);
    for command in early {
        on_command(Some(command), &mut lobby, &mut send, view).await;
    }
    loop {
        tokio::select! {
            frame = frames.recv() => {
                let Some(bytes) = frame else {
                    log::warn!("Joiner: the Host's stream ended ({})", connection.close_reason().map_or_else(|| "no reason given".to_owned(), |r| r.to_string()));
                    lobby.lost("it closed");
                    publish_join(view, &lobby);
                    break;
                };
                for reply in lobby.receive(&bytes) {
                    let _ = write_frame(&mut send, &reply).await;
                }
                lock(&inbox).extend(lobby.take_inbox());
                publish_join(view, &lobby);
            }
            command = commands.recv() => {
                if !on_command(command, &mut lobby, &mut send, view).await {
                    break;
                }
            }
        }
    }
    reader.abort();
    *lock(&held) = None;
    connection.close(0u8.into(), b"bye");
    Ok(())
}

/// Carries out a Player's command on the connection to the Host; `false`
/// when it is to stop.
async fn on_command(
    command: Option<JoinCommand>,
    lobby: &mut JoinLobby,
    send: &mut SendStream,
    view: &Mutex<LobbyView>,
) -> bool {
    match command {
        Some(JoinCommand::RomPicked(sha1)) => {
            if let Some(message) = lobby.rom_picked(&sha1) {
                let _ = write_frame(send, &message).await;
            }
            publish_join(view, lobby);
        }
        Some(JoinCommand::Send(message)) => {
            let _ = write_frame(send, &lobby.send(&message)).await;
        }
        Some(JoinCommand::Stop) | None => return false,
    }
    true
}

/// A Player's key, shortened for the log.
fn short(peer: &PeerKey) -> String {
    peer.to_string().chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::AsyncWriteExt;

    /// The joiner's loop selects between the Host's messages and its own
    /// sends: a message arriving slowly, in pieces, must still come through
    /// whole however often the other side of the select wins.
    #[test]
    fn a_message_arriving_in_pieces_comes_through_whole_while_the_caller_selects() {
        super::runtime().block_on(async {
            let (mut write, read) = tokio::io::duplex(256);
            let (mut frames, _reader) = super::frames_of(read);
            let message: Vec<u8> = (0..5_000u32).map(|i| i as u8).collect();
            let sent = message.clone();
            tokio::spawn(async move {
                for _ in 0..3 {
                    write.write_all(&(sent.len() as u16).to_le_bytes()).await.unwrap();
                    for piece in sent.chunks(700) {
                        write.write_all(piece).await.unwrap();
                        tokio::time::sleep(Duration::from_millis(2)).await;
                    }
                }
            });
            let mut got = Vec::new();
            while got.len() < 3 {
                tokio::select! {
                    frame = frames.recv() => got.push(frame.expect("the stream goes on")),
                    // As the Player's app sending something meanwhile.
                    () = tokio::time::sleep(Duration::from_micros(300)) => {}
                }
            }
            assert!(got.iter().all(|frame| *frame == message));
        });
    }
}
