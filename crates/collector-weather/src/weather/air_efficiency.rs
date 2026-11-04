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
            .map(Arc::new)
    };
    if let Some(carry) = job.carry.as_deref_mut()
        && let Some((first, visits)) = carry.region_window()
        && carry.air_efficiency.len() == count
        && carry.air_efficiency.iter().all(Option::is_some)
    {
        for visit in 0..visits {
            let slot = (first + visit) % array.count;
            carry.air_efficiency[slot as usize] = read(slot);
        }
        job.air_efficiency
            .extend(carry.air_efficiency[..room].iter().cloned());
    } else {
        job.air_efficiency.extend((0..room as u32).map(read));
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
