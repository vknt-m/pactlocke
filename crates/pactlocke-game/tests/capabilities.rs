//! Seam 1: the capability report a ROM gets when it loads, from
//! its profile's patch sites checked against the ROM's bytes.

mod support;

use pactlocke_game::{Capability, ProfileStore, RomIdentity, Rule, sha1_hex};
use support::{BAG_SITE, CAP_SITE, CATCH_SITE, patchable_rom, profile_toml, put};

/// Loads `rom` against the test profile keyed by that ROM's own SHA-1.
fn capabilities_of(rom: &[u8], profile_body: impl FnOnce(String) -> String) -> pactlocke_game::CapabilityReport {
    let toml = profile_body(profile_toml("test-game", &sha1_hex(rom)));
    let store = ProfileStore::from_sources([("test.toml", toml)]).unwrap();
    match store.identify(rom) {
        RomIdentity::Profiled { capabilities, .. } => capabilities,
        other => panic!("the test profile should match, got {other:?}"),
    }
}

#[test]
fn a_clean_rom_can_enforce_every_rule_its_profile_has_a_method_for() {
    let report = capabilities_of(&patchable_rom(b"BPRE"), |t| t);

    for rule in [
        Rule::FirstEncounter,
        Rule::DupesClause,
        Rule::FaintIsDead,
        Rule::Links,
        Rule::LevelCap,
        Rule::NoItems,
        Rule::MustNickname,
        Rule::SetMode,
        Rule::TypeClause,
    ] {
        assert_eq!(report.get(rule), &Capability::Enforceable, "{rule:?}");
    }
}

#[test]
fn a_patch_site_holding_other_bytes_greys_its_rules_out_and_says_where() {
    let mut rom = patchable_rom(b"BPRE");
    put(&mut rom, CATCH_SITE, &[0xDE, 0xAD, 0xBE, 0xEF]);
    let report = capabilities_of(&rom, |t| t);

    let expected = Capability::GreyedOut {
        reason: "the ROM holds DE AD BE EF at 0x08000100 where the Game Profile expects 10 B5 00 24".into(),
    };
    assert_eq!(report.get(Rule::FirstEncounter), &expected);
    assert_eq!(report.get(Rule::DupesClause), &expected);
    assert_eq!(
        report.get(Rule::LevelCap),
        &Capability::Enforceable,
        "other rules are unaffected"
    );
}

#[test]
fn every_site_of_a_rule_is_checked() {
    let mut rom = patchable_rom(b"BPRE");
    put(&mut rom, CAP_SITE, &[0x64, 0x28]);
    put(&mut rom, BAG_SITE, &[0x02, 0x09, 0x00, 0x01]);
    let report = capabilities_of(&rom, |t| t);
    assert!(matches!(report.get(Rule::NoItems), Capability::GreyedOut { .. }));
}

#[test]
fn a_patch_site_past_the_end_of_the_rom_greys_its_rule_out() {
    let report = capabilities_of(&patchable_rom(b"BPRE"), |t| {
        t.replace("at = 0x08000200", "at = 0x08F00000")
    });
    assert_eq!(
        report.get(Rule::LevelCap),
        &Capability::GreyedOut {
            reason: "0x08F00000 is past the end of the ROM".into()
        }
    );
}

#[test]
fn a_rule_the_game_has_its_own_option_for_is_a_native_option() {
    let report = capabilities_of(&patchable_rom(b"BPRE"), |t| {
        t.replace(
            "[rules.set_mode]\nmethod = \"option_bit\"",
            "[rules.set_mode]\nmethod = \"native\"",
        )
    });
    assert_eq!(
        report.get(Rule::SetMode),
        &Capability::NativeOption {
            name: "Battle style".into(),
            menu: "OPTION > BATTLE STYLE".into(),
            value: "set".into()
        }
    );
}

#[test]
fn a_rule_the_profile_has_no_method_for_is_greyed_out() {
    let report = capabilities_of(&patchable_rom(b"BPRE"), |t| {
        // Drop the whole [rules.must_nickname] table.
        let start = t.find("[rules.must_nickname]").unwrap();
        let end = start + t[start..].find("\n[areas]").unwrap();
        format!("{}{}", &t[..start], &t[end..])
    });
    assert_eq!(
        report.get(Rule::MustNickname),
        &Capability::GreyedOut {
            reason: "this Game Profile has no way to enforce it".into()
        }
    );
}

#[test]
fn gifts_statics_and_sync_gates_wait_for_their_catalogues() {
    let report = capabilities_of(&patchable_rom(b"BPRE"), |t| t);
    for rule in [Rule::Gifts, Rule::Statics, Rule::SyncGates] {
        assert!(matches!(report.get(rule), Capability::GreyedOut { .. }), "{rule:?}");
    }
}

#[test]
fn must_nickname_is_greyed_out_when_the_nickname_command_isnt_in_the_command_table() {
    let mut rom = patchable_rom(b"BPRE");
    put(&mut rom, support::COMMAND_TABLE + 0xF0 * 4, &[0; 4]);
    let report = capabilities_of(&rom, |t| t);
    assert_eq!(
        report.get(Rule::MustNickname),
        &Capability::GreyedOut {
            reason: "the battle script command table at 0x08002000 has no command 0x0802DD39".into()
        }
    );
}
