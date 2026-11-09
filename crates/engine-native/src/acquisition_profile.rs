use std::{cell::Cell, time::Instant};

#[derive(Clone, Copy, Default)]
pub struct Costs {
    pub field_ns: u64,
    pub bulk_ns: u64,
    pub pointer_width_reads: u64,
}

thread_local! {
    static COSTS: Cell<Costs> = Cell::new(Costs::default());
}

pub fn take() -> Costs {
    COSTS.with(|costs| costs.replace(Costs::default()))
}

pub fn pointer_width_read() {
    COSTS.with(|costs| {
        let mut value = costs.get();
        value.pointer_width_reads += 1;
        costs.set(value);
    });
}

pub struct Timer {
    start: Instant,
    bulk: bool,
}

impl Timer {
    pub fn new(bulk: bool) -> Self {
        Self {
            start: Instant::now(),
            bulk,
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        let ns = self.start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        COSTS.with(|costs| {
            let mut value = costs.get();
            if self.bulk {
                value.bulk_ns += ns;
            } else {
                value.field_ns += ns;
            }
            costs.set(value);
        });
    }
}
