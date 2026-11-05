use super::*;
use std::sync::Arc;

pub(super) type Tree = Option<Arc<Vec<rows::ModifierBreakdownNode>>>;

pub(super) fn allocated_bytes(trees: &Vec<Tree>) -> usize {
    trees.capacity() * size_of::<Tree>()
        + trees
            .iter()
            .flatten()
            .map(|tree| {
                tree.capacity() * size_of::<rows::ModifierBreakdownNode>()
                    + tree.iter().map(|node| node.name.capacity()).sum::<usize>()
            })
            .sum::<usize>()
}

/// An equal re-read keeps the held tree, so an unchanged row stays the same
/// allocation and compares by pointer.
fn kept(held: Option<&Tree>, read: Option<Vec<rows::ModifierBreakdownNode>>) -> Tree {
    match (held.and_then(Option::as_ref), read) {
        (Some(held), Some(read)) if **held == read => Some(Arc::clone(held)),
        (_, read) => read.map(Arc::new),
    }
}

pub(super) fn capture(memory: &impl Memory, base: usize, job: &mut WeatherJob) {
    let Some(array) = job.regions.array else {
        return;
    };
    let count = array.count as usize;
    if job.worker_growth
        && engine_allocation::reserve_exact(&mut job.air_efficiency, count).is_err()
    {
        return;
    }
    let room = count.min(job.air_efficiency.capacity());
    let read = |slot| {
        array
            .element(slot, sweather_per_region::STRIDE)
            .and_then(|record| record.checked_add(sweather_per_region::MODIFIERS))
            .and_then(|root| {
                engine_native::modifier_breakdown::printed_nodes(memory, base, root, 0x11).ok()
            })
            .and_then(|nodes| rows::printed_tree(&nodes).ok())
    };
    if let Some(carry) = job.carry.as_deref_mut()
        && let Some((first, visits)) = carry.region_window()
        && carry.air_efficiency.len() == count
        && carry.air_efficiency.iter().all(Option::is_some)
    {
        for visit in 0..visits {
            let slot = (first + visit) % array.count;
            let held = &mut carry.air_efficiency[slot as usize];
            *held = kept(Some(&*held), read(slot));
        }
        job.air_efficiency
            .extend(carry.air_efficiency[..room].iter().cloned());
    } else {
        let previous = job
            .carry
            .as_deref()
            .map(|carry| carry.air_efficiency.as_slice())
            .filter(|held| held.len() == count);
        job.air_efficiency.extend(
            (0..room as u32).map(|slot| kept(previous.and_then(|held| held.get(slot as usize)), read(slot))),
        );
        if let Some(carry) = job.carry.as_deref_mut() {
            carry.air_efficiency.clear();
            if room == count
                && engine_allocation::reserve_exact(&mut carry.air_efficiency, count).is_ok()
            {
                carry
                    .air_efficiency
                    .extend(job.air_efficiency.iter().cloned());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(value: i64) -> Vec<rows::ModifierBreakdownNode> {
        vec![rows::ModifierBreakdownNode {
            children: 0,
            value,
            name: b"air".to_vec(),
        }]
    }

    #[test]
    fn an_equal_read_keeps_the_held_tree_and_a_changed_one_replaces_it() {
        let held: Tree = Some(Arc::new(tree(5)));
        let same = kept(Some(&held), Some(tree(5))).unwrap();
        assert!(Arc::ptr_eq(&same, held.as_ref().unwrap()));
        let changed = kept(Some(&held), Some(tree(6))).unwrap();
        assert!(!Arc::ptr_eq(&changed, held.as_ref().unwrap()));
        assert_eq!(*changed, tree(6));
        assert_eq!(kept(Some(&held), None), None);
        assert_eq!(kept(None, Some(tree(5))).as_deref(), Some(&tree(5)));
    }
}
