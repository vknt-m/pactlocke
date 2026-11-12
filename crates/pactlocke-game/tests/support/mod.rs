//! A small, complete Game Profile and ROMs built in memory, shared by the tests.
#![allow(dead_code)]

/// The SHA-1 of `b"PACTLOCKE TEST ROM"` (from `sha1sum`).
pub const TEST_ROM_SHA1: &str = "90b98ba6db543d3019bab92be9e9e9dda15441d9";

pub fn test_rom_bytes() -> Vec<u8> {
    b"PACTLOCKE TEST ROM".to_vec()
}

/// Where the test profile's patch sites sit in the test ROM.
pub const CATCH_SITE: u32 = 0x0800_0100;
pub const CAP_SITE: u32 = 0x0800_0200;
pub const BAG_SITE: u32 = 0x0800_0300;
pub const COMMAND_TABLE: u32 = 0x0800_2000;
pub const SPECIES_INFO: u32 = 0x0800_3000;
pub const SPECIES_NAMES: u32 = 0x0800_4000;
pub const EVOLUTION_TABLE: u32 = 0x0800_5000;

/// A test ROM of 64 KiB with the given header game code and every test patch
/// site holding its expected bytes. Its SHA-1 is whatever it is: tests use
/// these for capability checks, not lookup.
pub fn patchable_rom(game_code: &[u8; 4]) -> Vec<u8> {
    let mut rom = vec![0u8; 0x1_0000];
    rom[0xAC..0xB0].copy_from_slice(game_code);
    put(&mut rom, CATCH_SITE, &[0x10, 0xB5, 0x00, 0x24]);
    put(&mut rom, CAP_SITE, &[0x64, 0x28]);
    put(&mut rom, BAG_SITE, &[0x02, 0x09, 0x00, 0x00]);
    // The nickname command sits somewhere in the battle script command table.
    put(&mut rom, COMMAND_TABLE + 0xF0 * 4, &0x0802_DD39u32.to_le_bytes());
    rom
}

pub fn put(rom: &mut [u8], addr: u32, bytes: &[u8]) {
    let at = (addr - 0x0800_0000) as usize;
    rom[at..at + bytes.len()].copy_from_slice(bytes);
}

/// A complete profile laid out like FireRed, keyed by `sha1`.
pub fn profile_toml(id: &str, sha1: &str) -> String {
    format!(
        r#"
schema = 1
id = "{id}"
name = "Test Game"
game_code = "TEST"
family = "FRLG"
sha1 = "{sha1}"
status = "supported"
{BODY}
"#
    )
}

/// Everything below a profile's header: symbols, layout, options, rules, Areas.
pub const BODY: &str = r#"
[symbols]
party = 0x02024284
party_count = 0x02024029
enemy_party = 0x0202402C
pc_storage_pointer = 0x03005010
save_block_1_pointer = 0x03005008
save_block_2_pointer = 0x0300500C
region_map_section = 0x02036E10
main_callback2 = 0x030030F4
overworld_callback = 0x080565B4
in_battle = { at = 0x03003529, mask = 0x02 }
battle_type_flags = 0x02022B4C
battle_mons = 0x02023BE4
battler_party_indexes = 0x02023BCE
battler_statuses3 = 0x02023DFC
disable_structs = 0x02023E0C
script_context_status = 0x03000EA8
script_context = 0x03000EB0
species_info = 0x08003000
species_names = 0x08004000
evolution_table = 0x08005000
items = 0x08001000

[layout]
party_slot_size = 100
box_slot_size = 80
encrypted = true
ot_id = 0x04
nickname = 0x08
nickname_length = 10
flags = 0x13
egg_mask = 0x04
has_species_mask = 0x02
checksum = 0x1C
substructures = 0x20
level = 0x54
hp = 0x56
max_hp = 0x58
battle_mon_size = 0x58
battle_mon_hp = 0x28
battle_type_trainer = 0x08
battle_type_double = 0x01
battle_type_uncatchable = 0x18200
perish_song_status3 = 0x20
disable_struct_size = 0x1C
perish_timer = 0x0F
script_context_pointer = 0x08
script_context_stack_depth = 0x00
script_context_stack = 0x0C
species_info_size = 28
species_name_length = 11
species_count = 412
evolutions_per_species = 5
evolution_size = 8
evolution_target = 4
primary_type = 6
item_size = 44
item_count = 4
item_pocket = 26
item_battle_usage = 32
ball_pocket = 3
pc_boxes = 14
pc_box_slots = 30
pc_boxes_offset = 4
trainer_id = 0x0A
badges = 0xFE4
balls_pocket = 0x430
balls_pocket_slots = 13
dead_box = 14

[options.battle_style]
name = "Battle style"
menu = "OPTION > BATTLE STYLE"
save_block = 2
offset = 0x15
mask = 0x02
values = { shift = 0x00, set = 0x02 }

[rules.catch_block]
method = "patch"
sites = [{ at = 0x08000100, expect = "10 B5 00 24", write = "01 20 70 47" }]

[rules.level_cap]
method = "patch"
sites = [{ at = 0x08000200, expect = "64 28" }]

[rules.no_items_trainer]
method = "patch"
sites = [{ at = 0x08000300, expect = "02 09 00 00", write = "0A 09 00 00" }]

[rules.no_items_wild]
method = "zero_table_field"

[rules.set_mode]
method = "option_bit"
option = "battle_style"
value = "set"

[rules.must_nickname]
method = "key_filter"
battle_script_pointer = 0x02023D74
command_table = 0x08002000
command = 0x0802DD39
battle_communication = 0x02023E82

[areas]
0x58 = "Pallet Town"
0x65 = "Route 1"
0x81 = "Underground Path"
0x82 = "Underground Path"
0x8C = "Pokemon Tower"

[area_renames]
0x8C = "Pokémon Tower"

[[sources]]
key = "starter"
name = "Starter"
kind = "starter"
at = [0x08006000]

[[sources]]
key = "snorlax_route_12"
name = "Snorlax on Route 12"
kind = "static"
at = [0x08006100]

[[sources]]
key = "mt_moon_fossil"
name = "Mt. Moon fossil"
kind = "gift"
at = [0x08006200, 0x08006240]

[[sources]]
key = "togepi_egg"
name = "Togepi egg"
kind = "egg"
at = [0x08006300]
"#;

/// A GBA address space in memory: EWRAM, IWRAM and a ROM, for driving the
/// profile-driven game without an emulator.
pub struct MemoryImage {
    pub ewram: Vec<u8>,
    pub iwram: Vec<u8>,
    pub rom: Vec<u8>,
}

impl MemoryImage {
    pub fn new(rom: Vec<u8>) -> Self {
        Self {
            ewram: vec![0; 0x4_0000],
            iwram: vec![0; 0x8000],
            rom,
        }
    }

    fn slice(&mut self, addr: u32, len: usize) -> &mut [u8] {
        let (region, offset) = match addr >> 24 {
            0x02 => (&mut self.ewram, addr - 0x0200_0000),
            0x03 => (&mut self.iwram, addr - 0x0300_0000),
            0x08 | 0x09 => (&mut self.rom, addr - 0x0800_0000),
            _ => panic!("{addr:#010X} is outside the test image"),
        };
        &mut region[offset as usize..offset as usize + len]
    }

    pub fn put(&mut self, addr: u32, bytes: &[u8]) {
        self.slice(addr, bytes.len()).copy_from_slice(bytes);
    }

    pub fn put8(&mut self, addr: u32, value: u8) {
        self.put(addr, &[value]);
    }

    pub fn put16(&mut self, addr: u32, value: u16) {
        self.put(addr, &value.to_le_bytes());
    }

    pub fn put32(&mut self, addr: u32, value: u32) {
        self.put(addr, &value.to_le_bytes());
    }

    pub fn get(&mut self, addr: u32, len: usize) -> Vec<u8> {
        self.slice(addr, len).to_vec()
    }

    pub fn get32(&mut self, addr: u32) -> u32 {
        u32::from_le_bytes(self.get(addr, 4).try_into().unwrap())
    }

    /// Lands what an action returned, the way the Run engine will.
    pub fn apply(&mut self, changes: &[pactlocke_game::Change]) {
        for change in changes {
            match change {
                pactlocke_game::Change::Write { addr, bytes } | pactlocke_game::Change::Patch { addr, bytes } => {
                    self.put(*addr, bytes)
                }
            }
        }
    }
}

impl pactlocke_game::Memory for MemoryImage {
    fn read(&mut self, addr: u32, out: &mut [u8]) {
        out.copy_from_slice(self.slice(addr, out.len()));
    }
}

/// Where the test image keeps the save blocks and the PC (pointed at by the
/// profile's pointer symbols).
pub const SAVE_BLOCK_1: u32 = 0x0202_5000;
pub const SAVE_BLOCK_2: u32 = 0x0202_4F00;
pub const PC_STORAGE: u32 = 0x0202_9000;
pub const PARTY: u32 = 0x0202_4284;
pub const PARTY_COUNT: u32 = 0x0202_4029;

/// A Pokémon as the game stores it, before encryption.
pub struct TestMon {
    pub pid: u32,
    pub ot_id: u32,
    pub species: u16,
    /// Already in the game's character set.
    pub nickname: [u8; 10],
    pub met_section: u8,
    pub egg: bool,
    pub level: u8,
    pub hp: u16,
    pub max_hp: u16,
}

/// "BULBA" in the game's character set, padded with 0xFF.
pub const BULBA: [u8; 10] = [0xBC, 0xCF, 0xC6, 0xBC, 0xBB, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
/// "Rat 2" in the game's character set.
pub const RAT_2: [u8; 10] = [0xCC, 0xD5, 0xE8, 0x00, 0xA3, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];

impl TestMon {
    /// The 100-byte party form: pret's struct Pokemon, substructures
    /// shuffled by PID mod 24 and XORed with PID ^ OT ID.
    pub fn party_bytes(&self) -> [u8; 100] {
        const ORDERS: [&str; 24] = [
            "GAEM", "GAME", "GEAM", "GEMA", "GMAE", "GMEA", "AGEM", "AGME", "AEGM", "AEMG", "AMGE", "AMEG", "EGAM",
            "EGMA", "EAGM", "EAMG", "EMGA", "EMAG", "MGAE", "MGEA", "MAGE", "MAEG", "MEGA", "MEAG",
        ];
        let sub = |kind: char| -> [u8; 12] {
            let mut s = [0u8; 12];
            match kind {
                'G' => s[0..2].copy_from_slice(&self.species.to_le_bytes()),
                'M' => {
                    s[1] = self.met_section;
                }
                _ => {}
            }
            s
        };
        let mut plain = [0u8; 48];
        for (i, kind) in ORDERS[(self.pid % 24) as usize].chars().enumerate() {
            plain[i * 12..i * 12 + 12].copy_from_slice(&sub(kind));
        }
        let checksum = plain
            .chunks(2)
            .fold(0u16, |sum, w| sum.wrapping_add(u16::from_le_bytes([w[0], w[1]])));
        let key = self.pid ^ self.ot_id;
        let mut out = [0u8; 100];
        out[0..4].copy_from_slice(&self.pid.to_le_bytes());
        out[4..8].copy_from_slice(&self.ot_id.to_le_bytes());
        out[8..18].copy_from_slice(&self.nickname);
        out[0x13] = 0x02 | if self.egg { 0x04 } else { 0 };
        out[0x1C..0x1E].copy_from_slice(&checksum.to_le_bytes());
        for (i, word) in plain.chunks(4).enumerate() {
            let v = u32::from_le_bytes(word.try_into().unwrap()) ^ key;
            out[0x20 + i * 4..0x24 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        out[0x54] = self.level;
        out[0x56..0x58].copy_from_slice(&self.hp.to_le_bytes());
        out[0x58..0x5A].copy_from_slice(&self.max_hp.to_le_bytes());
        out
    }
}

/// A test image laid out like FireRed: pointers set, at the overworld, no
/// battle, no script.
pub fn overworld_image(rom: Vec<u8>) -> MemoryImage {
    let mut image = MemoryImage::new(rom);
    image.put32(0x0300_5008, SAVE_BLOCK_1);
    image.put32(0x0300_500C, SAVE_BLOCK_2);
    image.put32(0x0300_5010, PC_STORAGE);
    image.put32(0x0300_30F4, 0x0805_65B5); // CB2_Overworld, Thumb bit set
    image.put8(0x0300_0EA8, 2); // script context idle
    image
}

pub const ENEMY_PARTY: u32 = 0x0202_402C;

/// The opponent's party, as the game fills it in before a battle.
pub fn set_enemy_party(image: &mut MemoryImage, mons: &[TestMon]) {
    for (i, mon) in mons.iter().enumerate() {
        image.put(ENEMY_PARTY + i as u32 * 100, &mon.party_bytes());
    }
}

pub fn set_party(image: &mut MemoryImage, mons: &[TestMon]) {
    image.put8(PARTY_COUNT, mons.len() as u8);
    for (i, mon) in mons.iter().enumerate() {
        image.put(PARTY + i as u32 * 100, &mon.party_bytes());
    }
}

pub fn bulbasaur() -> TestMon {
    TestMon {
        pid: 0x1234_5678,
        ot_id: 0x0BAD_F00D,
        species: 1,
        nickname: BULBA,
        met_section: 0x58,
        egg: false,
        level: 5,
        hp: 19,
        max_hp: 21,
    }
}

pub fn rattata() -> TestMon {
    TestMon {
        pid: 0x8765_4329,
        ot_id: 0x0BAD_F00D,
        species: 19,
        nickname: RAT_2,
        met_section: 0x65,
        egg: false,
        level: 3,
        hp: 0,
        max_hp: 14,
    }
}
