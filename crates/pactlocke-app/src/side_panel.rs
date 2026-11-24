//! The side panel during a Run. With other Players, the Player strip, the
//! next Sync Gate's ready check and any open vote stay fixed at the top, and
//! everything else is a tab, with the tab bar at the bottom: Feed, Party,
//! Links, Areas, Graveyard and Ruleset, which holds the Lobby too. Alone, there
//! is no strip, Links or Lobby. Before a Run with friends starts, the panel is
//! the Lobby alone.

use eframe::egui;
use pactlocke_game::Rule;
use std::collections::{BTreeMap, BTreeSet};

use pactlocke_run::{
    FeedEntry, FeedKind, Grave, Link, Member, PlayerKey, Preset, Ruleset, RunClock, RunCommand, RunEvent, RunView,
    SeenPokemon, SystemRunClock,
};

use crate::lobby_ui::Lobbies;
use crate::settings::AreaStatusStyle;
use crate::spectate::{DropoutAsk, Spectate};
use crate::theme::{self, Icons};
use crate::{player_strip, rules_text, run_panel};

/// The side panel's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Feed,
    Party,
    Links,
    Areas,
    Graveyard,
    /// The Ruleset and, with other Players, the Lobby under it.
    Ruleset,
}

/// What the panel asked for this frame.
#[derive(Default)]
pub struct Asks {
    pub watch: Option<usize>,
    pub vote: Option<RunEvent>,
    pub dropout: Option<DropoutAsk>,
    pub command: Option<RunCommand>,
}

/// A Player as the panel shows them: their key, name and colour's index.
struct Who {
    key: PlayerKey,
    name: String,
    index: usize,
    you: bool,
}

/// Everything the panel shows of a Run.
pub struct RunPanel<'a> {
    pub view: &'a RunView,
    /// The Run's Ruleset: the locked one with friends, this Run's own alone.
    pub ruleset: &'a Ruleset,
    /// With other Players: their Shadow Games.
    pub spectate: Option<&'a Spectate>,
    pub speed: u8,
    /// How the Area status shows: the Area card and the Strip are in this panel.
    pub area_status_style: crate::settings::AreaStatusStyle,
}

/// The panel during a Run. `tab` is the open tab, changed here.
pub fn show(ui: &mut egui::Ui, run: RunPanel<'_>, icons: &mut Icons, lobbies: &mut Lobbies, tab: &mut Tab) -> Asks {
    let mut asks = Asks::default();
    let now = SystemRunClock.now_ms();
    let players = players(&run, lobbies);
    let coop = run.spectate.is_some();
    let links: Vec<Link> = match (run.spectate, lobbies.log()) {
        (Some(spectate), Some(log)) if run.ruleset.links => spectate.links(log),
        _ => Vec::new(),
    };
    let areas = icons.profile().all_areas();
    let others = match lobbies.log() {
        Some(log) if coop => pactlocke_run::area_encounters(log),
        _ => Default::default(),
    };
    let graves = match lobbies.log() {
        Some(log) if coop => pactlocke_run::graves(log),
        _ => Default::default(),
    };
    let dead: usize = players.iter().map(|who| graves_of(&run, who, &graves).len()).sum();
    let tabs: Vec<(Tab, String)> = [
        (Tab::Feed, "Feed".to_owned()),
        (Tab::Party, "Party".to_owned()),
        (Tab::Links, format!("Links · {}", links.len())),
        (Tab::Areas, crate::areas_panel::label(&run.view.encounters, &areas)),
        (Tab::Graveyard, format!("Graveyard · {dead}")),
        (
            Tab::Ruleset,
            if coop { "Ruleset & Lobby" } else { "Ruleset" }.to_owned(),
        ),
    ]
    .into_iter()
    .filter(|(t, _)| match t {
        Tab::Links => coop && run.ruleset.links,
        _ => true,
    })
    .collect();
    if !tabs.iter().any(|(t, _)| t == tab) {
        *tab = Tab::Feed;
    }

    // Fixed at the top.
    if let (Some(spectate), Some(log)) = (run.spectate, lobbies.log()) {
        asks.watch = strip(ui, &run, spectate, lobbies, &players);
        if !lobbies.online() {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "You're offline: your game goes on and your Run is kept, and Sync Gate battles and votes wait until you're back.",
            );
        }
        if let Some(check) = log.ready_check() {
            egui::Frame::new()
                .fill(ui.visuals().faint_bg_color)
                .corner_radius(5.0)
                .inner_margin(6)
                .show(ui, |ui| {
                    ui.strong(format!(
                        "Next Sync Gate: {} · {} of {} ready",
                        check.gate,
                        check.ready(),
                        check.players.len()
                    ));
                    ui.horizontal_wrapped(|ui| {
                        for (name, ready) in &check.players {
                            let index = players.iter().find(|p| &p.name == name).map_or(0, |p| p.index);
                            theme::chip(ui, name, index);
                            if *ready {
                                ui.label("Ready");
                            } else {
                                ui.weak("Not yet");
                            }
                        }
                    });
                });
        }
        asks.vote = crate::spectate::votes(ui, log, spectate.me(), lobbies.online()).or(crate::spectate::flag_votes(
            ui,
            log,
            spectate.me(),
            lobbies.online(),
        ));
        asks.dropout = crate::spectate::dropouts(ui, log, spectate, lobbies.online());
    }
    if let Some(wanted) = run.view.nickname_wanted.first() {
        asks.command = run_panel::nickname_prompt(ui, wanted);
    }
    if run.area_status_style == AreaStatusStyle::Strip
        && let Some(status) = &run.view.area_status
    {
        crate::area_status::strip(ui, status);
    }
    ui.separator();

    // The tab bar at the bottom, the open tab above it.
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        // Rows laid out here, bottom row first: a wrapped row in a bottom-up
        // layout puts its second row below the panel, out of sight.
        for row in tab_rows(ui, &tabs).iter().rev() {
            ui.horizontal(|ui| {
                for (t, label) in *row {
                    if ui.selectable_label(tab == t, label.as_str()).clicked() {
                        *tab = *t;
                    }
                }
            });
        }
        ui.separator();
        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
            egui::ScrollArea::vertical()
                .id_salt(("tab", *tab as u8))
                .auto_shrink(false)
                .show(ui, |ui| match tab {
                    Tab::Feed => {
                        if run.area_status_style == AreaStatusStyle::Card
                            && let Some(status) = &run.view.area_status
                        {
                            crate::area_status::card(ui, status, icons);
                            ui.add_space(6.0);
                        }
                        sprite_bar(ui, &run, &players, &links, icons);
                        ui.add_space(6.0);
                        feed(ui, &run, &players, now);
                    }
                    Tab::Party => party(ui, &run, &players, &links, icons),
                    Tab::Links => {
                        if let Some(spectate) = run.spectate {
                            let roster: Vec<(PlayerKey, String, usize)> =
                                players.iter().map(|p| (p.key, p.name.clone(), p.index)).collect();
                            if let Some(skip) = crate::links_panel::tab(ui, &links, &roster, &spectate.me(), icons) {
                                asks.command = Some(skip);
                            }
                        }
                    }
                    Tab::Areas => {
                        let columns: Vec<crate::areas_panel::Column<'_>> = players
                            .iter()
                            .map(|who| crate::areas_panel::Column {
                                name: &who.name,
                                index: who.index,
                                you: who.you,
                                encounters: if who.you {
                                    &run.view.encounters
                                } else {
                                    others.get(&who.key).map_or(&[][..], Vec::as_slice)
                                },
                            })
                            .collect();
                        crate::areas_panel::tab(ui, &areas, &columns, icons);
                    }
                    Tab::Graveyard => graveyard(ui, &run, &players, &graves, icons, now),
                    Tab::Ruleset => {
                        if let Some(command) = ruleset_tab(ui, &run, lobbies) {
                            asks.command = Some(command);
                        }
                        if !coop {
                            return;
                        }
                        ui.add_space(6.0);
                        ui.separator();
                        ui.strong("Lobby");
                        lobbies.lobby_tab(ui, now);
                        if let (Some(spectate), Some(log)) = (run.spectate, lobbies.log()) {
                            let online = lobbies.online();
                            if let Some(ask) = crate::spectate::dropout_by_hand(ui, log, spectate, online) {
                                asks.dropout = Some(ask);
                            }
                            if let Some(vote) = crate::spectate::flags_to_clear(ui, log, spectate.me(), online) {
                                asks.vote = Some(vote);
                            }
                        }
                    }
                });
        });
    });
    asks
}

/// The tabs split into rows that each fit the panel's width, top row first.
fn tab_rows<'a>(ui: &egui::Ui, tabs: &'a [(Tab, String)]) -> Vec<&'a [(Tab, String)]> {
    let spacing = ui.spacing();
    let (gap, padding, room) = (spacing.item_spacing.x, spacing.button_padding.x, ui.available_width());
    let mut rows = Vec::new();
    let (mut start, mut used) = (0, 0.0);
    for (i, (_, label)) in tabs.iter().enumerate() {
        let text = egui::WidgetText::from(label.as_str()).into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Button,
        );
        let width = text.size().x + 2.0 * padding;
        if i > start && used + gap + width > room {
            rows.push(&tabs[start..i]);
            (start, used) = (i, width);
        } else {
            used += if i > start { gap + width } else { width };
        }
    }
    if start < tabs.len() {
        rows.push(&tabs[start..]);
    }
    rows
}

/// Before a Run with friends starts: the Lobby, beside the home screen.
pub fn lobby_only(ui: &mut egui::Ui, lobbies: &mut Lobbies) {
    let now = SystemRunClock.now_ms();
    ui.strong("Lobby");
    ui.weak("Everyone's game starts here after the lock's 3-2-1.");
    ui.separator();
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| lobbies.lobby_tab(ui, now));
}

/// Every Player in join order (the Host first): alone, just this Player.
fn players(run: &RunPanel<'_>, lobbies: &Lobbies) -> Vec<Who> {
    match (run.spectate, lobbies.log().and_then(|log| log.locked())) {
        (Some(spectate), Some(locked)) => locked
            .roster
            .iter()
            .enumerate()
            .map(|(index, p)| Who {
                key: p.key,
                name: p.name.clone(),
                index,
                you: p.key == spectate.me(),
            })
            .collect(),
        _ => vec![Who {
            key: lobbies.public_key().unwrap_or([0; 32]),
            name: lobbies.name.clone(),
            index: 0,
            you: true,
        }],
    }
}

/// A Player's party as this app last saw it: their own game, or their Shadow.
fn party_of(run: &RunPanel<'_>, who: &Who) -> Vec<SeenPokemon> {
    if who.you {
        return run.view.party.clone();
    }
    run.spectate
        .and_then(|s| s.shadows.iter().find(|shadow| shadow.key == who.key))
        .map(|shadow| shadow.status().seen.party)
        .unwrap_or_default()
}

/// The Pokémon of this Player's that are Dead, as the Run judges them: their
/// own Graveyard, or what their Shadow Game showed with Faint = Dead on, and
/// any whose Link-mate died.
fn dead_of(run: &RunPanel<'_>, who: &Who, links: &[Link]) -> BTreeSet<u32> {
    let mut dead: BTreeSet<u32> = if who.you {
        run.view.graveyard.iter().map(|g| g.pid).collect()
    } else if run.ruleset.faint_is_dead {
        run.spectate
            .and_then(|s| s.shadows.iter().find(|shadow| shadow.key == who.key))
            .map(|shadow| shadow.status().seen.fainted)
            .unwrap_or_default()
    } else {
        BTreeSet::new()
    };
    for link in links {
        if let Some(Member::Caught { pid, dead: true, .. }) = link.member(&who.key) {
            dead.insert(*pid);
        }
    }
    dead
}

/// The preset a Ruleset is, or "Custom".
pub fn preset_name(ruleset: &Ruleset) -> &'static str {
    let caps = ruleset.level_caps.as_deref().unwrap_or(&[]);
    // Links aren't compared: alone there are none, and the Run is still that preset.
    let rules = |r: &Ruleset| {
        [
            r.first_encounter,
            r.faint_is_dead,
            r.dupes_clause,
            r.gifts,
            r.statics,
            r.no_items,
            r.set_mode,
            r.must_nickname,
            r.type_clause,
            r.level_caps.is_some(),
        ]
    };
    Preset::ALL
        .into_iter()
        .find(|p| rules(&p.ruleset(caps)) == rules(ruleset))
        .map_or("Custom", Preset::name)
}

/// The Player strip: a box per Player, side by side, each with their name,
/// what they are doing, their Area, speed and connection, and Watch for the
/// others.
fn strip(
    ui: &mut egui::Ui,
    run: &RunPanel<'_>,
    spectate: &Spectate,
    lobbies: &Lobbies,
    players: &[Who],
) -> Option<usize> {
    let log = lobbies.log()?;
    let mut watch = None;
    // A line that is cut short with "…" where the box is too narrow, whole on hover.
    let line = |ui: &mut egui::Ui, text: egui::RichText| {
        let whole = text.text().to_owned();
        ui.add(egui::Label::new(text).truncate()).on_hover_text(whole);
    };
    ui.columns(players.len().max(1), |columns| {
        for (who, ui) in players.iter().zip(columns.iter_mut()) {
            let shadow = spectate.shadows.iter().position(|s| s.key == who.key);
            let status = shadow.map(|i| spectate.shadows[i].status());
            let (area, doing) = match &status {
                _ if who.you => (run.view.area.clone(), run.view.doing.to_string()),
                Some(status) if status.catching_up => (status.seen.area.clone(), "catching up…".to_owned()),
                Some(status) => (status.seen.area.clone(), status.seen.doing.to_string()),
                None => (None, String::new()),
            };
            let doing = match log.ready_gate_of(&who.key) {
                Some(gate) if !doing.starts_with("At ") => format!("Ready at {gate}"),
                _ => doing,
            };
            let speed = if who.you { run.speed } else { log.speed_of(&who.key) };
            let reach = if !who.you && spectate.shows_offline(&who.key) {
                Some(player_strip::Reach::Offline)
            } else {
                lobbies.reach(who.key)
            };
            let offline = reach == Some(player_strip::Reach::Offline);
            egui::Frame::new()
                .fill(ui.visuals().faint_bg_color)
                .corner_radius(5.0)
                .inner_margin(5)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.horizontal(|ui| {
                        theme::dot(ui, who.index);
                        let name = if who.you {
                            format!("{} (you)", who.name)
                        } else {
                            who.name.clone()
                        };
                        line(ui, egui::RichText::new(name).strong());
                    });
                    // What they are doing leads; where, and how they are reached, follow it.
                    let doing = egui::RichText::new(if doing.is_empty() { "—" } else { &doing });
                    line(
                        ui,
                        if offline {
                            egui::RichText::new("Offline").color(ui.visuals().warn_fg_color)
                        } else {
                            doing.color(ui.visuals().strong_text_color())
                        },
                    );
                    line(ui, egui::RichText::new(area.unwrap_or_else(|| "—".to_owned())));
                    let meta = [format!("{speed}x"), player_strip::reach_text(reach, who.you)];
                    let meta: Vec<&str> = meta.iter().map(String::as_str).filter(|m| !m.is_empty()).collect();
                    line(ui, egui::RichText::new(meta.join(" · ")).weak().small());
                    if let Some(i) = shadow {
                        ui.add_space(2.0);
                        match spectate.shadows[i].cant_watch() {
                            None if spectate.watching == Some(i) => {
                                ui.add_enabled(false, egui::Button::new("Watching").small());
                            }
                            None => {
                                if ui.add(egui::Button::new("Watch").small()).clicked() {
                                    watch = Some(i);
                                }
                            }
                            Some(why) => {
                                ui.add_enabled(false, egui::Button::new("Watch").small())
                                    .on_disabled_hover_text(why);
                            }
                        }
                    }
                });
        }
    });
    watch
}

/// A Player's Dead, newest first: their own Graveyard, or the Run log's
/// Deaths for another Player.
fn graves_of<'a>(run: &'a RunPanel<'_>, who: &Who, graves: &'a BTreeMap<PlayerKey, Vec<Grave>>) -> &'a [Grave] {
    if who.you {
        &run.view.graveyard
    } else {
        graves.get(&who.key).map_or(&[][..], Vec::as_slice)
    }
}

/// The Graveyard: every Player's Dead as party icons, newest first, each
/// one's name and what killed it on hover.
fn graveyard(
    ui: &mut egui::Ui,
    run: &RunPanel<'_>,
    players: &[Who],
    graves: &BTreeMap<PlayerKey, Vec<Grave>>,
    icons: &mut Icons,
    now: i64,
) {
    /// How big each Dead Pokémon's icon is.
    const ICON: f32 = 32.0;
    if players.iter().all(|who| graves_of(run, who, graves).is_empty()) {
        ui.weak("No one has died yet.");
        return;
    }
    for who in players {
        let dead = graves_of(run, who, graves);
        ui.horizontal(|ui| {
            theme::dot(ui, who.index);
            ui.strong(if who.you && players.len() > 1 { "You" } else { &who.name });
            ui.weak(dead.len().to_string());
        });
        if dead.is_empty() {
            ui.weak("No Deaths.");
        }
        ui.horizontal_wrapped(|ui| {
            for grave in dead {
                let name = if grave.name == grave.species_name {
                    grave.name.clone()
                } else {
                    format!("{} ({})", grave.name, grave.species_name)
                };
                // A Death from a log kept before Deaths were timed has no time.
                let when = if grave.at_ms > 0 {
                    format!(" · {}", run_panel::ago(now - grave.at_ms))
                } else {
                    String::new()
                };
                let tip = format!("{name}\nKilled by {}{when}", grave.killed_by);
                icons.show(ui, grave.species, ICON, true, &tip);
            }
        });
        ui.add_space(6.0);
    }
}

/// Every party's sprites in a row per Player, the Dead in red.
fn sprite_bar(ui: &mut egui::Ui, run: &RunPanel<'_>, players: &[Who], links: &[Link], icons: &mut Icons) {
    let parties: Vec<(Vec<SeenPokemon>, BTreeSet<u32>)> = players
        .iter()
        .map(|who| (party_of(run, who), dead_of(run, who, links)))
        .collect();
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(6.0)
        .inner_margin(6)
        .show(ui, |ui| {
            for (who, (party, dead_ones)) in players.iter().zip(&parties) {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [52.0, 24.0],
                        egui::Label::new(
                            egui::RichText::new(&who.name)
                                .color(theme::player_colour(who.index))
                                .strong(),
                        ),
                    );
                    for slot in 0..6 {
                        match party.get(slot) {
                            Some(pokemon) => {
                                let dead = dead_ones.contains(&pokemon.pid);
                                let tip = format!(
                                    "{} · Lv {}{}",
                                    shown_name(pokemon),
                                    pokemon.level,
                                    if dead { " · Dead" } else { "" }
                                );
                                icons.show(ui, pokemon.species, 24.0, dead, &tip);
                            }
                            None => theme::empty_slot(ui, 24.0),
                        }
                    }
                });
            }
        });
}

fn shown_name(pokemon: &SeenPokemon) -> &str {
    if pokemon.nickname.is_empty() {
        &pokemon.species_name
    } else {
        &pokemon.nickname
    }
}

/// The feed: every Player's lines, newest first, each with the Player's
/// name in their colour and an icon for its kind.
fn feed(ui: &mut egui::Ui, run: &RunPanel<'_>, players: &[Who], now: i64) {
    let me = players.iter().find(|p| p.you).map_or(0, |p| p.index);
    let mut lines: Vec<(usize, &FeedEntry)> = run
        .view
        .feed
        .iter()
        .map(|entry| {
            let index = entry
                .about
                .as_ref()
                .and_then(|name| players.iter().find(|p| &p.name == name))
                .map_or(me, |p| p.index);
            (index, entry)
        })
        .collect();
    if let Some(spectate) = run.spectate {
        for (author, entry) in spectate.others_feed() {
            if let Some(who) = players.iter().find(|p| p.key == *author) {
                lines.push((who.index, entry));
            }
        }
    }
    lines.sort_by_key(|(_, entry)| std::cmp::Reverse(entry.at_ms));
    if lines.is_empty() {
        ui.weak("Nothing yet: catches, Encounters, Deaths and more show here, newest first.");
    }
    for (index, entry) in lines {
        let name = players.get(index).map_or("", |p| p.name.as_str());
        let (colour, strong) = match entry.kind {
            FeedKind::Violation => (theme::DANGER, true),
            FeedKind::Cheat => (theme::CHEAT, true),
            FeedKind::Problem => (ui.visuals().warn_fg_color, false),
            _ => (ui.visuals().text_color(), false),
        };
        let icon_colour = if strong { colour } else { ui.visuals().weak_text_color() };
        ui.horizontal_wrapped(|ui| {
            ui.weak(egui::RichText::new(run_panel::ago(now - entry.at_ms)).small());
            if run.spectate.is_some() {
                theme::chip(ui, name, index);
            }
            theme::feed_icon(ui, entry.icon, icon_colour);
            let text = egui::RichText::new(&entry.text).color(colour);
            ui.label(if strong { text.strong() } else { text });
        });
    }
}

/// Every Player's party: icon, name, level and HP, the Dead in red, a
/// hovered Pokémon's Link-mates highlighted.
fn party(ui: &mut egui::Ui, run: &RunPanel<'_>, players: &[Who], links: &[Link], icons: &mut Icons) {
    let hover_id = egui::Id::unique("hovered link");
    let hovered: Option<String> = ui.data(|d| d.get_temp(hover_id));
    let mut now_hovered = None;
    for who in players {
        ui.horizontal(|ui| {
            theme::dot(ui, who.index);
            ui.strong(&who.name);
        });
        let party = party_of(run, who);
        let dead_ones = dead_of(run, who, links);
        if party.is_empty() {
            ui.weak("No Pokémon seen yet.");
        }
        for pokemon in &party {
            let link = links
                .iter()
                .find(|l| matches!(l.member(&who.key), Some(Member::Caught { pid, .. }) if *pid == pokemon.pid));
            let dead = dead_ones.contains(&pokemon.pid);
            let lit = link.is_some_and(|l| Some(&l.source.key) == hovered.as_ref());
            let fill = if dead {
                theme::DANGER.gamma_multiply(0.15)
            } else if lit {
                ui.visuals().selection.bg_fill.gamma_multiply(0.5)
            } else {
                egui::Color32::TRANSPARENT
            };
            let stroke = if dead {
                egui::Stroke::new(1.0, theme::DANGER)
            } else if lit {
                egui::Stroke::new(1.0, ui.visuals().selection.stroke.color)
            } else if link.is_some_and(Link::pending) {
                // The pending outline: not every Player has had this Encounter yet.
                egui::Stroke::new(1.0, ui.visuals().weak_text_color())
            } else {
                egui::Stroke::NONE
            };
            let row = egui::Frame::new()
                .fill(fill)
                .stroke(stroke)
                .corner_radius(5.0)
                .inner_margin(4)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        icons.show(ui, pokemon.species, 32.0, dead, shown_name(pokemon));
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                let name = egui::RichText::new(shown_name(pokemon));
                                ui.label(if dead { name.color(theme::DANGER) } else { name });
                                if dead {
                                    ui.colored_label(theme::DANGER, "Dead");
                                }
                                ui.weak(format!("Lv {}", pokemon.level));
                            });
                            let fraction = if pokemon.max_hp > 0 {
                                f32::from(pokemon.hp) / f32::from(pokemon.max_hp)
                            } else {
                                0.0
                            };
                            let bar = egui::ProgressBar::new(fraction)
                                .desired_height(4.0)
                                .desired_width(160.0);
                            ui.add(bar.fill(if fraction < 0.3 {
                                theme::DANGER
                            } else {
                                egui::Color32::from_rgb(0x57, 0xB9, 0x7A)
                            }));
                        });
                    });
                })
                .response;
            if let Some(link) = link {
                if row.hovered() {
                    now_hovered = Some(link.source.key.clone());
                }
                row.on_hover_text(format!("Linked from {}", link.source.name));
            }
        }
        ui.add_space(6.0);
    }
    ui.data_mut(|d| match now_hovered {
        Some(key) => {
            d.insert_temp(hover_id, key);
        }
        None => d.remove::<String>(hover_id),
    });
}

/// The Ruleset: every rule, the level cap, Retries (with the solo Retry),
/// Sync Gates and the Encounters used.
fn ruleset_tab(ui: &mut egui::Ui, run: &RunPanel<'_>, lobbies: &Lobbies) -> Option<RunCommand> {
    let mut command = None;
    let r = run.ruleset;
    // Each fact wraps to the panel's width, however narrow.
    let fact = |ui: &mut egui::Ui, name: &str, value: String| {
        ui.horizontal_wrapped(|ui| {
            ui.weak(name);
            ui.label(value);
        })
        .response
    };
    fact(ui, "Preset", preset_name(r).to_string());
    if let Some(locked) = lobbies
        .log()
        .and_then(|l| l.locked())
        .filter(|_| run.spectate.is_some())
    {
        fact(ui, "Locked", run_panel::ago(SystemRunClock.now_ms() - locked.start_ms));
    }
    let cap = match &run.view.level_cap {
        Some(cap) => {
            let soft = if cap.soft { " (soft)" } else { "" };
            match &cap.next {
                Some(next) => format!("{} until {next}{soft}", cap.level),
                None => "every cap passed".to_owned(),
            }
        }
        None => "off".to_owned(),
    };
    fact(ui, "Level cap", cap).on_hover_text(rules_text::detail(Rule::LevelCap, r));
    let retries = match run.view.retries_left {
        Some(left) => format!("{left} of {} left", r.retries),
        None if r.retries > 0 => format!("{} in the shared pool", r.retries),
        None => "off".to_owned(),
    };
    fact(ui, "Retries", retries).on_hover_text(rules_text::retries(r));
    if !r.sync_gates.is_empty() {
        fact(
            ui,
            "Sync Gates",
            format!("{}: {}", r.sync_gates.len(), r.sync_gates.join(", ")),
        )
        .on_hover_text(rules_text::detail(Rule::SyncGates, r));
    }
    for native in &run.view.native_options {
        let read = native.read.as_deref().unwrap_or("not a known setting");
        fact(
            ui,
            &native.rule,
            format!("the game's {} is {read} (the rule needs {})", native.name, native.value),
        )
        .on_hover_text(format!(
            "This game does {} itself, through its {} option ({}). Pactlocke set it for the Run.",
            native.rule, native.name, native.menu
        ));
    }
    if let Some(caps) = r.level_caps.as_ref().filter(|c| !c.is_empty()) {
        let caps: Vec<String> = caps.iter().map(|c| format!("{} {}", c.name, c.level)).collect();
        fact(ui, "Level caps", caps.join(" · "));
    }
    if run.spectate.is_none()
        && let Some(left) = run.view.retries_left
    {
        let retry = ui
            .add_enabled(
                left > 0 && run.view.run_checkpoint,
                egui::Button::new(format!("Retry · {left} left")),
            )
            .on_hover_text("Go back to your latest Run Checkpoint (your last in-game save), undoing everything since");
        if retry.clicked() {
            command = Some(RunCommand::Retry {
                to: pactlocke_run::RetryTo::Latest,
            });
        }
    }
    ui.add_space(6.0);
    ui.strong("Rules");
    ui.add(egui::Label::new(egui::RichText::new("Hover over a rule for the details.").weak().small()).wrap());
    let rules = [
        (Rule::FirstEncounter, r.first_encounter),
        (Rule::FaintIsDead, r.faint_is_dead),
        (Rule::DupesClause, r.dupes_clause),
        (Rule::Gifts, r.gifts),
        (Rule::Statics, r.statics),
        (Rule::LevelCap, r.level_caps.is_some()),
        (Rule::NoItems, r.no_items),
        (Rule::SetMode, r.set_mode),
        (Rule::MustNickname, r.must_nickname),
        (Rule::TypeClause, r.type_clause),
        (Rule::Links, r.links),
    ];
    for (rule, on) in rules {
        let fill = if on {
            theme::ON.gamma_multiply(0.16)
        } else {
            egui::Color32::TRANSPARENT
        };
        let row = egui::Frame::new()
            .fill(fill)
            .corner_radius(4.0)
            .inner_margin(egui::Margin::symmetric(6, 2))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    if on {
                        theme::tick(ui, theme::ON);
                        ui.strong(rules_text::name(rule));
                    } else {
                        ui.weak("–");
                        ui.weak(rules_text::name(rule));
                        ui.weak(egui::RichText::new("off").small());
                    }
                });
            });
        let help = if on {
            rules_text::detail(rule, r)
        } else {
            format!("Off in this Run. When on: {}", rules_text::summary(rule, r))
        };
        row.response.on_hover_text(help);
    }
    command
}
