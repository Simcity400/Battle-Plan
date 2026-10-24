use super::*;
use std::sync::Weak;

pub(crate) struct Request {
    pub base: usize,
    pub index: usize,
    pub part: Work,
}
pub(crate) enum Work {
    Variant(production::VariantPart),
    Context(production::DesignContextPart),
}
impl Work {
    pub(super) fn capture(&mut self, memory: &impl engine_native::Memory, base: usize) {
        match self {
            Self::Variant(part) => part.capture(memory, base),
            Self::Context(part) => part.capture(memory, base),
        }
    }
}
#[derive(Default)]
pub(crate) struct Dispatch {
    executor: Weak<acquisition::Pool<Job, Report>>,
    calibration: Option<Arc<SharedCalibration>>,
    workers: usize,
}
impl Dispatch {
    pub(super) fn arm(&mut self, pool: &Pool) {
        self.executor = Arc::downgrade(&pool.executor);
        self.calibration = Some(Arc::clone(&pool.calibration));
        self.workers = pool.worker_count;
    }
    pub(super) fn lanes(&self) -> usize {
        self.workers.max(1)
    }
    pub(super) fn acquire(
        &self,
        parts: Vec<production::VariantPart>,
        memory: &impl engine_native::Memory,
        base: usize,
    ) -> Result<Vec<production::VariantPart>, &'static str> {
        let parts = engine_allocation::collect_vec(parts.into_iter().map(Work::Variant))
            .map_err(|_| "variant worker request allocation failed")?;
        let parts = self.acquire_work(parts, memory, base)?;
        let mut output = engine_allocation::vec(parts.len())
            .map_err(|_| "variant worker result allocation failed")?;
        for part in parts {
            let Work::Variant(part) = part else {
                return Err("variant worker returned another family");
            };
            output.push(part);
        }
        Ok(output)
    }
    pub(super) fn context(
        &self,
        parts: Vec<production::DesignContextPart>,
        memory: &impl engine_native::Memory,
        base: usize,
    ) -> Result<Vec<production::DesignContextPart>, &'static str> {
        let parts = engine_allocation::collect_vec(parts.into_iter().map(Work::Context))
            .map_err(|_| "design context worker request allocation failed")?;
        let parts = self.acquire_work(parts, memory, base)?;
        let mut output = engine_allocation::vec(parts.len())
            .map_err(|_| "design context worker result allocation failed")?;
        for part in parts {
            let Work::Context(part) = part else {
                return Err("design context worker returned another family");
            };
            output.push(part);
        }
        Ok(output)
    }
    fn acquire_work(
        &self,
        parts: Vec<Work>,
        memory: &impl engine_native::Memory,
        base: usize,
    ) -> Result<Vec<Work>, &'static str> {
        let Some(executor) = self.executor.upgrade().filter(|_| self.workers > 1) else {
            let mut parts = parts;
            for part in &mut parts {
                part.capture(memory, base);
            }
            return Ok(parts);
        };
        let mut pending = Children {
            executor: &executor,
            calibration: self.calibration.as_deref().unwrap(),
            tickets: engine_allocation::vec(parts.len())
                .map_err(|_| "variant worker ticket allocation failed")?,
        };
        let mut output = engine_allocation::vec(parts.len())
            .map_err(|_| "variant worker result allocation failed")?;
        let mut parts = parts.into_iter();
        let local = parts.next();
        let mut failure = None;
        for (index, part) in parts.enumerate() {
            let mut request = Some(Job::VariantPart(Request {
                base,
                index: index + 1,
                part,
            }));
            // SAFETY: the worker parent drains every child before returning its live lease.
            let submitted = unsafe {
                executor.submit_with_priority(&mut request, Role::Variants.priority(), true, true)
            };
            note_dispatch(
                &pending.calibration.roles[Role::Variants as usize],
                submitted.is_ok(),
            );
            match submitted {
                Ok(ticket) => pending.tickets.push(Some(ticket)),
                Err(SubmitError::Full) => {
                    let Some(Job::VariantPart(mut request)) = request else {
                        unreachable!()
                    };
                    request.part.capture(memory, base);
                    output.push(request.part);
                }
                Err(_) => failure = Some("variant partition admission stopped before reading"),
            }
        }
        if failure.is_none()
            && let Some(mut local) = local
        {
            local.capture(memory, base);
            output.push(local);
        }
        for index in 0..pending.tickets.len() {
            let ticket = pending.tickets[index].take().unwrap();
            let mut completed = None;
            executor.finish_worker_into(ticket, &mut completed);
            let Some(completed) = completed else {
                failure = Some("variant partition completion missing");
                continue;
            };
            account_completion(pending.calibration, &completed);
            match completed.report {
                Ok(Report::VariantPart) => {
                    let Job::VariantPart(request) = completed.request else {
                        unreachable!()
                    };
                    output.push(request.part);
                }
                Err(Failure::Panicked) => failure = Some("variant partition acquisition panicked"),
                _ => failure = Some("variant partition acquisition stopped before reading"),
            }
        }
        failure.map_or(Ok(output), Err)
    }
}
struct Children<'a> {
    executor: &'a acquisition::Pool<Job, Report>,
    calibration: &'a SharedCalibration,
    tickets: Vec<Option<Ticket>>,
}
impl Drop for Children<'_> {
    fn drop(&mut self) {
        for ticket in self.tickets.iter_mut().filter_map(Option::take) {
            contain(|| {
                let mut completed = None;
                self.executor.finish_worker_into(ticket, &mut completed);
                if let Some(completed) = completed {
                    account_completion(self.calibration, &completed);
                }
            });
        }
    }
}
