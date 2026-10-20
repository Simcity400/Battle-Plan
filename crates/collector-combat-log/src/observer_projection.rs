//! Stable occurrence labels from the encoder's complete baseline, not values.
//! No native reads or process-global identity cache are involved.
use replay_data::rows::CombatLogObserverRow;

pub fn project(
    current: &[CombatLogObserverRow],
    previous: &[CombatLogObserverRow],
) -> Result<Vec<CombatLogObserverRow>, &'static str> {
    let mut before = engine_allocation::collect_vec(previous.iter())
        .map_err(|_| "combat log observer key allocation failed")?;
    before.sort_unstable_by_key(|row| row.semantic_key());
    if before
        .windows(2)
        .any(|rows| rows[0].semantic_key() == rows[1].semantic_key())
    {
        return Err("combat log observer baseline occurrence repeated");
    }
    let mut rows = engine_allocation::collect_vec(current.iter().cloned())
        .map_err(|_| "combat log observer row allocation failed")?;
    rows.sort_unstable_by_key(CombatLogObserverRow::semantic_key);
    if rows
        .windows(2)
        .any(|rows| rows[0].semantic_key() == rows[1].semantic_key())
    {
        return Err("combat log observer occurrence repeated");
    }
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
    for row in &mut rows {
        if let Ok(index) =
            before.binary_search_by_key(&row.semantic_key(), |row| row.semantic_key())
        {
            row.ordinal = before[index].ordinal;
        } else {
            if next > u64::MAX / 2 {
                return Err("combat log observer label overflow");
            }
            row.ordinal = next;
            next += 1;
        }
    }
    rows.sort_unstable_by_key(CombatLogObserverRow::key);
    Ok(rows)
}

#[cfg(test)]
mod tests;
