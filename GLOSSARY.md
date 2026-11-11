# Pactlocke

A co-op challenge app where up to three Players play their own copies of the same game together, and the app enforces the Run's challenge rules automatically.

## Language

### Runs and Players

**Run**:
One co-op playthrough shared by up to three Players, from the moment the Ruleset is locked until it is won or abandoned.
_Avoid_: Session, game, playthrough

**Player**:
A person taking part in a Run, playing their own copy of the game.
_Avoid_: User, partner (when meaning a specific person)

**Ruleset**:
The challenge rules chosen when a Run starts, every one of them optional, Links included. It is locked for the rest of the Run.
_Avoid_: Settings, config, options

**Game Family**:
A group of games that share an engine and data layout, so they are supported together: RSE (Ruby, Sapphire, Emerald), FRLG (FireRed, LeafGreen), DPPt, HGSS and BW/B2W2.
_Avoid_: Generation (one generation can contain several Game Families)

### Encounters and Links

**Area**:
A place in the game that counts as one encounter location for the Run's first-encounter rule, such as a route, cave or town. It is the game's own region-map section, the place name the game records as a Pokémon's met location, so every floor of a cave and every zone of the Safari Zone is one Area. Grass, surfing, fishing and rock smash in an Area all share its one Encounter.
_Avoid_: Route (too narrow), map (one Area can span many maps), location

**Encounter**:
A Player's one chance at a Pokémon from a given source, whether or not it is caught. There are three kinds of source: an Area (the first wild Pokémon the Player battles there, not merely entering it), a Gift, or a Static.
_Avoid_: Catch (a catch is only one possible outcome of an Encounter)

**Gift**:
A Pokémon the game hands a Player rather than one met in battle, such as the starter, a fossil or an egg. Where the game offers a choice, each Player's pick is still the same Gift. The starter is always allowed; other Gifts only if the Ruleset allows them.
_Avoid_: Freebie

**Static**:
A scripted, one-off battle with a specific Pokémon, such as a legendary, Kecleon or Snorlax. Depending on the Ruleset it is either its own bonus Encounter or simply counts as the Area's Encounter if it comes first there.
_Avoid_: Special encounter, legendary (too narrow)

**Link**:
The set of each Player's Encounter from the same source: the same Area, the same Gift, or the same Static. If any member faints, every member of the Link dies. If any Player misses, skips or loses their Encounter from a source, that source is closed to everyone.
_Avoid_: Pair (Links can have three members), soul link (that is the name of the challenge)

### Playing together

**Sync Gate**:
A battle chosen in the Ruleset (gym leaders, Elite Four, Champion) that no Player can start until every Player has readied up at it. A Player may leave and come back to ready up when the others arrive.
_Avoid_: Sync battle, boss lock

**Spectate**:
Watching another Player's game screen live, view-only, at any point in the Run.
_Avoid_: Stream, watch party

**Dropout**:
A Player leaving a Run for good by vote of the remaining Players, not through Deaths. The leaver's Pokémon are removed from every Link, and the Run continues with the remaining Players.
_Avoid_: Disconnect (that is temporary), quit, kick

**Partner Avatar**:
Another Player's character, shown walking around inside your own game.
_Avoid_: Ghost, other player

**Follower**:
The lead Pokémon in a Player's party, shown walking behind that Player's character in their own game and on their Partner Avatar in others' games.
_Avoid_: Walking Pokémon, pet

**Dead**:
The permanent state of a Pokémon that fainted, or whose Link-mate fainted. It can never be used again in the Run, unless a Retry rewinds the Run to before it happened.
_Avoid_: Fainted (fainting is temporary in the game itself), released

**Violation**:
A Ruleset rule broken in a way the app could not stop. It is shown to every Player, flagged in red in the live feed.
_Avoid_: Strike, cheat (cheating is tampering with the game, not breaking a rule)

**Host**:
The Player whose app puts the Run's events in order. Hosting passes to the next Player who joined if the Host leaves. The Host cannot change or invent other Players' events.
_Avoid_: Server, admin, owner

**Shadow Game**:
A copy of another Player's game that your app runs in the background from their button presses alone, so it matches theirs exactly. Spectate shows it.
_Avoid_: Mirror, replay (a replay is after the fact), ghost

**Run Checkpoint**:
A moment every Player's game is captured together, taken whenever any Player saves in-game and at every Sync Gate. It is what a Retry returns to.
_Avoid_: Save (a save belongs to one Player), savestate (that is the emulator mechanism)

**Retry**:
An optional Ruleset rule: a small shared number of times the group may vote to send every Player back to the latest Run Checkpoint, undoing everything since.
_Avoid_: Reset (one Player alone), rewind, reload

### Games and hacks

**Romhack**:
A patched, fan-made game built on one of the supported Game Families. A Run can be played on a Romhack in place of the original game.
_Avoid_: Mod, fan game, hack (alone)

**Game Profile**:
Everything Pactlocke needs to know about one exact game, an original or a Romhack, so a Run can be played on it, such as its places, Gifts, Statics and level caps. A Romhack without one is either Untested or not supported.
_Avoid_: Hack Profile (original games have one too), mod config, hack pack

**Shared Seed**:
An optional Ruleset rule that gives every Player the same randomized game. Without it, each Player's randomized game is different.
_Avoid_: Same seed, synced randomizer
