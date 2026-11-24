use super::*;

pub(super) struct Boundary {
    pub active: bool,
    pub world_epoch: u64,
    pub availability: Option<u64>,
    pub lifetime_epoch: Option<u64>,
    pub variant_epoch: Option<u64>,
    pub stamp: Option<((u64, u64), u64)>,
    pub retired: Vec<usize>,
    pub dirty: Vec<usize>,
    pub operands: crate::observations::Taken,
    pub result: Result<(), &'static str>,
}
impl Default for Boundary {
    fn default() -> Self {
        Self {
            active: false,
            world_epoch: 0,
            availability: None,
            lifetime_epoch: None,
            variant_epoch: None,
            stamp: None,
            retired: Vec::new(),
            dirty: Vec::new(),
            operands: Default::default(),
            result: Ok(()),
        }
    }
}
impl Boundary {
    pub fn allocated_bytes(&self) -> usize {
        (self.retired.capacity() + self.dirty.capacity()) * std::mem::size_of::<usize>()
            + self.operands.allocated_bytes()
    }
}
impl Store {
    pub fn capture_boundary(&mut self) {
        let boundary = &mut self.boundary;
        if boundary.active {
            return;
        }
        boundary.active = true;
        boundary.world_epoch = operands::world_epoch();
        boundary.availability = operands::variant_capability();
        boundary.retired.clear();
        boundary.dirty.clear();
        boundary.result = Ok(());
        boundary.lifetime_epoch = operands::take_lifetimes(|pointer| {
            if engine_allocation::reserve(&mut boundary.retired, 1).is_err() {
                boundary.result = Err("variant lifetime ranges allocation failed");
            } else {
                boundary.retired.push(pointer);
            }
        });
        boundary.retired.sort_unstable();
        if boundary.result.is_ok() {
            boundary.variant_epoch = operands::take(|pointer| {
                if engine_allocation::reserve(&mut boundary.dirty, 1).is_err() {
                    boundary.result = Err("attrition dirty ranges allocation failed");
                } else {
                    boundary.dirty.push(pointer);
                }
            });
            boundary.dirty.sort_unstable();
        }
        boundary.stamp = operands::observed_variant_stamp();
        boundary
            .operands
            .take(|visit| operands::take_operands(visit));
    }
    pub fn can_settle_owned(&self) -> bool {
        self.boundary.active
            && self.boundary.result.is_ok()
            && self.world_epoch == self.boundary.world_epoch
            && self.lifetime_epoch == self.boundary.lifetime_epoch
            && self.boundary.availability.is_some()
    }
    #[allow(clippy::too_many_arguments)]
    pub fn settle_owned<'a>(
        &mut self,
        base: usize,
        state: usize,
        seeds: &[DivisionSeed],
        selected: &mut Vec<(u64, Option<[u64; 3]>)>,
        prepared: impl Fn(usize) -> Option<Selection>,
        reads: impl Fn(usize) -> Option<&'a Preparation>,
        variant_reads: impl Fn(u64, usize) -> Option<&'a Preparation>,
    ) -> Result<Arc<SourceSnapshot>, &'static str> {
        assert!(
            self.can_settle_owned(),
            "source boundary captured before resume"
        );
        let boundary = std::mem::take(&mut self.boundary);
        self.settling_owned = true;
        self.settling_availability = boundary.availability;
        let result = self.capture_inner(
            &Owned,
            base,
            state,
            seeds,
            selected,
            prepared,
            reads,
            variant_reads,
            &boundary,
        );
        self.settling_owned = false;
        self.boundary = boundary;
        self.boundary.active = false;
        result
    }
}
struct Owned;
impl Memory for Owned {
    fn copy(&self, _: usize, _: &mut [u8]) -> bool {
        panic!("owned source settlement attempted a native read")
    }
}
