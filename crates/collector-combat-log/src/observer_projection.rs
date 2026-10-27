//! Stable occurrence labels from the encoder's complete baseline, not values.
//! No native reads or process-global identity cache are involved.
use replay_data::rows::CombatLogObserverRow;

type Occurrence = (u64, bool, u8, u64, u64);

/// Sort scratch its owner keeps across hours, so a steady hour allocates only its rows.
#[derive(Default)]
pub struct Scratch {
    before: Vec<(Occurrence, u64)>,
    order: Vec<(Occurrence, usize)>,
    placed: Vec<(u128, usize, usize)>,
}

impl Scratch {
    pub fn allocated_bytes(&self) -> usize {
        self.before.capacity() * size_of::<(Occurrence, u64)>()
            + self.order.capacity() * size_of::<(Occurrence, usize)>()
            + self.placed.capacity() * size_of::<(u128, usize, usize)>()
    }
}

pub fn project(
    current: &[CombatLogObserverRow],
    previous: &[CombatLogObserverRow],
) -> Result<Vec<CombatLogObserverRow>, &'static str> {
    project_with(current, previous, &mut Scratch::default())
}

/// Sorts move small (occurrence, label) and (occurrence, index) pairs, never whole rows.
pub fn project_with(
    current: &[CombatLogObserverRow],
    previous: &[CombatLogObserverRow],
    scratch: &mut Scratch,
) -> Result<Vec<CombatLogObserverRow>, &'static str> {
    let Scratch {
        before,
        order,
        placed,
    } = scratch;
    before.clear();
    engine_allocation::reserve(before, previous.len())
        .map_err(|_| "combat log observer key allocation failed")?;
    before.extend(previous.iter().map(|row| (row.semantic_key(), row.ordinal)));
    before.sort_unstable_by_key(|&(key, _)| key);
    if before.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err("combat log observer baseline occurrence repeated");
    }
    order.clear();
    engine_allocation::reserve(order, current.len())
        .map_err(|_| "combat log observer row allocation failed")?;
    order.extend(current.iter().enumerate().map(|(index, row)| (row.semantic_key(), index)));
    order.sort_unstable_by_key(|&(key, _)| key);
    if order.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err("combat log observer occurrence repeated");
    }
    let mut rows = engine_allocation::collect_vec(current.iter().cloned())
        .map_err(|_| "combat log observer row allocation failed")?;
    placed.clear();
    engine_allocation::reserve(placed, current.len())
        .map_err(|_| "combat log observer row allocation failed")?;
    // Labels are scoped by full combat/side in the row key. A table-wide high
    // water mark makes new labels independent of which collection grew. No
    // packing steals bits from either native array index or a full handle.
    let mut next = previous
        .iter()
        .map(|row| row.ordinal)
        .max()
        .map_or(Ok(0), |last| {
            last.checked_add(1)
                .ok_or("combat log observer label overflow")
        })?;
    // New occurrences take labels in occurrence order; both lists are in it.
    let mut held = before.iter().peekable();
    for (rank, &(key, index)) in order.iter().enumerate() {
        while held.next_if(|&&(earlier, _)| earlier < key).is_some() {}
        rows[index].ordinal = match held.peek() {
            Some(&&(same, ordinal)) if same == key => ordinal,
            _ => {
                if next > u64::MAX / 2 {
                    return Err("combat log observer label overflow");
                }
                next += 1;
                next - 1
            }
        };
        placed.push((rows[index].key(), rank, index));
    }
    // Key order, occurrence order among equal keys, then moved into place by cycles.
    placed.sort_unstable_by_key(|&(key, rank, _)| (key, rank));
    for start in 0..rows.len() {
        if placed[start].2 == usize::MAX {
            continue;
        }
        let first = std::mem::take(&mut rows[start]);
        let mut hole = start;
        loop {
            let from = std::mem::replace(&mut placed[hole].2, usize::MAX);
            if from == start {
                rows[hole] = first;
                break;
            }
            rows[hole] = std::mem::take(&mut rows[from]);
            hole = from;
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests;
