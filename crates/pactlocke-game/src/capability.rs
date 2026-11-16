//! The capability report: for each game-dependent Ruleset rule, whether this
//! ROM can enforce it, leaves it to the game's own option, or can't, and why.

use std::collections::BTreeMap;

use crate::SourceKind;
use crate::profile::{GameProfile, HexBytes, Method, ROM_BASE};

/// The Ruleset rules whose enforcement depends on the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    FirstEncounter,
    DupesClause,
    Gifts,
    Statics,
    FaintIsDead,
    Links,
    LevelCap,
    NoItems,
    MustNickname,
    SetMode,
    TypeClause,
    SyncGates,
}

impl Rule {
    pub const ALL: [Rule; 12] = [
        Rule::FirstEncounter,
        Rule::DupesClause,
        Rule::Gifts,
        Rule::Statics,
        Rule::FaintIsDead,
        Rule::Links,
        Rule::LevelCap,
        Rule::NoItems,
        Rule::MustNickname,
        Rule::SetMode,
        Rule::TypeClause,
        Rule::SyncGates,
    ];

    /// The rule's name as Players see it.
    pub fn name(self) -> &'static str {
        match self {
            Rule::FirstEncounter => "First Encounter",
            Rule::DupesClause => "Dupes clause",
            Rule::Gifts => "Gifts",
            Rule::Statics => "Statics",
            Rule::FaintIsDead => "Faint = Dead",
            Rule::Links => "Links",
            Rule::LevelCap => "Level caps",
            Rule::NoItems => "No items in battle",
            Rule::MustNickname => "Must nickname",
            Rule::SetMode => "Set mode",
            Rule::TypeClause => "Type clause",
            Rule::SyncGates => "Sync Gates",
        }
    }
}

/// A rule the game's own option does, as its Game Profile lists it:
/// Pactlocke sets the option at the start of every boot and, when it is
/// watched, sets it back whenever the Player changes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeOption {
    /// The Pactlocke rule the option does.
    pub rule: Rule,
    /// The option's key in the Game Profile.
    pub option: String,
    /// Its in-game name: "BATTLE STYLE".
    pub name: String,
    /// Where it is in the game's menus: "OPTIONS ▸ BATTLE STYLE".
    pub menu: String,
    /// The setting the rule needs.
    pub value: String,
    /// Whether a change mid-Run is set back, or the option only set at each boot.
    pub watched: bool,
    /// Other rules that must be on too for the option to be used (Heart and
    /// Soul's Nuzlocke mode needs first Encounter and faint = Dead).
    pub with: Vec<Rule>,
    /// Whether Pactlocke still enforces the rule its own way: the option
    /// goes with it rather than replacing it.
    pub pactlocke_too: bool,
}

impl NativeOption {
    /// Whether a Ruleset uses this option: its rule and every one it goes with are on.
    pub fn in_use(&self, has: impl Fn(Rule) -> bool) -> bool {
        has(self.rule) && self.with.iter().all(|&rule| has(rule))
    }
}

/// An option of the game's that Pactlocke sets for a rule, as a Game Profile
/// lists it beside the rule methods.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleOptionDef {
    pub rule: Rule,
    pub option: String,
    pub value: String,
    /// Other rules that must be on too.
    #[serde(default)]
    pub with: Vec<Rule>,
    /// Whether Pactlocke still enforces the rule its own way.
    #[serde(default)]
    pub pactlocke_too: bool,
}

impl GameProfile {
    /// Every rule this game does through its own option.
    pub fn native_options(&self) -> Vec<NativeOption> {
        let methods = &self.rules;
        let mut found: Vec<NativeOption> = Vec::new();
        for (rule, method) in [
            (Rule::FirstEncounter, &methods.catch_block),
            (Rule::LevelCap, &methods.level_cap),
            (Rule::NoItems, &methods.no_items_trainer),
            (Rule::NoItems, &methods.no_items_wild),
            (Rule::MustNickname, &methods.must_nickname),
            (Rule::SetMode, &methods.set_mode),
        ] {
            let Some(Method::Native { option, value }) = method else {
                continue;
            };
            if found.iter().any(|n| n.rule == rule && n.option == *option) {
                continue;
            }
            // The store checked the option exists.
            let def = &self.options[option];
            found.push(NativeOption {
                rule,
                option: option.clone(),
                name: def.name.clone(),
                menu: def.menu.clone(),
                value: value.clone(),
                watched: def.watched,
                with: Vec::new(),
                pactlocke_too: false,
            });
        }
        for listed in &self.rule_options {
            let def = &self.options[&listed.option];
            found.push(NativeOption {
                rule: listed.rule,
                option: listed.option.clone(),
                name: def.name.clone(),
                menu: def.menu.clone(),
                value: listed.value.clone(),
                watched: def.watched,
                with: listed.with.clone(),
                pactlocke_too: listed.pactlocke_too,
            });
        }
        found
    }
}

/// How one rule can be applied on this ROM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capability {
    /// Pactlocke enforces it.
    Enforceable,
    /// The game's own option does it; Pactlocke sets the option.
    NativeOption(NativeOption),
    /// It can't be used on this ROM.
    GreyedOut { reason: String },
}

/// Every game-dependent rule's capability on one ROM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityReport {
    rules: BTreeMap<Rule, Capability>,
    /// The game's options Pactlocke sets for a rule beside its method.
    rule_options: Vec<NativeOption>,
    /// Whether the game has its own randomizer, whose seed Shared Seed can share.
    own_randomizer: bool,
}

impl CapabilityReport {
    /// Checks every patch site's bytes in `rom` and reports each rule.
    pub fn check(profile: &GameProfile, rom: &[u8]) -> Self {
        let methods = &profile.rules;
        let catch = method_capability(profile, methods.catch_block.as_ref(), rom);
        let no_items = match method_capability(profile, methods.no_items_trainer.as_ref(), rom) {
            Capability::Enforceable => method_capability(profile, methods.no_items_wild.as_ref(), rom),
            other => other,
        };

        let rules = Rule::ALL
            .into_iter()
            .map(|rule| {
                let capability = match rule {
                    Rule::FirstEncounter | Rule::DupesClause => catch.clone(),
                    Rule::Gifts => catalogue_capability(profile, |kind| kind != SourceKind::Static, "Gifts"),
                    Rule::Statics => catalogue_capability(profile, |kind| kind == SourceKind::Static, "Statics"),
                    Rule::SyncGates => sync_gate_capability(profile, rom),
                    // Read and written in RAM through the profile's symbols and layout.
                    Rule::FaintIsDead | Rule::Links | Rule::TypeClause => Capability::Enforceable,
                    Rule::LevelCap => method_capability(profile, methods.level_cap.as_ref(), rom),
                    Rule::NoItems => no_items.clone(),
                    Rule::MustNickname => method_capability(profile, methods.must_nickname.as_ref(), rom),
                    Rule::SetMode => method_capability(profile, methods.set_mode.as_ref(), rom),
                };
                (rule, capability)
            })
            .collect();
        let rule_options = profile
            .native_options()
            .into_iter()
            .filter(|native| native.pactlocke_too || !native.with.is_empty())
            .collect();
        Self {
            rules,
            rule_options,
            // Shared Seed needs the Romhack's own randomizer, its code as the profile expects.
            own_randomizer: profile.own_randomizer.as_ref().is_some_and(|own| {
                matches!(
                    method_capability(
                        profile,
                        Some(&Method::Patch {
                            sites: own.sites.clone()
                        }),
                        rom
                    ),
                    Capability::Enforceable
                )
            }),
        }
    }

    /// Every rule enforceable, as on a game that can do them all (the fake game in tests).
    pub fn everything_enforceable() -> Self {
        Self {
            rules: Rule::ALL
                .into_iter()
                .map(|rule| (rule, Capability::Enforceable))
                .collect(),
            rule_options: Vec::new(),
            own_randomizer: false,
        }
    }

    /// The same report for a game with its own randomizer.
    pub fn with_own_randomizer(mut self) -> Self {
        self.own_randomizer = true;
        self
    }

    /// Whether the game has its own randomizer, whose seed Shared Seed can share.
    pub fn own_randomizer(&self) -> bool {
        self.own_randomizer
    }

    /// The same report with an option the game sets for a rule beside its method.
    pub fn with_rule_option(mut self, native: NativeOption) -> Self {
        self.rule_options.push(native);
        self
    }

    /// Every native option on this ROM: the rules the game does, and the
    /// options Pactlocke sets beside its own methods.
    pub fn native_options(&self) -> impl Iterator<Item = &NativeOption> {
        let natives = self.rules.values().filter_map(|capability| match capability {
            Capability::NativeOption(native) => Some(native),
            _ => None,
        });
        natives.chain(&self.rule_options)
    }

    /// The same report with one rule's capability changed.
    pub fn with(mut self, rule: Rule, capability: Capability) -> Self {
        self.rules.insert(rule, capability);
        self
    }

    pub fn get(&self, rule: Rule) -> &Capability {
        &self.rules[&rule]
    }

    pub fn iter(&self) -> impl Iterator<Item = (Rule, &Capability)> {
        self.rules.iter().map(|(rule, capability)| (*rule, capability))
    }
}

/// Enforceable when the profile lists gates, every gate's `trainerbattle`
/// holds what it expects, and the hold area is free.
fn sync_gate_capability(profile: &GameProfile, rom: &[u8]) -> Capability {
    let Some(gates) = profile.sync_gates.as_ref().filter(|g| !g.gates.is_empty()) else {
        return Capability::GreyedOut {
            reason: "this Game Profile has no Sync Gate trainers yet".into(),
        };
    };
    let sites = gates.gates.iter().flat_map(|g| g.sites.iter().cloned()).collect();
    if let Capability::GreyedOut { reason } = method_capability(profile, Some(&Method::Patch { sites }), rom) {
        return Capability::GreyedOut { reason };
    }
    let start = (gates.hold - ROM_BASE) as usize;
    let free = rom
        .get(start..start + gates.hold_len())
        .is_some_and(|area| area.iter().all(|b| *b == 0xFF));
    if free {
        Capability::Enforceable
    } else {
        Capability::GreyedOut {
            reason: format!("the ROM isn't free at {:#010X}, where the hold scripts go", gates.hold),
        }
    }
}

/// Enforceable when the catalogue lists any source of the kinds `wanted` picks.
fn catalogue_capability(profile: &GameProfile, wanted: impl Fn(SourceKind) -> bool, what: &str) -> Capability {
    if profile.sources.iter().any(|source| wanted(source.kind)) {
        Capability::Enforceable
    } else {
        Capability::GreyedOut {
            reason: format!("this Game Profile has no {what} in its catalogue yet"),
        }
    }
}

fn method_capability(profile: &GameProfile, method: Option<&Method>, rom: &[u8]) -> Capability {
    match method {
        None => Capability::GreyedOut {
            reason: "this Game Profile has no way to enforce it".into(),
        },
        Some(Method::Patch { sites }) => sites
            .iter()
            .find_map(|site| {
                let start = (site.at - ROM_BASE) as usize;
                let expect = &site.expect.0;
                let Some(found) = rom.get(start..start + expect.len()) else {
                    return Some(format!("{:#010X} is past the end of the ROM", site.at));
                };
                (found != expect.as_slice()).then(|| {
                    format!(
                        "the ROM holds {} at {:#010X} where the Game Profile expects {}",
                        HexBytes(found.to_vec()),
                        site.at,
                        site.expect
                    )
                })
            })
            .map_or(Capability::Enforceable, |reason| Capability::GreyedOut { reason }),
        Some(Method::Native { option, value }) => profile
            .native_options()
            .into_iter()
            .find(|native| native.option == *option && native.value == *value)
            .map_or(
                Capability::GreyedOut {
                    reason: format!("the Game Profile has no native option {option:?}"),
                },
                Capability::NativeOption,
            ),
        Some(Method::KeyFilter {
            command_table, command, ..
        }) => {
            // The command must be one of the table's entries, or the filter
            // would never see it run.
            let start = (command_table - ROM_BASE) as usize;
            let table = rom.get(start..).unwrap_or_default();
            let found = table
                .as_chunks::<4>()
                .0
                .iter()
                .take(COMMAND_TABLE_ENTRIES)
                .any(|entry| u32::from_le_bytes(*entry) == *command);
            if found {
                Capability::Enforceable
            } else {
                Capability::GreyedOut {
                    reason: format!(
                        "the battle script command table at {command_table:#010X} has no command {command:#010X}"
                    ),
                }
            }
        }
        Some(Method::ZeroTableField | Method::OptionBit { .. } | Method::SetFlag { .. } | Method::Always) => {
            Capability::Enforceable
        }
    }
}

/// Generation III battle scripts have fewer commands than this.
const COMMAND_TABLE_ENTRIES: usize = 0x100;
