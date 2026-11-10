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
A place in the game that counts as one encounter location for the Run's first-encounter rule, such as a route, cave or town.
_Avoid_: Route (too narrow), map, location

**Encounter**:
The first wild Pokémon a Player meets in an Area, whether or not it is caught.
_Avoid_: Catch (a catch is only one possible outcome of an Encounter)

**Link**:
The set of each Player's Encounter from the same Area. If any member faints, every member of the Link dies. If any Player misses or loses their Encounter in an Area, that Area is closed to everyone.
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
The permanent state of a Pokémon that fainted, or whose Link-mate fainted. It can never be used again in the Run.
_Avoid_: Fainted (fainting is temporary in the game itself), released

### Games and hacks

**Romhack**:
A patched, fan-made game built on one of the supported Game Families. A Run can be played on a Romhack in place of the original game.
_Avoid_: Mod, fan game, hack (alone)

**Shared Seed**:
An optional Ruleset rule that gives every Player the same randomized game. Without it, each Player's randomized game is different.
_Avoid_: Same seed, synced randomizer
