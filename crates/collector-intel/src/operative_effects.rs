use collector_observers::{
    naval_tree_changes,
    unit_leader_changes::{self, Changes},
};
use engine_native::{
    Memory, ccountry, modifier_accumulator, modifier_breakdown, operative_missions, read,
};
use replay_data::rows::{OperativeEffect, OperativeEffectModifier};
use std::{
    collections::HashMap,
    mem::size_of,
    sync::{
        Arc, Mutex, OnceLock, TryLockError,
        atomic::{AtomicUsize, Ordering},
    },
};

pub type Effects = [Option<Arc<OperativeEffect>>; 2];
type Certificate = [(u64, u64); 2];

struct Held {
    generation: u64,
    certificate: Option<Certificate>,
    header: [u8; 16],
    value: i64,
    tree: Option<Arc<Vec<modifier_breakdown::Node>>>,
}

struct HeldEffect {
    generation: u64,
    value: Arc<OperativeEffect>,
}

#[derive(Default)]
struct Cache {
    generation: u64,
    roots: HashMap<(usize, u32), Held>,
    effects: HashMap<(u64, bool), HeldEffect>,
    trade: HashMap<(u32, u32), Option<i64>>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);
static BYTES: AtomicUsize = AtomicUsize::new(0);

fn empty_tree() -> Arc<Vec<modifier_breakdown::Node>> {
    static EMPTY: OnceLock<Arc<Vec<modifier_breakdown::Node>>> = OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| Arc::new(Vec::new())))
}

impl Cache {
    fn tree(
        &mut self,
        memory: &impl Memory,
        base: usize,
        root: usize,
        key: u32,
        certificate: Option<Certificate>,
        value: i64,
    ) -> Option<Arc<Vec<modifier_breakdown::Node>>> {
        if root == 0 {
            return Some(empty_tree());
        }
        let header = read::<[u8; 16]>(memory, root)?;
        if let Some(held) = self.roots.get_mut(&(root, key))
            && certificate.is_some()
            && held.tree.is_some()
            && held.certificate == certificate
            && held.header == header
            && held.value == value
        {
            held.generation = self.generation;
            return held.tree.clone();
        }
        let mut tree = modifier_breakdown::printed_nodes(memory, base, root, key)
            .ok()
            .filter(|tree| tree.first().is_some_and(|node| node.value == value))
            .map(Arc::new);
        if let (Some(previous), Some(current)) = (
            self.roots
                .get(&(root, key))
                .and_then(|held| held.tree.as_ref()),
            tree.as_ref(),
        ) && **previous == **current
        {
            tree = Some(Arc::clone(previous));
        }
        self.roots.insert(
            (root, key),
            Held {
                generation: self.generation,
                certificate,
                header,
                value,
                tree: tree.clone(),
            },
        );
        tree
    }

    fn effect<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        game: usize,
        countries: &ccountry::Reader<'_, M>,
        row: &operative_missions::Row,
        pending: bool,
    ) -> Option<Arc<OperativeEffect>> {
        let roots = operative_missions::effect_roots(memory, base, game, countries, row, pending)
            .ok()??;
        let owner = countries.resolve(row.owner_country).ok()?;
        let sequences = || {
            (
                naval_tree_changes::sequence(owner.pointer),
                unit_leader_changes::Observer.sequence(row.native_pointer),
                naval_tree_changes::sequence(row.native_pointer),
            )
        };
        let before = sequences();
        let certificates = [
            before.0.map(|c| [c, c]),
            before.1.zip(before.2).map(|(a, b)| [a, b]),
            before.1.map(|(epoch, _)| [(epoch, 0); 2]),
        ];
        let mut modifiers: [OperativeEffectModifier; 2] =
            std::array::from_fn(|_| Default::default());
        for (&key, modifier) in roots.keys[..roots.key_count].iter().zip(&mut modifiers) {
            let mut value = 0i64;
            let mut sources = std::array::from_fn(|_| empty_tree());
            let mut complete = true;
            for (index, (root, certificate)) in
                roots.roots.into_iter().zip(certificates).enumerate()
            {
                let term = if root == 0 {
                    0
                } else {
                    modifier_accumulator::lookup_effective_at(memory, base, root, key)
                        .ok()?
                        .effective
                };
                value = value.wrapping_add(term);
                match self.tree(memory, base, root, key, certificate, term) {
                    Some(tree) => sources[index] = tree,
                    None => complete = false,
                }
            }
            *modifier = OperativeEffectModifier {
                key,
                value,
                sources: complete.then_some(sources),
            };
        }
        if before != sequences() {
            return None;
        }
        let (variant, target) = if pending {
            (row.pending_variant, row.pending_country)
        } else {
            (row.variant, row.target_country)
        };
        let trade_opinion = if variant == Some(engine_native::operative_action::CONTROL_TRADE) {
            let pair = (target?, row.owner_country);
            if let Some(value) = self.trade.get(&pair) {
                *value
            } else {
                let value = engine_intelligence::strategic_operatives::trade_influence(
                    memory, base, game, countries, pair.0, pair.1,
                )
                .ok()
                .and_then(|influence| {
                    engine_resources::trade_opinion::read_current(
                        memory, base, game, countries, pair.0, pair.1, influence,
                    )
                    .ok()
                });
                self.trade.insert(pair, value);
                value
            }
        } else {
            None
        };
        let key = (row.id, pending);
        let modifiers = &modifiers[..roots.key_count];
        if let Some(held) = self.effects.get_mut(&key)
            && held.value.owned_states == roots.owned_states
            && held.value.network_gain_scale == roots.network_gain_scale
            && held.value.trade_opinion == trade_opinion
            && held.value.modifiers.len() == modifiers.len()
            && held.value.modifiers.iter().zip(modifiers).all(|(a, b)| {
                a.key == b.key
                    && a.value == b.value
                    && match (&a.sources, &b.sources) {
                        (None, None) => true,
                        (Some(a), Some(b)) => a.iter().zip(b).all(|(a, b)| Arc::ptr_eq(a, b)),
                        _ => false,
                    }
            })
        {
            held.generation = self.generation;
            return Some(Arc::clone(&held.value));
        }
        let mut owned_modifiers = engine_allocation::vec(modifiers.len()).ok()?;
        owned_modifiers.extend_from_slice(modifiers);
        let value = Arc::new(OperativeEffect {
            owned_states: roots.owned_states,
            network_gain_scale: roots.network_gain_scale,
            trade_opinion,
            modifiers: owned_modifiers,
        });
        self.effects.insert(
            key,
            HeldEffect {
                generation: self.generation,
                value: Arc::clone(&value),
            },
        );
        Some(value)
    }

    fn allocated_bytes(&self) -> usize {
        self.roots.capacity() * size_of::<((usize, u32), Held)>()
            + self.effects.capacity() * size_of::<((u64, bool), HeldEffect)>()
            + self.trade.capacity() * size_of::<((u32, u32), Option<i64>)>()
            + self
                .effects
                .values()
                .map(|effect| {
                    size_of::<OperativeEffect>()
                        + effect.value.modifiers.capacity() * size_of::<OperativeEffectModifier>()
                })
                .sum::<usize>()
            + self
                .roots
                .values()
                .filter_map(|held| held.tree.as_ref())
                .map(|tree| {
                    size_of::<Vec<modifier_breakdown::Node>>()
                        + tree.capacity() * size_of::<modifier_breakdown::Node>()
                        + tree.iter().map(|node| node.name.capacity()).sum::<usize>()
                })
                .sum::<usize>()
    }
}

pub fn allocated_bytes() -> usize {
    BYTES.load(Ordering::Relaxed)
}

pub fn snapshot(
    memory: &impl Memory,
    base: usize,
    game: usize,
    rows: &[operative_missions::Row],
    output: &mut Vec<Effects>,
) {
    output.clear();
    if engine_allocation::reserve(output, rows.len()).is_err() {
        return;
    }
    output.resize_with(rows.len(), Default::default);
    naval_tree_changes::operative_effects(rows.len().saturating_mul(2));
    unit_leader_changes::operative_effects(rows.len());
    let Ok(countries) = ccountry::Reader::new(memory, base, game) else {
        return;
    };
    let mut guard = match CACHE.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => {
            let mut guard = poisoned.into_inner();
            *guard = None;
            guard
        }
        Err(TryLockError::WouldBlock) => return,
    };
    let cache = guard.get_or_insert_with(Cache::default);
    cache.generation = cache.generation.wrapping_add(1);
    cache.trade.clear();
    let required = rows.len().saturating_mul(12);
    let additional = required.saturating_sub(cache.roots.len());
    let effects = rows
        .len()
        .saturating_mul(2)
        .saturating_sub(cache.effects.len());
    if engine_allocation::reserve_map(&mut cache.roots, additional).is_err()
        || engine_allocation::reserve_map(&mut cache.effects, effects).is_err()
        || engine_allocation::reserve_map(&mut cache.trade, rows.len().saturating_mul(2)).is_err()
    {
        return;
    }
    for (row, effects) in rows.iter().zip(output) {
        *effects =
            [false, true].map(|pending| cache.effect(memory, base, game, &countries, row, pending));
    }
    cache
        .roots
        .retain(|_, held| held.generation == cache.generation);
    cache
        .effects
        .retain(|_, held| held.generation == cache.generation);
    BYTES.store(cache.allocated_bytes(), Ordering::Relaxed);
}
