//! How a Run's game is booted: from power-on, its last in-game save, the
//! suspend slot, or, after a crash, rebuilt from the last checkpoint plus the
//! button presses and stamped changes recorded after it.

use std::collections::BTreeMap;

use pactlocke_emu::{CoreAdapter, CoreError, Keys};
use pactlocke_game::Change;

use crate::event::ResumedFrom;
use crate::storage::StoredRun;
use crate::{RunEvent, Stamped, StoredCheckpoint};

/// How the game is to be booted, as the Run engine decided from the Run file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootPlan {
    /// A new game: power on with no save.
    PowerOn,
    /// The last in-game save, as a console would after being switched off.
    FromSave(Vec<u8>),
    /// Exactly where the Player suspended.
    Suspended(StoredCheckpoint),
    /// A Retry: back to this Run Checkpoint.
    Retry(StoredCheckpoint),
    /// After a crash: the checkpoint, then the keys held for each frame after
    /// it and the changes stamped in that stretch, in order.
    Rebuild {
        /// The checkpoint to start from.
        from: StoredCheckpoint,
        /// The keys held for each frame after it.
        keys: Vec<Keys>,
        /// The changes stamped in that stretch, in order.
        stamped: Vec<Stamped>,
    },
}

/// What booting left: the clock the boot runs on and the ROM patches active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Booted {
    /// The clock the boot runs on (Unix milliseconds).
    pub clock_ms: i64,
    /// Every ROM patch active, by address.
    pub patches: BTreeMap<u32, Vec<u8>>,
}

/// Boots `core`, which has the ROM loaded and nothing run, by `plan`. A
/// checkpoint's clock is set before it is restored, so the game reads the
/// same time as when it was taken; a fresh boot's clock starts at `now_ms`.
pub fn boot_core<C: CoreAdapter>(core: &mut C, plan: &BootPlan, now_ms: i64) -> Result<Booted, CoreError> {
    match plan {
        BootPlan::PowerOn | BootPlan::FromSave(_) => {
            if let BootPlan::FromSave(save) = plan {
                core.load_save_memory(save)?;
            }
            core.set_clock(now_ms);
            Ok(Booted {
                clock_ms: now_ms,
                patches: BTreeMap::new(),
            })
        }
        BootPlan::Suspended(from) => restore(core, from),
        BootPlan::Retry(from) => {
            // A new boot: the clock catches up with real time, never back.
            let mut booted = restore(core, from)?;
            let reads = core.clock_ms();
            if now_ms > reads {
                booted.clock_ms += now_ms - reads;
                core.set_clock(booted.clock_ms);
            }
            Ok(booted)
        }
        BootPlan::Rebuild { from, keys, stamped } => {
            let mut booted = restore(core, from)?;
            let mut stamped = stamped.iter().peekable();
            for (n, &held) in keys.iter().enumerate() {
                let boundary = from.frame + n as u64;
                while let Some(change) = stamped.next_if(|s| s.frame <= boundary) {
                    apply(core, &change.change, &mut booted.patches);
                }
                core.run_frame(held);
            }
            for change in stamped {
                apply(core, &change.change, &mut booted.patches);
            }
            Ok(booted)
        }
    }
}

/// The checkpoint's clock, then its save memory and savestate (the core
/// restores both, save memory first), then exactly its patch set.
fn restore<C: CoreAdapter>(core: &mut C, from: &StoredCheckpoint) -> Result<Booted, CoreError> {
    core.set_clock(from.clock_ms);
    core.restore(&from.core)?;
    let mut patches = BTreeMap::new();
    for (addr, bytes) in &from.patches {
        core.patch_rom(*addr, bytes);
        patches.insert(*addr, bytes.clone());
    }
    Ok(Booted {
        clock_ms: from.clock_ms,
        patches,
    })
}

fn apply<C: CoreAdapter>(core: &mut C, change: &Change, patches: &mut BTreeMap<u32, Vec<u8>>) {
    match change {
        Change::Write { addr, bytes } => core.write(*addr, bytes),
        Change::Patch { addr, bytes } => {
            core.patch_rom(*addr, bytes);
            patches.insert(*addr, bytes.clone());
        }
    }
}

/// How a resumed Run's game is booted, from the last boot whose game got
/// going (took its boot checkpoint; a boot that failed is passed over). One
/// that ended suspended resumes from the suspend slot; one that ended in a
/// crash is rebuilt; otherwise the game boots from its last in-game save.
///
/// A crash is rebuilt from the newest of the suspend slot and the last
/// checkpoint, so a Run with friends never boots from an older in-game save
/// while it has one: the others' Shadow Games would see a savestate load.
pub(crate) fn plan(stored: &StoredRun) -> (BootPlan, ResumedFrom) {
    let from_save = || match &stored.save_memory {
        Some(save) => (BootPlan::FromSave(save.clone()), ResumedFrom::Save),
        None => (BootPlan::PowerOn, ResumedFrom::Start),
    };
    let last_boot = stored
        .events
        .iter()
        .filter(|e| matches!(e.event, RunEvent::Checkpoint { .. }))
        .map(|e| e.boot)
        .max()
        .unwrap_or(0);
    let ended = stored
        .events
        .iter()
        .rev()
        .find(|e| e.boot == last_boot)
        .map(|e| &e.event);
    // After a crash: the newest of the suspend slot (a Run with friends
    // keeps it up to date, and so does its solo copy) and the last checkpoint.
    let newest = [&stored.suspended, &stored.latest_checkpoint]
        .into_iter()
        .flatten()
        .filter(|from| from.boot == last_boot)
        .max_by_key(|from| from.frame);
    match ended {
        Some(RunEvent::Suspended) => match &stored.suspended {
            Some(from) => (BootPlan::Suspended(from.clone()), ResumedFrom::Suspend),
            None => from_save(),
        },
        // A Retry or Rescue that never got its new boot going: as after a clean close.
        Some(RunEvent::Closed | RunEvent::Retried { .. } | RunEvent::Rescued { .. }) | None => from_save(),
        Some(_) => newest
            .and_then(|from| rebuild(from, stored, last_boot))
            .unwrap_or_else(from_save),
    }
}

/// A crashed boot rebuilt from its checkpoint: the keys held after it and
/// the changes stamped after it, if the input stream has no gap.
fn rebuild(from: &StoredCheckpoint, stored: &StoredRun, boot: u32) -> Option<(BootPlan, ResumedFrom)> {
    let mut keys = Vec::new();
    let mut stamped = Vec::new();
    for batch in stored.inputs.iter().filter(|b| b.boot == boot) {
        if batch.first_frame != keys.len() as u64 {
            return None;
        }
        keys.extend_from_slice(&batch.keys);
        stamped.extend_from_slice(&batch.stamped);
    }
    let keys = keys.get(usize::try_from(from.frame).ok()?..)?.to_vec();
    let stamped = stamped.get(from.stamped_before..)?.to_vec();
    let frames = keys.len() as u64;
    Some((
        BootPlan::Rebuild {
            from: from.clone(),
            keys,
            stamped,
        },
        ResumedFrom::Rebuild { frames },
    ))
}
