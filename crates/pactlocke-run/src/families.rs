//! Evolution families (a species and everything it evolves from or into),
//! worked out from a game's evolution table. Not to be confused with a Game Family.

/// Which family each species belongs to, named by its lowest species number.
pub(crate) struct EvolutionFamilies {
    family: Vec<u16>,
}

/// With evolutions random every level nearly every species evolves, and the
/// families run together; only the exact species is compared then. In the
/// unmodified games fewer than half the species evolve.
const RANDOM_EVERY_LEVEL_SHARE: f64 = 0.8;

impl EvolutionFamilies {
    /// `evolutions` lists, for each species from 0, the species it can evolve into.
    pub fn from_evolutions(evolutions: &[Vec<u16>]) -> Self {
        let count = evolutions.len();
        let mut family: Vec<u16> = (0..count).map(species_number).collect();
        let evolving = evolutions.iter().skip(1).filter(|into| !into.is_empty()).count();
        if count > 1 && evolving as f64 >= (count - 1) as f64 * RANDOM_EVERY_LEVEL_SHARE {
            return Self { family };
        }
        for (species, into) in evolutions.iter().enumerate() {
            for &target in into.iter().filter(|&&t| usize::from(t) < count) {
                let a = root(&mut family, species_number(species));
                let b = root(&mut family, target);
                family[usize::from(a.max(b))] = a.min(b);
            }
        }
        for species in 0..count {
            family[species] = root(&mut family, species_number(species));
        }
        Self { family }
    }

    /// The family of `species`; a species the table doesn't cover is its own family.
    pub fn of(&self, species: u16) -> u16 {
        self.family.get(usize::from(species)).copied().unwrap_or(species)
    }
}

fn species_number(index: usize) -> u16 {
    u16::try_from(index).expect("species fit in u16")
}

/// The lowest species of the family `species` is joined to so far.
fn root(family: &mut [u16], species: u16) -> u16 {
    let mut at = species;
    while family[usize::from(at)] != at {
        at = family[usize::from(at)];
    }
    family[usize::from(species)] = at;
    at
}
