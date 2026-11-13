//! The key script a headless app follows: one step a line, `#` starts a
//! comment. A script sets up or joins a lobby, plays the Run's game with
//! button presses, and checks what the Run log and the Shadow Games show.
//! `docs/headless.md` lists every step.

use std::path::PathBuf;

use pactlocke_emu::Keys;
use pactlocke_run::Preset;

/// How long an `expect` waits by default, in seconds.
pub const DEFAULT_WITHIN_S: u64 = 30;

/// One line of a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The name other Players see.
    Name(String),
    /// The relays to use: `n0`, `off`, or one URL.
    Relays(RelayArg),
    /// Opens a lobby on this ROM.
    Host { rom: PathBuf },
    /// Joins with the invite written in `invite`, on this ROM. Waits for the file.
    Join { invite: PathBuf, rom: PathBuf },
    /// The Host writes each invite link to `invite-1`, `invite-2`, … in this
    /// folder, a new one once the last was used.
    Invites(PathBuf),
    /// The folder Players share for `sync`.
    Shared(PathBuf),
    /// The Host's preset.
    Preset(Preset),
    /// The Host turns one rule on or off.
    Rule { rule: String, on: bool },
    /// The Host's Retries pool.
    Retries(u8),
    /// A battle the Game Profile doesn't list, made a Sync Gate for this
    /// test: its `trainerbattle` sites, each with the bytes a clean ROM holds.
    /// Every Player's script gives the same, before hosting or joining.
    ExtraGate { name: String, sites: Vec<(u32, Vec<u8>)> },
    /// The Host turns a Sync Gate on or off.
    Gate { gate: String, on: bool },
    /// Ready to the Ruleset as it is now.
    Ready,
    /// The Host waits until this many Players, them included, are in setup:
    /// rules for several Players (Links) can only be chosen then.
    Players(usize),
    /// The Host locks once this many Players, them included, are Ready.
    LockWhen(usize),
    /// Holds `keys` for `frames`.
    Hold { keys: Keys, frames: u32 },
    /// Mashes `keys` for `frames`: pressed 3 frames in 16.
    Mash { keys: Keys, frames: u32 },
    /// Presses `keys` for 4 frames, then waits `wait`.
    Press { keys: Keys, wait: u32 },
    /// Presses nothing for `frames`.
    Wait(u32),
    /// Plays until `cond` holds, doing `action` meanwhile; fails after `limit` frames.
    Until { cond: Cond, action: Action, limit: u32 },
    /// The game's speed: 1 to 4.
    Speed(u8),
    /// A test setup write, stamped into the input stream so Shadow Games see it.
    Poke { addr: Addr, bytes: Vec<u8> },
    /// A memory edit the Run doesn't know about: a cheat.
    CheatPoke { addr: Addr, bytes: Vec<u8> },
    /// A savestate load: the game goes back to its latest Run Checkpoint
    /// with no Retry in the log.
    CheatSavestateLoad,
    /// Starts a Retry vote, or says Yes to the open one.
    VoteRetry,
    /// Cuts this app's network for `secs`, through the rig's cut command.
    Cut { secs: u64 },
    /// Waits until every Player sharing the `shared` folder reached this label.
    Sync { label: String, players: usize },
    /// Waits until `check` holds, or fails after `within_s`.
    Expect { check: Check, within_s: u64 },
    /// Saves the game's screen as a PNG.
    Shot(PathBuf),
    /// Stops at once, as if the app's machine vanished.
    Vanish,
}

/// Which relays a headless app uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayArg {
    N0,
    Off,
    Url(String),
}

/// A place in memory: an address, or an address read from a pointer plus an offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Addr {
    At(u32),
    Deref { pointer: u32, offset: u32 },
}

/// A test on memory, for `until`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cond {
    /// Bytes read: 1, 2 or 4.
    pub width: u8,
    pub addr: Addr,
    pub mask: u32,
    pub test: Test,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Test {
    Is(u32),
    IsNot(u32),
    In(Vec<u32>),
}

impl Cond {
    /// Whether the value read from memory passes.
    pub fn holds(&self, value: u32) -> bool {
        let value = value & self.mask;
        match &self.test {
            Test::Is(v) => value == *v,
            Test::IsNot(v) => value != *v,
            Test::In(vs) => vs.contains(&value),
        }
    }
}

/// What `until` does each frame while waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Nothing,
    Mash(Keys),
    Hold(Keys),
    /// Walks one way for 16 frames, then the other.
    Walk(Keys, Keys),
}

impl Action {
    /// The keys for frame `i` of the wait.
    pub fn keys(self, i: u32) -> Keys {
        match self {
            Action::Nothing => Keys::NONE,
            Action::Mash(keys) => mashed(keys, i),
            Action::Hold(keys) => keys,
            Action::Walk(a, b) => {
                if (i / 16).is_multiple_of(2) {
                    a
                } else {
                    b
                }
            }
        }
    }
}

/// Mashing: pressed 3 frames in every 16.
pub fn mashed(keys: Keys, i: u32) -> Keys {
    if i % 16 < 3 { keys } else { Keys::NONE }
}

/// What an `expect` waits for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// Every Shadow Game matches and is caught up.
    Matched,
    /// The lobby reaches every other Player.
    Connected,
    /// A Run log entry whose kinds follow `path` (`event died`), by the
    /// Player named `by` if given, with every `field=value` given.
    Log {
        path: Vec<String>,
        by: Option<String>,
        /// The Player a field `player` names, as in a cheat flag.
        about: Option<String>,
        fields: Vec<(String, String)>,
        /// How many such entries at least.
        count: usize,
    },
    /// Nothing has gone wrong: no Run log or network error.
    NoErrors,
    /// This joiner lost the Host, connected again, and reaches every Player now.
    Reconnected,
    /// This Player's game, and every Shadow Game here, is on this boot or a
    /// later one (a Retry starts a new boot).
    Boot(u32),
}

/// A script, parsed: its steps, each with its line number.
pub fn parse(text: &str) -> Result<Vec<(usize, Step)>, String> {
    let mut steps = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        let step = step(&words).map_err(|e| format!("line {}: {e}: {line:?}", i + 1))?;
        steps.push((i + 1, step));
    }
    Ok(steps)
}

/// The script at `path` as one text: each `include FILE` line (relative to
/// the including script) replaced by that file, and each `${NAME}` by the
/// environment variable `NAME`.
pub fn load(path: &std::path::Path) -> Result<String, String> {
    load_within(path, 0)
}

fn load_within(path: &std::path::Path, depth: u32) -> Result<String, String> {
    if depth > 8 {
        return Err(format!("{}: includes nest too deep", path.display()));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{} can't be read: {e}", path.display()))?;
    let text = expand(&text, |name| std::env::var(name).ok())
        .map_err(|name| format!("{}: ${{{name}}} isn't set", path.display()))?;
    let mut out = String::new();
    for line in text.lines() {
        match line.trim().strip_prefix("include ") {
            Some(file) => {
                let file = path.parent().unwrap_or(std::path::Path::new(".")).join(file.trim());
                out.push_str(&load_within(&file, depth + 1)?);
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    Ok(out)
}

/// `text` with each `${NAME}` replaced by `var(NAME)`; the first name with no value is the error.
fn expand(text: &str, var: impl Fn(&str) -> Option<String>) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after.find('}').ok_or_else(|| after.to_owned())?;
        let name = &after[..end];
        out.push_str(&var(name).ok_or_else(|| name.to_owned())?);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn step(words: &[&str]) -> Result<Step, String> {
    Ok(match words {
        ["extra-gate", name, sites @ ..] if !sites.is_empty() => Step::ExtraGate {
            name: (*name).to_owned(),
            sites: sites
                .iter()
                .map(|site| {
                    let (at, bytes) = site.split_once(':').ok_or("a gate site is ADDR:BYTES")?;
                    Ok((hex(at)?, hex_bytes(bytes)?))
                })
                .collect::<Result<_, String>>()?,
        },
        ["name", rest @ ..] if !rest.is_empty() => Step::Name(rest.join(" ")),
        ["relays", "n0"] => Step::Relays(RelayArg::N0),
        ["relays", "off"] => Step::Relays(RelayArg::Off),
        ["relays", url] => Step::Relays(RelayArg::Url((*url).to_owned())),
        // A ROM path is the rest of the line: it may have spaces.
        ["host", rom @ ..] if !rom.is_empty() => Step::Host {
            rom: rom.join(" ").into(),
        },
        ["join", invite, rom @ ..] if !rom.is_empty() => Step::Join {
            invite: invite.into(),
            rom: rom.join(" ").into(),
        },
        ["invites", dir] => Step::Invites(dir.into()),
        ["shared", dir] => Step::Shared(dir.into()),
        ["preset", name] => Step::Preset(preset(name)?),
        ["rule", rule, on] => Step::Rule {
            rule: rule_name(rule)?,
            on: on_off(on)?,
        },
        ["retries", n] => match number(n)? {
            n if n <= u32::from(pactlocke_run::MAX_RETRIES) => Step::Retries(n as u8),
            _ => return Err(format!("at most {} Retries", pactlocke_run::MAX_RETRIES)),
        },
        ["gate", rest @ .., on] if !rest.is_empty() => Step::Gate {
            gate: rest.join(" "),
            on: on_off(on)?,
        },
        ["ready"] => Step::Ready,
        ["players", n] => Step::Players(number(n)? as usize),
        ["lock-when", n] => Step::LockWhen(number(n)? as usize),
        ["h", k, n] => Step::Hold {
            keys: keys(k)?,
            frames: number(n)?,
        },
        ["m", k, n] => Step::Mash {
            keys: keys(k)?,
            frames: number(n)?,
        },
        ["p", k, n] => Step::Press {
            keys: keys(k)?,
            wait: number(n)?,
        },
        ["w", n] => Step::Wait(number(n)?),
        ["until", rest @ ..] => until(rest)?,
        ["speed", n] => match number(n)? {
            n @ 1..=4 => Step::Speed(n as u8),
            _ => return Err("speed is 1 to 4".into()),
        },
        ["poke", addr, bytes] => Step::Poke {
            addr: address(addr)?,
            bytes: hex_bytes(bytes)?,
        },
        ["cheat", "poke", addr, bytes] => Step::CheatPoke {
            addr: address(addr)?,
            bytes: hex_bytes(bytes)?,
        },
        ["cheat", "savestate-load"] => Step::CheatSavestateLoad,
        ["vote", "retry"] => Step::VoteRetry,
        ["cut", secs] => Step::Cut {
            secs: number(secs)?.into(),
        },
        ["sync", label, players] => Step::Sync {
            label: (*label).to_owned(),
            players: number(players)? as usize,
        },
        ["expect", rest @ ..] => expect(rest)?,
        ["shot", path @ ..] if !path.is_empty() => Step::Shot(path.join(" ").into()),
        ["vanish"] => Step::Vanish,
        _ => return Err("not a step".into()),
    })
}

/// `until memN ADDR [&MASK] (== V | != V | in V,V) [mash K | hold K | walk K K] LIMIT`
fn until(words: &[&str]) -> Result<Step, String> {
    let [width, addr, rest @ ..] = words else {
        return Err("until needs memN and an address".into());
    };
    let width = match *width {
        "mem8" => 1,
        "mem16" => 2,
        "mem32" => 4,
        _ => return Err("until reads mem8, mem16 or mem32".into()),
    };
    let addr = address(addr)?;
    let (mask, rest) = match rest {
        [m, rest @ ..] if m.starts_with('&') => (hex(&m[1..])?, rest),
        _ => (u32::MAX, rest),
    };
    let (test, rest) = match rest {
        ["==", v, rest @ ..] => (Test::Is(hex(v)?), rest),
        ["!=", v, rest @ ..] => (Test::IsNot(hex(v)?), rest),
        ["in", vs, rest @ ..] => (Test::In(vs.split(',').map(hex).collect::<Result<_, _>>()?), rest),
        _ => return Err("until tests with ==, != or in".into()),
    };
    let (action, limit) = match rest {
        [limit] => (Action::Nothing, limit),
        ["mash", k, limit] => (Action::Mash(keys(k)?), limit),
        ["hold", k, limit] => (Action::Hold(keys(k)?), limit),
        ["walk", a, b, limit] => (Action::Walk(keys(a)?, keys(b)?), limit),
        _ => return Err("until ends with an action and a frame limit".into()),
    };
    Ok(Step::Until {
        cond: Cond {
            width,
            addr,
            mask,
            test,
        },
        action,
        limit: number(limit)?,
    })
}

/// `expect CHECK [within SECS]`
fn expect(words: &[&str]) -> Result<Step, String> {
    let (words, within_s) = match words {
        [rest @ .., "within", secs] => (rest, number(secs)?.into()),
        rest => (rest, DEFAULT_WITHIN_S),
    };
    let check = match words {
        ["matched"] => Check::Matched,
        ["connected"] => Check::Connected,
        ["no-errors"] => Check::NoErrors,
        ["reconnected"] => Check::Reconnected,
        ["boot", n] => Check::Boot(number(n)?),
        ["log", rest @ ..] if !rest.is_empty() => {
            let mut path = Vec::new();
            let (mut by, mut about, mut fields, mut count) = (None, None, Vec::new(), 1);
            let mut rest = rest.iter();
            while let Some(word) = rest.next() {
                match *word {
                    "by" => by = Some(rest.next().ok_or("by needs a name")?.to_string()),
                    "about" => about = Some(rest.next().ok_or("about needs a name")?.to_string()),
                    "x" => count = number(rest.next().ok_or("x needs a count")?)? as usize,
                    w if w.contains('=') => {
                        let (k, v) = w.split_once('=').expect("has =");
                        fields.push((k.to_owned(), v.to_owned()));
                    }
                    w => path.push(w.to_owned()),
                }
            }
            if path.is_empty() {
                return Err("expect log needs an entry kind".into());
            }
            Check::Log {
                path,
                by,
                about,
                fields,
                count,
            }
        }
        _ => return Err("not a check".into()),
    };
    Ok(Step::Expect { check, within_s })
}

/// One key, or several joined by `+`: `A`, `B`, `U`, `D`, `L`, `R`, `S`
/// (Start), `E` (Select), `LS` and `RS` (the shoulder buttons).
pub fn keys(name: &str) -> Result<Keys, String> {
    let mut all = Keys::NONE;
    for key in name.split('+') {
        all |= match key {
            "A" => Keys::A,
            "B" => Keys::B,
            "U" => Keys::UP,
            "D" => Keys::DOWN,
            "L" => Keys::LEFT,
            "R" => Keys::RIGHT,
            "S" => Keys::START,
            "E" => Keys::SELECT,
            "LS" => Keys::L,
            "RS" => Keys::R,
            other => return Err(format!("no key {other}")),
        };
    }
    Ok(all)
}

fn preset(name: &str) -> Result<Preset, String> {
    Ok(match name {
        "classic" => Preset::ClassicNuzlocke,
        "soul-link" => Preset::SoulLink,
        "hardcore" => Preset::Hardcore,
        _ => return Err("presets are classic, soul-link and hardcore".into()),
    })
}

/// The rules a script can turn on or off, by their Ruleset names.
pub const RULE_NAMES: [&str; 11] = [
    "first_encounter",
    "dupes_clause",
    "statics",
    "gifts",
    "faint_is_dead",
    "links",
    "no_items",
    "set_mode",
    "must_nickname",
    "type_clause",
    "level_caps",
];

/// Turns the rule called `rule` on or off: one of [`RULE_NAMES`]. `level_caps`
/// on uses the ROM's own caps. Returns whether there is such a rule.
pub fn set_rule(ruleset: &mut pactlocke_run::Ruleset, rule: &str, on: bool, caps: &[pactlocke_game::LevelCap]) -> bool {
    let flag = match rule {
        "first_encounter" => &mut ruleset.first_encounter,
        "dupes_clause" => &mut ruleset.dupes_clause,
        "statics" => &mut ruleset.statics,
        "gifts" => &mut ruleset.gifts,
        "faint_is_dead" => &mut ruleset.faint_is_dead,
        "links" => &mut ruleset.links,
        "no_items" => &mut ruleset.no_items,
        "set_mode" => &mut ruleset.set_mode,
        "must_nickname" => &mut ruleset.must_nickname,
        "type_clause" => &mut ruleset.type_clause,
        "level_caps" => {
            ruleset.level_caps = on.then(|| caps.to_vec());
            return true;
        }
        _ => return false,
    };
    *flag = on;
    true
}

fn rule_name(name: &str) -> Result<String, String> {
    RULE_NAMES
        .contains(&name)
        .then(|| name.to_owned())
        .ok_or_else(|| format!("no rule {name}; the rules are {}", RULE_NAMES.join(", ")))
}

fn on_off(word: &str) -> Result<bool, String> {
    match word {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err("on or off".into()),
    }
}

fn number(word: &str) -> Result<u32, String> {
    word.parse().map_err(|_| format!("{word} isn't a number"))
}

fn hex(word: &str) -> Result<u32, String> {
    let digits = word.strip_prefix("0x").unwrap_or(word);
    u32::from_str_radix(digits, 16).map_err(|_| format!("{word} isn't hex"))
}

/// `0x02024284`, or `*0x0300500C+0x14` for an offset from a pointer.
fn address(word: &str) -> Result<Addr, String> {
    match word.strip_prefix('*') {
        Some(rest) => {
            let (pointer, offset) = rest.split_once('+').unwrap_or((rest, "0"));
            Ok(Addr::Deref {
                pointer: hex(pointer)?,
                offset: hex(offset)?,
            })
        }
        None => Ok(Addr::At(hex(word)?)),
    }
}

/// Bytes as hex, in memory order: `0201` is 0x02 then 0x01.
fn hex_bytes(word: &str) -> Result<Vec<u8>, String> {
    let word = word.strip_prefix("0x").unwrap_or(word);
    if word.is_empty() || word.len() % 2 == 1 {
        return Err(format!("{word} isn't whole bytes of hex"));
    }
    (0..word.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&word[i..i + 2], 16).map_err(|_| format!("{word} isn't hex")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(line: &str) -> Step {
        let steps = parse(line).unwrap();
        assert_eq!(steps.len(), 1);
        steps.into_iter().next().unwrap().1
    }

    #[test]
    fn lobby_steps_parse() {
        let script = "# Red hosts\nname Red the Brave\nrelays http://relay:3340\nhost /roms/Fire Red.gba\n\
                      invites /rig/shared\npreset soul-link\nrule links on\nretries 2\ngate Rival in the lab on\n\
                      ready\nplayers 3\nlock-when 3\n";
        let steps: Vec<Step> = parse(script).unwrap().into_iter().map(|(_, s)| s).collect();
        assert_eq!(
            steps,
            vec![
                Step::Name("Red the Brave".into()),
                Step::Relays(RelayArg::Url("http://relay:3340".into())),
                Step::Host {
                    rom: "/roms/Fire Red.gba".into()
                },
                Step::Invites("/rig/shared".into()),
                Step::Preset(Preset::SoulLink),
                Step::Rule {
                    rule: "links".into(),
                    on: true
                },
                Step::Retries(2),
                Step::Gate {
                    gate: "Rival in the lab".into(),
                    on: true
                },
                Step::Ready,
                Step::Players(3),
                Step::LockWhen(3),
            ]
        );
    }

    #[test]
    fn line_numbers_skip_comments_and_blanks() {
        let steps = parse("\n# hi\nw 10  # wait\n").unwrap();
        assert_eq!(steps, vec![(3, Step::Wait(10))]);
    }

    #[test]
    fn key_steps_parse_as_the_acceptance_tests_write_them() {
        assert_eq!(
            one("m A 5100"),
            Step::Mash {
                keys: Keys::A,
                frames: 5100
            }
        );
        assert_eq!(
            one("h R+B 64"),
            Step::Hold {
                keys: Keys::RIGHT | Keys::B,
                frames: 64
            }
        );
        assert_eq!(
            one("p S 90"),
            Step::Press {
                keys: Keys::START,
                wait: 90
            }
        );
    }

    #[test]
    fn until_reads_memory_with_a_mask_and_a_test() {
        let Step::Until { cond, action, limit } =
            one("until mem32 0x03004FE0 &FFFFFFFE in 0x0802E438,0x080E763C mash B 3000")
        else {
            panic!("an until");
        };
        assert_eq!(cond.width, 4);
        assert_eq!(cond.addr, Addr::At(0x0300_4FE0));
        assert!(cond.holds(0x0802_E439), "the low bit is masked off");
        assert!(!cond.holds(0x0802_E43A));
        assert_eq!(action, Action::Mash(Keys::B));
        assert_eq!(limit, 3000);

        let Step::Until { cond, action, .. } = one("until mem8 0x03003529 &02 != 0 walk L R 9000") else {
            panic!("an until");
        };
        assert!(cond.holds(0x02) && !cond.holds(0x01));
        assert_eq!(action, Action::Walk(Keys::LEFT, Keys::RIGHT));
        assert_eq!(action.keys(0), Keys::LEFT);
        assert_eq!(action.keys(16), Keys::RIGHT);
    }

    #[test]
    fn pokes_take_an_address_or_a_pointer_and_bytes_in_memory_order() {
        assert_eq!(
            one("poke *0x0300500C+0x14 02"),
            Step::Poke {
                addr: Addr::Deref {
                    pointer: 0x0300_500C,
                    offset: 0x14
                },
                bytes: vec![2]
            }
        );
        assert_eq!(
            one("cheat poke 0x02024284 0100"),
            Step::CheatPoke {
                addr: Addr::At(0x0202_4284),
                bytes: vec![1, 0]
            }
        );
        assert_eq!(one("cheat savestate-load"), Step::CheatSavestateLoad);
    }

    #[test]
    fn expect_log_takes_kinds_a_player_fields_and_a_count() {
        assert_eq!(
            one("expect log event cheat_flag cheat=restarted_without_retry by Red about Blue x 2 within 90"),
            Step::Expect {
                check: Check::Log {
                    path: vec!["event".into(), "cheat_flag".into()],
                    by: Some("Red".into()),
                    about: Some("Blue".into()),
                    fields: vec![("cheat".into(), "restarted_without_retry".into())],
                    count: 2,
                },
                within_s: 90
            }
        );
        assert_eq!(
            one("expect matched"),
            Step::Expect {
                check: Check::Matched,
                within_s: DEFAULT_WITHIN_S
            }
        );
    }

    #[test]
    fn extra_gates_and_variables() {
        assert_eq!(
            one("extra-gate Rival 0x081693AC:5C09460103 0x08169424:5C09480103"),
            Step::ExtraGate {
                name: "Rival".into(),
                sites: vec![
                    (0x0816_93AC, vec![0x5C, 0x09, 0x46, 0x01, 0x03]),
                    (0x0816_9424, vec![0x5C, 0x09, 0x48, 0x01, 0x03]),
                ]
            }
        );
        let var = |name: &str| (name == "ROM").then(|| "/roms/fr.gba".to_owned());
        assert_eq!(
            expand("host ${ROM} # ${ROM}", var).unwrap(),
            "host /roms/fr.gba # /roms/fr.gba"
        );
        assert_eq!(expand("host ${NOPE}", var).unwrap_err(), "NOPE");
    }

    #[test]
    fn includes_are_read_relative_to_the_script() {
        let dir = std::env::temp_dir().join(format!("pactlocke-script-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("common")).unwrap();
        std::fs::write(dir.join("common/start.txt"), "m A 10\n").unwrap();
        std::fs::write(dir.join("red.txt"), "name Red\ninclude common/start.txt\nw 5\n").unwrap();
        let text = load(&dir.join("red.txt")).unwrap();
        let steps: Vec<Step> = parse(&text).unwrap().into_iter().map(|(_, s)| s).collect();
        assert_eq!(steps.len(), 3);
        assert_eq!(
            steps[1],
            Step::Mash {
                keys: Keys::A,
                frames: 10
            }
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn every_rule_a_script_names_can_be_set() {
        let mut ruleset = pactlocke_run::Ruleset::default();
        for rule in RULE_NAMES {
            assert!(set_rule(&mut ruleset, rule, true, &[]), "{rule}");
        }
        assert!(ruleset.links && ruleset.type_clause && ruleset.level_caps.is_some());
        assert!(!set_rule(&mut ruleset, "no_such_rule", true, &[]));
        assert_eq!(
            one("expect reconnected within 9").clone(),
            Step::Expect {
                check: Check::Reconnected,
                within_s: 9
            }
        );
        assert_eq!(
            one("expect boot 2"),
            Step::Expect {
                check: Check::Boot(2),
                within_s: DEFAULT_WITHIN_S
            }
        );
    }

    #[test]
    fn mistakes_name_the_line() {
        let e = parse("w 10\nm Q 5").unwrap_err();
        assert!(e.starts_with("line 2: no key Q"), "{e}");
        assert!(parse("rule nothing on").unwrap_err().contains("the rules are"));
        assert!(parse("speed 9").is_err());
        assert!(parse("poke 0x0 123").is_err());
        assert!(parse("dance").is_err());
    }
}
