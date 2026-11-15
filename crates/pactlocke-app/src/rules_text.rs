//! What each rule does, in one place: a clean description under each rule
//! while the Ruleset is chosen (the New Run prompt and the Lobby's setup),
//! and in-depth help on hover in the Ruleset tab during the Run. Numbers
//! the text depends on come from the Ruleset.

use eframe::egui;
use pactlocke_game::Rule;
use pactlocke_run::{Preset, Ruleset};

/// The rule's name, as the checkboxes and the Ruleset tab show it.
pub fn name(rule: Rule) -> &'static str {
    match rule {
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

/// What the rule does, in a sentence or two, shown under it while the
/// Ruleset is chosen.
pub fn summary(rule: Rule, ruleset: &Ruleset) -> String {
    match rule {
        Rule::FirstEncounter => {
            "Only the first wild battle in each Area is its Encounter: catch that Pokémon, or that Area gives you nothing. Ball throws are refused in every later wild battle there.".to_owned()
        }
        Rule::DupesClause => {
            "A wild Pokémon from an evolution family you already have alive doesn't use up the Area's Encounter, so you can look for another.".to_owned()
        }
        Rule::Gifts => {
            "Keep the Pokémon the game gives you (a Gift), besides your starter. When this is off, each Gift is released as soon as you get it.".to_owned()
        }
        Rule::Statics => {
            "A Static (a legendary, a Snorlax in the road, the Pokémon you fight in a scripted battle) is a bonus Encounter of its own, besides its Area's.".to_owned()
        }
        Rule::FaintIsDead => {
            "A Pokémon that faints is Dead: it goes to the Dead box and can't be used again. If your whole party is Dead, the Run is over: a game over.".to_owned()
        }
        Rule::Links => {
            "Each Player's Encounter in the same Area is linked. If one of them faints, all of them die. Only Pokémon whose Link is whole may be in your party: one still waiting on another Player's catch goes to the PC when your party is full.".to_owned()
        }
        Rule::LevelCap => match ruleset.level_caps.as_ref().and_then(|caps| caps.first()) {
            Some(first) => format!(
                "Your Pokémon can't level past the cap before the next gym leader or boss: the first is level {} for {}. Experience stops at the cap.",
                first.level, first.name
            ),
            None => "Your Pokémon can't level past the cap before the next gym leader or boss. Experience stops at the cap.".to_owned(),
        },
        Rule::NoItems => {
            "The Bag is refused in trainer battles. In wild battles only Poké Balls work. Outside battle you can use items as usual.".to_owned()
        }
        Rule::MustNickname => {
            "Every Pokémon you catch or get as a Gift needs a nickname. If you skip the game's naming screen, the side panel asks you for one.".to_owned()
        }
        Rule::SetMode => {
            "The battle style stays on Set: you aren't offered a switch when you knock out a trainer's Pokémon.".to_owned()
        }
        Rule::TypeClause => {
            "Only one Pokémon of each primary type in your party. One that joins with a type your party already has goes to the PC.".to_owned()
        }
        Rule::SyncGates => format!(
            "Every Player waits at each ticked gym leader or boss until everyone is Ready there, then you all fight it together ({} ticked).",
            ruleset.sync_gates.len()
        ),
    }
}

/// The in-depth help for the Ruleset tab, on hover: the details and the edge cases.
pub fn detail(rule: Rule, ruleset: &Ruleset) -> String {
    let full = pactlocke_run::PARTY_SIZE;
    let links;
    let more = match rule {
        Rule::FirstEncounter => {
            "What counts: the first wild battle in an Area once you have Poké Balls, whatever you do in it (catch it, run, knock it out). Wild battles before you have Poké Balls don't count, and neither do trainer battles. A battle in a place the Game Profile doesn't list as an Area is left alone."
        }
        Rule::DupesClause => {
            "The family is the whole evolution line: with a living Charmander, a wild Charmeleon is a dupe. A Dead Pokémon's family is open again. When a dupe shows up, ball throws are refused and the Area's Encounter stays unused, so the next wild battle there is the Encounter."
        }
        Rule::Gifts => {
            "Gifts are the Pokémon the game hands you: the Eevee in Celadon, the Lapras in Silph Co., fossils, eggs. Your starter is always kept. With Links on, a Gift is linked across Players like an Encounter, and you can skip one from the Links tab; skipping it breaks its Link for everyone."
        }
        Rule::Statics => {
            "With Statics off, a Static is part of its Area: it is the Area's Encounter if it comes first. With them on, each Static is an Encounter of its own, and catching it doesn't use up the Area's."
        }
        Rule::FaintIsDead => {
            "A Pokémon that faints in battle is Dead at once; it goes to the Dead box once you are back on the overworld, before anything can heal it. Poison fainting one on the overworld kills it too. Putting a Dead Pokémon back in your party is a Violation, and it goes straight back. When every Pokémon in your party is Dead (living ones in the PC don't count), the game stops: with a Retry left and a Run Checkpoint to go back to you are offered a Retry, and otherwise it is a game over. With friends, one Player's wipe is a game over for everyone, unless everyone votes to Retry."
        }
        Rule::Links => {
            links = format!(
                "A Link is made of every Player's Encounter from the same source: an Area, a Gift or a Static. Your catch waits on the others until each of them has had their Encounter there. A Link breaks when any member dies, or when a Player misses, skips or loses their Encounter: then every member dies, and a Player who hadn't had it yet can't catch there any more. Only Pokémon whose Link is whole belong in your party. A catch waiting on its Link stays in a party of fewer than {full}; when your party has {full}, the newest one waiting goes to the PC until its Link is whole. The other Players' catches and Deaths are checked against their games, not taken on their word."
            );
            &links
        }
        Rule::LevelCap => {
            "Each cap holds until you beat the gym leader or boss it is for, then the next one applies. A Pokémon at the cap gains no experience, and a Rare Candy does nothing to it. Pokémon already over the cap keep their levels."
        }
        Rule::NoItems => "The ban is in battle only: heal up between battles as you like.",
        Rule::MustNickname => {
            "The game's \"Give a nickname?\" can't be declined. A Pokémon left with its species name (a trade, say) is named from the side panel; a blank name or the species name is refused."
        }
        Rule::SetMode => "The game's battle style option is kept on Set for the whole Run.",
        Rule::TypeClause => {
            "The primary type is the first of the species' types, read from your ROM. Eggs and Dead Pokémon hold no type. The Pokémon already in your party keep their place; the newcomer goes to the first PC box with room."
        }
        Rule::SyncGates => {
            "Talking to a gated leader holds you with a message until every Player is Ready there. Once the last Player is Ready, everyone's game takes a Run Checkpoint, and the battle goes ahead."
        }
    };
    format!("{} {more}", summary(rule, ruleset))
}

/// Retries: what the pool is and how one is taken, with this Ruleset's pool.
pub fn retries(ruleset: &Ruleset) -> String {
    match ruleset.retries {
        0 => "Retries are off: what happens stays.".to_owned(),
        pool => format!(
            "The Run has {pool} {}. A Retry sends every Player's game back to the latest Run Checkpoint (an in-game save, or a Sync Gate), undoing everything since. Alone you take one when you like; with friends everyone must vote Yes, and the first No ends the vote.",
            plural(pool, "Retry", "Retries")
        ),
    }
}

/// "1 Retry", "2 Retries": the word for `n` of them.
pub fn plural<'a>(n: u8, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
}

/// What a preset turns on.
pub fn preset(preset: Preset) -> &'static str {
    match preset {
        Preset::ClassicNuzlocke => {
            "Classic Nuzlocke: First Encounter, Faint = Dead and the dupes clause, with Gifts kept."
        }
        Preset::SoulLink => "Soul Link: the classic rules, plus Links between the Players' Pokémon.",
        Preset::Hardcore => "Hardcore: the classic rules, plus level caps, no items in battle and set mode.",
    }
}

/// A rule's checkbox with its description under it, greyed out with the
/// reason when it can't be chosen.
pub fn rule_row(ui: &mut egui::Ui, on: &mut bool, rule: Rule, ruleset: &Ruleset, unavailable: Option<String>) {
    let checkbox = ui.add_enabled(unavailable.is_none(), egui::Checkbox::new(on, name(rule)));
    if let Some(reason) = &unavailable {
        checkbox.on_disabled_hover_text(format!("Unavailable: {reason}"));
    }
    description(ui, &summary(rule, ruleset), unavailable.as_deref());
}

/// The game's own options the chosen rules are done with, and where each
/// is in the game's menus: Pactlocke sets them in every Player's game.
pub fn native_options(ui: &mut egui::Ui, settings: &[pactlocke_run::NativeOptionView]) {
    if settings.is_empty() {
        return;
    }
    ui.add_space(4.0);
    ui.strong("The game's own options");
    ui.add(
        egui::Label::new(
            egui::RichText::new(
                "This game does these rules itself. Pactlocke sets each option in every Player's game, and sets a watched one back if it is changed.",
            )
            .weak()
            .small(),
        )
        .wrap(),
    );
    for setting in settings {
        ui.add(
            egui::Label::new(format!(
                "{}: {} set to {} ({})",
                setting.rule, setting.name, setting.value, setting.menu
            ))
            .wrap(),
        );
    }
}

/// A rule's description, indented under its checkbox and wrapped to the
/// width there is; with the reason it can't be chosen, when it can't.
pub fn description(ui: &mut egui::Ui, text: &str, unavailable: Option<&str>) {
    ui.indent(text, |ui| {
        ui.add(egui::Label::new(egui::RichText::new(text).weak().small()).wrap());
        if let Some(reason) = unavailable {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("Unavailable: {reason}"))
                        .small()
                        .color(ui.visuals().warn_fg_color),
                )
                .wrap(),
            );
        }
    });
}
