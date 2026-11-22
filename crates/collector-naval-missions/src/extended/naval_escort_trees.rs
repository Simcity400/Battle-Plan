use engine_executable::modifier_keys;
use engine_native::{Memory, ccountry};
use engine_naval_history::naval_modifier_trees as native;
use replay_data::rows::{ModifierBreakdownNode, Recorded};
use std::sync::Mutex;
const KEYS: [u32; 1] = [modifier_keys::CONVOY_ESCORT_EFFICIENCY];
static REFUSALS: Mutex<std::collections::BTreeMap<&'static str, u64>> =
    Mutex::new(std::collections::BTreeMap::new());
fn refused(reason: &'static str) {
    *REFUSALS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(reason)
        .or_default() += 1;
}
#[derive(Default)]
pub struct Trees {
    nodes: Vec<native::RawNode<1>>,
    text: Vec<u8>,
    stack: Vec<native::Frame>,
    definitions: Option<Result<native::Definitions<1>, &'static str>>,
}
impl Trees {
    pub fn reset(&mut self) {
        self.nodes.clear();
        self.text.clear();
        self.stack.clear();
        self.definitions = None;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.nodes.capacity() * size_of::<native::RawNode<1>>()
            + self.text.capacity()
            + self.stack.capacity() * size_of::<native::Frame>()
    }
    pub fn read(
        &mut self,
        memory: &impl Memory,
        image: usize,
        country: ccountry::Country,
        demanded: bool,
        certificate: Option<(u64, u64)>,
    ) -> Option<[usize; 2]> {
        if !demanded {
            return None;
        }
        if certificate.is_none() {
            refused("escort modifier writer certificate unavailable");
            return None;
        }
        let first = self.nodes.len();
        let text_first = self.text.len();
        let result = (|| {
            let definitions = self
                .definitions
                .get_or_insert_with(|| native::Definitions::requested(memory, image, KEYS));
            let definitions = definitions.as_ref().map_err(|reason| *reason)?;
            let root = country
                .pointer
                .checked_add(native::COUNTRY_TREE)
                .ok_or("escort modifier tree address overflow")?;
            native::capture_nonzero_tree(
                memory,
                root,
                definitions,
                &mut self.nodes,
                &mut self.text,
                &mut self.stack,
            )
        })();
        match result {
            Ok(()) => Some([first, self.nodes.len() - first]),
            Err(reason) => {
                self.nodes.truncate(first);
                self.text.truncate(text_first);
                refused(reason);
                None
            }
        }
    }
    pub fn project(
        &self,
        span: Option<[usize; 2]>,
    ) -> Recorded<Option<Vec<ModifierBreakdownNode>>> {
        let value = span.and_then(|[first, len]| {
            let end = first.checked_add(len)?;
            let nodes = self.nodes.get(first..end)?;
            if nodes.is_empty() {
                return Some(Vec::new());
            }
            match collector_naval_details::naval_trees::project_keys(nodes, &self.text, &KEYS) {
                Ok(mut trees) => Some(trees.pop().map_or_else(Vec::new, |tree| tree.nodes)),
                Err(reason) => {
                    refused(reason);
                    None
                }
            }
        });
        Recorded::Value(value)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn undemanded_and_uncertified_sources_never_touch_memory() {
        let memory = engine_native::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        let country = ccountry::Country {
            pointer: usize::MAX,
            canonical_slot: 0,
            id: 0,
        };
        let mut trees = Trees::default();
        assert_eq!(trees.read(&memory, 0, country, false, Some((1, 0))), None);
        assert_eq!(trees.read(&memory, 0, country, true, None), None);
        assert_eq!(trees.allocated_bytes(), 0);
        assert_eq!(trees.project(None), Recorded::Value(None));
    }
    #[test]
    fn projected_empty_differs_from_refused_and_reset_reuses_storage() {
        let mut trees = Trees::default();
        trees.nodes.push(native::RawNode::default());
        assert_eq!(
            trees.project(Some([0, 1])),
            Recorded::Value(Some(Vec::new()))
        );
        let bytes = trees.allocated_bytes();
        trees.reset();
        assert_eq!(trees.allocated_bytes(), bytes);
    }
}
