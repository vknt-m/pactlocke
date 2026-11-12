//! Seam 2: the in-memory fake game the Run engine tests will
//! drive. Like the real one, its actions only return changes; applying them
//! is what changes the game.

use pactlocke_game::{
    Area, Change, FakeGame, Game, GameError, PartyStats, Pokemon, ScriptContext, ScriptStatus, StoredPokemon,
};

fn pokemon(pid: u32, species: u16) -> Pokemon {
    Pokemon {
        pid,
        ot_id: 1,
        species,
        nickname: "MON".into(),
        met_section: 0x65,
        is_egg: false,
        stats: Some(PartyStats {
            level: 5,
            hp: 20,
            max_hp: 20,
        }),
    }
}

fn with_party() -> FakeGame {
    FakeGame {
        party: vec![pokemon(1, 1), pokemon(2, 19)],
        ..FakeGame::default()
    }
}

#[test]
fn the_fake_reads_back_what_the_test_set() {
    let mut game = FakeGame {
        area: Some(Area {
            id: 0x65,
            name: "Route 1".into(),
        }),
        script: ScriptContext {
            status: ScriptStatus::Waiting,
            position: Some(0x0816_0000),
        },
        trainer_id: 7,
        ..with_party()
    };
    assert_eq!(game.area().unwrap().name, "Route 1");
    assert_eq!(game.party().len(), 2);
    assert_eq!(game.script().status, ScriptStatus::Waiting);
    assert_eq!(game.trainer_id(), 7);
    assert!(game.at_safe_overworld_point(), "the fake starts on the overworld");
}

#[test]
fn an_action_changes_nothing_until_its_changes_are_applied() {
    let mut game = with_party();
    let changes = game.move_to_dead_box(1).unwrap();
    assert_eq!(game.party().len(), 2);

    game.apply(&changes);
    assert_eq!(game.party(), vec![pokemon(1, 1)]);
    assert_eq!(
        game.pc(),
        vec![StoredPokemon {
            box_number: 14,
            slot: 1,
            pokemon: Pokemon {
                stats: None,
                ..pokemon(2, 19)
            }
        }]
    );
}

#[test]
fn patches_toggle_the_fakes_rule_state() {
    let mut game = with_party();
    let changes = [
        game.block_catches(true).unwrap(),
        game.set_level_cap(12).unwrap(),
        game.ban_items_in_wild_battles(true).unwrap(),
        game.lock_bag_in_trainer_battles(true).unwrap(),
    ]
    .concat();
    assert!(changes.iter().all(|c| matches!(c, Change::Patch { .. })));
    game.apply(&changes);
    assert!(game.catches_blocked && game.wild_items_banned && game.bag_locked);
    assert_eq!(game.level_cap, Some(12));

    let off = game.block_catches(false).unwrap();
    game.apply(&off);
    assert!(!game.catches_blocked);
}

#[test]
fn faints_nicknames_and_options_land_when_applied() {
    let mut game = with_party();
    let changes = [
        game.faint_benched(0).unwrap(),
        game.write_nickname(1, "Nibbles").unwrap(),
        game.set_option("battle_style", "set").unwrap(),
    ]
    .concat();
    game.apply(&changes);
    assert_eq!(game.party()[0].stats.unwrap().hp, 0);
    assert_eq!(game.party()[1].nickname, "Nibbles");
    assert_eq!(game.option("battle_style").as_deref(), Some("set"));
}

#[test]
fn the_fake_refuses_what_the_real_game_refuses() {
    let mut game = with_party();
    assert_eq!(game.faint_benched(5), Err(GameError::NoSuchSlot(5)));
    assert_eq!(game.faint_active_battler(0), Err(GameError::NotInBattle));
    assert_eq!(game.set_level_cap(0), Err(GameError::BadLevel(0)));
    assert!(matches!(game.write_nickname(0, ""), Err(GameError::BadName(_))));

    let full_box: Vec<_> = (1..=30)
        .map(|slot| StoredPokemon {
            box_number: 2,
            slot,
            pokemon: pokemon(100 + u32::from(slot), 16),
        })
        .collect();
    game.pc = full_box;
    assert_eq!(game.move_to_box(0, 2), Err(GameError::BoxFull(2)));
}

#[test]
fn the_fake_refuses_a_setting_its_option_doesnt_have() {
    let mut game = with_party();
    assert_eq!(
        game.set_option("battle_style", "fast"),
        Err(GameError::UnknownOption("battle_style = fast".into()))
    );
}
