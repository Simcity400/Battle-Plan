use collector_workers::{
    acquisition::verify_allocated_bytes,
    roles::MAX_JOBS,
};

#[test]
fn executor_allocated_bytes_include_every_real_allocation() {
    for workers in [0, 2] {
        for capacity in [1, 3, MAX_JOBS] {
            let bias = [7u8; 129];
            // SAFETY: Test owns these temporary pools and shuts them down before returning.
            unsafe {
                verify_allocated_bytes(workers, capacity, (3, |_| 0), move |request: &mut u8| {
                    u64::from(*request) + u64::from(std::hint::black_box(&bias)[0])
                });
                verify_allocated_bytes(
                    workers,
                    capacity,
                    (3, |_| 0),
                    move |_: &mut crate::acquisition_jobs::Job| {
                        crate::acquisition_jobs::Report::World(
                            crate::acquisition_jobs::WorldReport {
                                ticks: u64::from(std::hint::black_box(&bias)[0]),
                                ..Default::default()
                            },
                        )
                    },
                );
            }
        }
    }
}

