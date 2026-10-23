//! The order a grouped block's tables are handed to decode workers.
use super::super::aliases::private::{TABLES, dependencies};
use super::*;

fn depth(table: TableId, depths: &mut [Option<usize>; TABLE_COUNT]) -> usize {
    if let Some(depth) = depths[table.index()] {
        return depth;
    }
    let depth = dependencies(table)
        .iter()
        .map(|&source| depth(source, depths) + 1)
        .max()
        .unwrap_or(0);
    depths[table.index()] = Some(depth);
    depth
}

/// Schedule alias dependencies first, then measured weight and registry order.
pub(super) fn table_order(sizes: &BTreeMap<usize, usize>) -> io::Result<Vec<usize>> {
    let mut depths = [None; TABLE_COUNT];
    let mut sources = engine_allocation::collect_vec(TABLES.iter().copied())?;
    let mut at = 0;
    while let Some(&table) = sources.get(at) {
        for &source in dependencies(table) {
            if !sources.contains(&source) {
                engine_allocation::reserve(&mut sources, 1)?;
                sources.push(source);
            }
        }
        at += 1;
    }
    let weight = |index: usize| match sizes.get(&index) {
        Some(&bytes) => (false, std::cmp::Reverse(bytes), index),
        None => (true, std::cmp::Reverse(0), index),
    };
    let mut order = engine_allocation::collect_vec(TableId::ALL.iter().map(|table| table.index()))?;
    // Every key is distinct, so the unstable sort's order is the only one.
    order.sort_unstable_by_key(|&index| {
        let table = TableId::ALL[index];
        let graph = sources.contains(&table);
        let level = if graph { depth(table, &mut depths) } else { 0 };
        (!graph, level, weight(index))
    });
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(order: &[usize], table: TableId) -> usize {
        order
            .iter()
            .position(|&index| index == table.index())
            .unwrap()
    }

    /// The alias tables and every table they read, directly or not.
    fn graph() -> Vec<TableId> {
        let mut graph = TABLES.to_vec();
        let mut at = 0;
        while at < graph.len() {
            for &source in dependencies(graph[at]) {
                if !graph.contains(&source) {
                    graph.push(source);
                }
            }
            at += 1;
        }
        graph
    }

    #[test]
    fn alias_sources_come_before_the_tables_reading_them_then_the_rest() {
        let graph = graph();
        for sizes in [
            BTreeMap::new(),
            TableId::ALL
                .iter()
                .map(|table| (table.index(), 1 + table.index() % 7))
                .collect(),
            // The sources measured lightest, their readers heaviest.
            graph
                .iter()
                .map(|table| (table.index(), 1_000 + 100 * dependencies(*table).len()))
                .collect(),
        ] {
            let order = table_order(&sizes).unwrap();
            // A permutation of every registry position.
            let mut sorted = order.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, (0..TABLE_COUNT).collect::<Vec<_>>());
            for &table in &graph {
                for &source in dependencies(table) {
                    assert!(
                        position(&order, source) < position(&order, table),
                        "{source:?} before {table:?}"
                    );
                }
            }
            let last_graph = graph
                .iter()
                .map(|&table| position(&order, table))
                .max()
                .unwrap();
            assert_eq!(last_graph, graph.len() - 1, "alias graph first");
        }
    }

    #[test]
    fn other_tables_go_heaviest_first_then_unmeasured_in_registry_order() {
        let graph = graph();
        let rest: Vec<usize> = TableId::ALL
            .iter()
            .filter(|table| !graph.contains(table))
            .map(|table| table.index())
            .collect();
        // Unmeasured: registry order.
        let order = table_order(&BTreeMap::new()).unwrap();
        assert_eq!(order[order.len() - rest.len()..], rest[..]);
        // Two measured tables lead the rest, the larger first.
        let (small, large) = (rest[rest.len() - 1], rest[rest.len() / 2]);
        let sizes = BTreeMap::from([(small, 10), (large, 99)]);
        let order = table_order(&sizes).unwrap();
        let tail = &order[order.len() - rest.len()..];
        assert_eq!(tail[..2], [large, small]);
        let unmeasured: Vec<_> = rest
            .iter()
            .copied()
            .filter(|index| *index != small && *index != large)
            .collect();
        assert_eq!(tail[2..], unmeasured[..]);
    }

    #[test]
    fn tables_at_one_depth_go_heaviest_first() {
        let level = |table| depth(table, &mut [None; TABLE_COUNT]);
        let first: Vec<TableId> = graph()
            .into_iter()
            .filter(|&table| level(table) == 0)
            .collect();
        assert!(first.len() >= 2);
        // The registry's last source measured heaviest leads its level.
        let heavy = *first.iter().max_by_key(|table| table.index()).unwrap();
        let sizes = first
            .iter()
            .map(|table| (table.index(), if *table == heavy { 99 } else { 1 }))
            .collect();
        let order = table_order(&sizes).unwrap();
        assert_eq!(order[0], heavy.index());
    }
}
