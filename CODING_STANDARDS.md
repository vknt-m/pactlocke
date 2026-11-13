# Coding standards

For the reviewer: judgement calls only. Formatting, clippy and missing docs (in the crates with `#![warn(missing_docs)]`) are checked by `bash scripts/check.sh` and CI; leave them to those.

## Domain words

- Code, docs, feed lines and UI text use `GLOSSARY.md`'s terms, and none of its _Avoid_ words in the sense the glossary rules out: Player (a person), Run, Shadow Game, Partner Avatar, Follower, Retry, Run Checkpoint, Sync Gate, Dropout, Host, Violation, cheat flag.
- A type named after a domain concept keeps that name everywhere it travels: a type, its fields, its feed text.

## Types

- A Player's key is `PlayerKey`, not `[u8; 32]`, wherever the type alias is in reach.
- A moment in a Player's game is a `GamePoint` (boot and frame), not a `boot` and a `frame` passed side by side.
- Fields that always travel together become one small type.

## One source of truth

- A rule worked out in two places (who may vote, what counts once per Player, which Players are still in the Run) lives in one function the others call. For the Run log, that is usually a method on `RunLog`.
- Constants a UI string depends on are formatted in from the constant, never written into the string.

## Text Players read

- Feed lines and UI strings are plain English sentences, written to the Player ("you"), naming other Players by name.
- An unclear cause reads as what was seen, not as an accusation ("game out of sync", not "cheated").

## Docs

- A doc comment says what an item is or holds, and why when that isn't obvious, in one line where one will do.
- A doc comment that no longer matches its item, such as a field it lists that's gone or a default that changed, is a defect, the same as a wrong line of code.
