//! Links: each Player's Encounter from the same source (an Area, a Gift or
//! a Static) is bound into one Link. If any member dies, every member dies;
//! if any Player misses, skips or loses their Encounter from a source, its
//! Link breaks and the source closes for everyone. Worked out from the
//! shared Run log, with every other Player's catches and Deaths judged from
//! their Shadow Game, not their app's word.

use std::collections::{BTreeMap, BTreeSet};

use crate::shared_log::RunLog;
use crate::stream::PlayerKey;
use crate::{RunCommand, RunEvent, SourceRef};

/// One Player's part of a Link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Member {
    /// Hasn't had this Encounter yet.
    Pending,
    /// Caught (or given) this Pokémon from it.
    Caught {
        pid: u32,
        species: u16,
        species_name: String,
        /// Their Shadow Game here has shown it (always so for this Player's own).
        judged: bool,
        /// Dead, and (for another Player's) their Shadow Game has shown it fainted.
        dead: bool,
    },
    /// Missed or lost it (`skipped`: chose to skip it): the Link is broken.
    Missed { skipped: bool },
}

/// The Link of one source: every Player's part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub source: SourceRef,
    /// A Gift (rather than an Area or a Static): one a Player may choose to skip.
    pub gift: bool,
    /// Every Player in the roster, in join order, with their part.
    pub members: Vec<(PlayerKey, Member)>,
    /// Why it is broken, if it is: "Blue's PIDGEY died", "Blue missed it".
    pub broken: Option<String>,
}

impl Link {
    /// Whether some Player hasn't had their Encounter yet (or their Shadow
    /// Game here hasn't shown their catch): a catch shows a pending outline.
    pub fn pending(&self) -> bool {
        self.broken.is_none()
            && self
                .members
                .iter()
                .any(|(_, m)| matches!(m, Member::Pending | Member::Caught { judged: false, .. }))
    }

    /// This Player's part.
    pub fn member(&self, player: &PlayerKey) -> Option<&Member> {
        self.members.iter().find(|(p, _)| p == player).map(|(_, m)| m)
    }
}

/// One of this Player's Pokémon whose Link waits on another Player's catch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitingOnLink {
    pub pid: u32,
    /// What it waits for, to finish a sentence: "until Blue catches on Route 1".
    pub until: String,
}

/// Where this Player's Pokémon stand in their Links, as the Run log has them now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkStanding {
    /// Every one of this Player's Pokémon in a Link, whole, waiting or broken.
    pub linked: Vec<u32>,
    /// Those whose Link waits on another Player's catch.
    pub waiting: Vec<WaitingOnLink>,
}

/// What another Player's Shadow Game here has shown, for judging their claims.
pub trait Judge {
    /// Whether `player`'s game has shown this Pokémon in their party or PC.
    fn has(&self, player: &PlayerKey, pid: u32) -> bool;
    /// Whether `player`'s game has shown this Pokémon fainted or in their Dead box.
    fn fainted(&self, player: &PlayerKey, pid: u32) -> bool;
}

/// Every Link in the Run, in the order their sources were first met.
pub fn links(log: &RunLog, me: &PlayerKey, judge: &dyn Judge) -> Vec<Link> {
    let Some(locked) = log.locked() else {
        return Vec::new();
    };
    // A Player dropped from the Run is out of every Link.
    let roster: Vec<PlayerKey> = log.active();
    let names: BTreeMap<PlayerKey, String> = locked.roster.iter().map(|p| (p.key, p.name.clone())).collect();
    let mut order: Vec<SourceRef> = Vec::new();
    let mut parts: BTreeMap<String, BTreeMap<PlayerKey, Member>> = BTreeMap::new();
    let mut deaths: BTreeMap<PlayerKey, BTreeSet<u32>> = BTreeMap::new();
    let mut gifts: BTreeSet<String> = BTreeSet::new();
    // Each Player's catches by PID: species and its name, for an Encounter's part.
    let mut caught: BTreeMap<(PlayerKey, u32), (u16, String)> = BTreeMap::new();
    for (author, event) in log.standing_events() {
        let trusted = author == *me;
        match event {
            RunEvent::Gift {
                source: Some(source),
                pid,
                species,
                species_name,
                ..
            } => {
                gifts.insert(source.key.clone());
                caught.insert((author, pid), (species, species_name.clone()));
                let judged = trusted || judge.has(&author, pid);
                set(
                    &source,
                    author,
                    Member::Caught {
                        pid,
                        species,
                        species_name,
                        judged,
                        dead: false,
                    },
                    &mut order,
                    &mut parts,
                );
            }
            RunEvent::Caught {
                pid,
                species,
                species_name,
                ..
            } => {
                caught.insert((author, pid), (species, species_name));
            }
            RunEvent::EncounterOver { source, pid } => {
                let member = match pid {
                    Some(pid) => {
                        let (species, species_name) = caught.get(&(author, pid)).cloned().unwrap_or_default();
                        Member::Caught {
                            pid,
                            species,
                            species_name,
                            judged: trusted || judge.has(&author, pid),
                            dead: false,
                        }
                    }
                    None => Member::Missed { skipped: false },
                };
                set(&source, author, member, &mut order, &mut parts);
            }
            RunEvent::Skipped { source } => set(
                &source,
                author,
                Member::Missed { skipped: true },
                &mut order,
                &mut parts,
            ),
            RunEvent::Released { pid, .. } => {
                // A released Pokémon (a declined Gift among them) is that Player's part lost.
                for members in parts.values_mut() {
                    if let Some(member) = members.get_mut(&author)
                        && matches!(member, Member::Caught { pid: p, .. } if *p == pid)
                    {
                        *member = Member::Missed { skipped: false };
                    }
                }
            }
            RunEvent::Died { pid, .. } if trusted || judge.fainted(&author, pid) => {
                deaths.entry(author).or_default().insert(pid);
            }
            _ => {}
        }
    }
    order
        .into_iter()
        .map(|source| {
            let mut found = parts.remove(&source.key).unwrap_or_default();
            let mut broken = None;
            let members: Vec<(PlayerKey, Member)> = roster
                .iter()
                .map(|player| {
                    let mut member = found.remove(player).unwrap_or(Member::Pending);
                    let name = names.get(player).cloned().unwrap_or_default();
                    match &mut member {
                        Member::Caught {
                            pid,
                            species_name,
                            judged: true,
                            dead,
                            ..
                        } if deaths.get(player).is_some_and(|pids| pids.contains(pid)) => {
                            *dead = true;
                            broken.get_or_insert(format!("{name}'s {species_name} died"));
                        }
                        Member::Missed { skipped: false } => {
                            broken.get_or_insert(format!("{name} didn't catch it"));
                        }
                        Member::Missed { skipped: true } => {
                            broken.get_or_insert(format!("{name} skipped it"));
                        }
                        _ => {}
                    }
                    (*player, member)
                })
                .collect();
            Link {
                gift: gifts.contains(&source.key),
                source,
                members,
                broken,
            }
        })
        .collect()
}

/// A Player's part of a source, unless they have one already.
fn set(
    source: &SourceRef,
    player: PlayerKey,
    member: Member,
    order: &mut Vec<SourceRef>,
    parts: &mut BTreeMap<String, BTreeMap<PlayerKey, Member>>,
) {
    if !order.iter().any(|s| s.key == source.key) {
        order.push(source.clone());
    }
    parts
        .entry(source.key.clone())
        .or_default()
        .entry(player)
        .or_insert(member);
}

/// What one Player's Run does about the Links: their own member of a broken
/// Link dies, and a broken Link's source they haven't had yet closes for
/// them. It is asked again whenever the Links change; the Run engine does
/// each only once, so a catch made after a source closed, a Retry and a
/// resume all come out right.
#[derive(Debug, Clone)]
pub struct LinkKeeper {
    me: PlayerKey,
}

impl LinkKeeper {
    pub fn new(me: PlayerKey) -> Self {
        Self { me }
    }

    /// The commands for this Player's Run from the Links as they are now.
    /// Nothing without the Links rule.
    pub fn keep(&self, log: &RunLog, links: &[Link]) -> Vec<RunCommand> {
        if !log.locked().is_some_and(|l| l.ruleset.links) {
            return Vec::new();
        }
        let linked = links
            .iter()
            .filter_map(|link| match link.member(&self.me) {
                Some(Member::Caught { pid, .. }) => Some(*pid),
                _ => None,
            })
            .collect();
        let standing = LinkStanding {
            linked,
            waiting: self.waiting(links, log),
        };
        let mut commands = vec![RunCommand::LinkStanding(standing)];
        for link in links {
            let Some(why) = &link.broken else { continue };
            match link.member(&self.me) {
                Some(Member::Caught { pid, dead: false, .. }) => {
                    commands.push(RunCommand::LinkDeath {
                        pid: *pid,
                        why: why.clone(),
                    });
                }
                Some(Member::Pending) => commands.push(RunCommand::CloseSource {
                    source: link.source.clone(),
                    why: why.clone(),
                }),
                _ => {}
            }
        }
        commands
    }

    /// This Player's living Pokémon whose Link is still waiting on another
    /// Player's catch, and on whom.
    fn waiting(&self, links: &[Link], log: &RunLog) -> Vec<WaitingOnLink> {
        links
            .iter()
            .filter(|link| link.pending())
            .filter_map(|link| {
                let Some(Member::Caught { pid, dead: false, .. }) = link.member(&self.me) else {
                    return None;
                };
                let others: Vec<String> = link
                    .members
                    .iter()
                    .filter(|(player, member)| {
                        *player != self.me && matches!(member, Member::Pending | Member::Caught { judged: false, .. })
                    })
                    .map(|(player, _)| log.name_of(player))
                    .collect();
                let one = others.len() == 1;
                let who = match others.split_last() {
                    Some((last, [])) => last.clone(),
                    Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
                    None => return None,
                };
                let until = match (link.source.area_id(), one) {
                    (Some(_), true) => format!("until {who} catches on {}", link.source.name),
                    (Some(_), false) => format!("until {who} catch on {}", link.source.name),
                    (None, true) => format!("until {who} gets theirs from {}", link.source.name),
                    (None, false) => format!("until {who} get theirs from {}", link.source.name),
                };
                Some(WaitingOnLink { pid: *pid, until })
            })
            .collect()
    }
}
