//! An in-memory transport: the Host's lobby and joiners' lobbies in one
//! process, trading bytes the way the network would, for tests.

use std::collections::{BTreeMap, VecDeque};

use crate::invite::PeerKey;
use crate::lobby::{Admission, HostAction, HostLobby, JoinLobby};
use crate::wire::Refusal;

/// The Host and every joiner, connected through memory.
pub struct MemoryNet {
    pub host: HostLobby,
    joiners: BTreeMap<PeerKey, JoinLobby>,
    connected: BTreeMap<PeerKey, bool>,
    closed: BTreeMap<PeerKey, Refusal>,
    now_ms: i64,
    /// Setup and log messages the Host's app has to read.
    host_inbox: Vec<(PeerKey, crate::Message)>,
}

impl MemoryNet {
    pub fn new(host: HostLobby) -> Self {
        Self {
            host,
            joiners: BTreeMap::new(),
            connected: BTreeMap::new(),
            closed: BTreeMap::new(),
            now_ms: 0,
            host_inbox: Vec::new(),
        }
    }

    /// A joiner connects with its invite's secret; once admitted it says hello.
    pub fn connect(&mut self, peer: PeerKey, joiner: JoinLobby) -> Admission {
        let proof = joiner.proof();
        self.connect_with(peer, joiner, Some(proof))
    }

    /// A stranger connects with no secret at all.
    pub fn connect_without_invite(&mut self, peer: PeerKey, joiner: JoinLobby) -> Admission {
        self.connect_with(peer, joiner, None)
    }

    fn connect_with(&mut self, peer: PeerKey, joiner: JoinLobby, proof: Option<[u8; 16]>) -> Admission {
        let admission = self.host.admit(peer, proof.as_ref());
        let hello = joiner.hello();
        self.joiners.insert(peer, joiner);
        match &admission {
            Admission::Admit => {
                self.connected.insert(peer, true);
                self.closed.remove(&peer);
                self.deliver_from_joiner(peer, hello);
            }
            Admission::Refuse(refusal) => {
                self.closed.insert(peer, refusal.clone());
            }
        }
        admission
    }

    /// Raw bytes from a joiner, as a buggy or hostile app might send.
    pub fn send_raw(&mut self, peer: PeerKey, bytes: Vec<u8>) {
        self.deliver_from_joiner(peer, bytes);
    }

    pub fn disconnect(&mut self, peer: PeerKey) {
        self.connected.insert(peer, false);
        let actions = self.host.disconnected(peer);
        self.deliver(actions);
    }

    /// The Host's app sends `message` to one Player or all of them.
    pub fn host_sends(&mut self, to: Option<PeerKey>, message: &crate::Message) {
        let actions = self.host.send(to, message);
        self.deliver(actions);
    }

    /// The setup and log messages the Host's app hasn't read yet.
    pub fn host_inbox(&mut self) -> Vec<(PeerKey, crate::Message)> {
        std::mem::take(&mut self.host_inbox)
    }

    pub fn advance_ms(&mut self, ms: i64) {
        self.now_ms += ms;
    }

    pub fn joiner(&self, peer: PeerKey) -> &JoinLobby {
        &self.joiners[&peer]
    }

    pub fn joiner_mut(&mut self, peer: PeerKey) -> &mut JoinLobby {
        self.joiners.get_mut(&peer).expect("a joiner")
    }

    /// Why the Host closed this peer's connection, if it did.
    pub fn closed(&self, peer: PeerKey) -> Option<&Refusal> {
        self.closed.get(&peer)
    }

    fn is_connected(&self, peer: PeerKey) -> bool {
        self.connected.get(&peer).copied().unwrap_or(false)
    }

    fn deliver_from_joiner(&mut self, peer: PeerKey, bytes: Vec<u8>) {
        if !self.is_connected(peer) {
            return;
        }
        let actions = self.host.receive(peer, &bytes, self.now_ms);
        self.deliver(actions);
    }

    /// Carries the Host's actions out, and the joiners' replies back, until quiet.
    fn deliver(&mut self, actions: Vec<HostAction>) {
        let mut queue: VecDeque<HostAction> = actions.into();
        while let Some(action) = queue.pop_front() {
            let (to, bytes, close) = match action {
                HostAction::Deliver { from, message } => {
                    self.host_inbox.push((from, message));
                    continue;
                }
                HostAction::Send { to, bytes } => (to, bytes, None),
                HostAction::Close { peer, refusal, bytes } => (peer, bytes, Some(refusal)),
            };
            if !self.is_connected(to) {
                continue;
            }
            let replies = self.joiners.get_mut(&to).map(|j| j.receive(&bytes)).unwrap_or_default();
            if let Some(refusal) = close {
                self.closed.insert(to, refusal);
                self.connected.insert(to, false);
                continue;
            }
            for reply in replies {
                queue.extend(self.host.receive(to, &reply, self.now_ms));
            }
        }
    }
}
