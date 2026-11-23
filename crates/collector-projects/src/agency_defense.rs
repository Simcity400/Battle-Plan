use engine_native::{Memory, ccountry};
use engine_native_politics::modifier_breakdown;
use engine_raid_inputs::raid_intel_inputs::Row;
use std::sync::{Arc, Mutex, TryLockError};

pub type Tree = Arc<Vec<modifier_breakdown::Node>>;

struct Held {
    country: u32,
    pointer: usize,
    certificate: Option<(u64, u64)>,
    value: i64,
    tree: Option<Tree>,
}

static TREES: Mutex<Vec<Option<Held>>> = Mutex::new(Vec::new());

pub fn snapshot(
    memory: &impl Memory,
    base: usize,
    game: usize,
    rows: &[Row],
    output: &mut Vec<Option<Tree>>,
) {
    output.clear();
    if engine_allocation::reserve(output, rows.len()).is_err() {
        return;
    }
    output.resize_with(rows.len(), || None);
    let Ok(countries) = ccountry::Reader::new(memory, base, game) else {
        return;
    };
    let mut held = match TREES.try_lock() {
        Ok(held) => held,
        Err(TryLockError::Poisoned(poisoned)) => {
            let mut held = poisoned.into_inner();
            held.clear();
            held
        }
        Err(TryLockError::WouldBlock) => return,
    };
    let additional = (countries.count() as usize).saturating_sub(held.len());
    if engine_allocation::reserve(&mut held, additional).is_err() {
        return;
    }
    held.resize_with(countries.count() as usize, || None);
    for (index, row) in rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.strategic_present)
    {
        let Ok(country) = countries.resolve(row.country) else {
            continue;
        };
        let certificate = crate::naval_tree_changes::sequence(country.pointer);
        let slot = &mut held[country.canonical_slot as usize];
        if let Some(previous) = slot
            && certificate.is_some()
            && previous.tree.is_some()
            && previous.country == country.id
            && previous.pointer == country.pointer
            && previous.certificate == certificate
            && previous.value == row.country_defense
        {
            output[index] = previous.tree.clone();
            continue;
        }
        let tree = country
            .pointer
            .checked_add(ccountry::MODIFIER_ACCUMULATOR)
            .and_then(|root| {
                modifier_breakdown::printed_nodes(
                    memory,
                    base,
                    root,
                    engine_raid_inputs::raid_intel_inputs::KEYS
                        [engine_raid_inputs::raid_intel_inputs::DEFENSE],
                )
                .ok()
            })
            .filter(|tree| {
                tree.first()
                    .is_some_and(|node| node.value == row.country_defense)
            })
            .and_then(|tree| replay_data::rows::printed_tree(&tree).ok())
            .filter(|_| crate::naval_tree_changes::sequence(country.pointer) == certificate);
        let tree = match (slot.as_ref().and_then(|held| held.tree.as_ref()), tree) {
            (Some(held), Some(tree)) if **held == tree => Some(Arc::clone(held)),
            (_, tree) => tree.map(Arc::new),
        };
        output[index] = tree.clone();
        *slot = Some(Held {
            country: country.id,
            pointer: country.pointer,
            certificate,
            value: row.country_defense,
            tree,
        });
    }
}
