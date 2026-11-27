use std::{
    any::Any,
    collections::{BTreeSet, VecDeque},
    io,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    ptr,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

pub struct Task<T> {
    id: usize,
    value: Arc<OnceLock<io::Result<T>>>,
}

impl<T> Clone for Task<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            value: Arc::clone(&self.value),
        }
    }
}

impl<T> Task<T> {
    pub fn id(&self) -> usize {
        self.id
    }

    pub fn read(&self) -> io::Result<&T> {
        self.value
            .get()
            .expect("dependency completed before its consumer")
            .as_ref()
            .map_err(|error| io::Error::new(error.kind(), error.to_string()))
    }

    pub fn take(self) -> io::Result<T> {
        let value = match Arc::try_unwrap(self.value) {
            Ok(value) => value,
            Err(_) => panic!("task consumers must finish before taking their operand"),
        };
        value.into_inner().expect("task completed")
    }
}

type Work<'a> = Box<dyn FnOnce() + Send + 'a>;

struct Node<'a> {
    work: Option<Work<'a>>,
    cancel: Option<Work<'a>>,
    context: Option<super::CarriedContext>,
    remaining: usize,
    consumers: Vec<usize>,
}

#[derive(Default)]
pub struct Dag<'a> {
    nodes: Vec<Node<'a>>,
}

struct DagState<'a> {
    nodes: Vec<Node<'a>>,
    ready: VecDeque<usize>,
    remaining: usize,
    failed: bool,
    cancelled: bool,
    panic: Option<Box<dyn Any + Send>>,
    overlay_refusals: Vec<BTreeSet<String>>,
}

struct DagBatch {
    pool: *const super::Pool,
    state: *const Mutex<DagState<'static>>,
    ready_count: AtomicUsize,
    finished: AtomicBool,
    alive: Option<Arc<AtomicBool>>,
}

// SAFETY: `DagState` is protected by a `Mutex`, lives on `Dag::run_in`'s stack frame alongside
// `&Pool`, and `BatchRegistration::drop` closes the batch and waits until `in_flight == 0` before
// returning.
unsafe impl Send for DagBatch {}
// SAFETY: same as `Send`.
unsafe impl Sync for DagBatch {}

impl DagBatch {
    fn pool(&self) -> &super::Pool {
        // SAFETY: `BatchRegistration` guarantees `&Pool` outlives every access.
        unsafe { &*self.pool }
    }

    fn state(&self) -> &Mutex<DagState<'static>> {
        // SAFETY: `BatchRegistration` guarantees `DagState` outlives every access.
        unsafe { &*self.state }
    }

    fn is_cancelled(&self) -> bool {
        super::is_root_cancelled(self.alive.as_ref())
    }

    fn run_one(&self) -> bool {
        let (id, work, context) = {
            let mut state = super::lock(self.state());
            if state.failed || state.cancelled || state.remaining == 0 {
                self.ready_count.store(0, Ordering::Release);
                self.finished.store(true, Ordering::Release);
                return false;
            }
            if self.is_cancelled() {
                state.cancelled = true;
                state.ready.clear();
                self.ready_count.store(0, Ordering::Release);
                self.finished.store(true, Ordering::Release);
                return false;
            }
            let Some(id) = state.ready.pop_front() else {
                self.ready_count.store(0, Ordering::Release);
                return false;
            };
            self.ready_count.store(state.ready.len(), Ordering::Release);
            let work = state.nodes[id].work.take().expect("ready task has work");
            let context = state.nodes[id]
                .context
                .take()
                .expect("ready task has context");
            (id, work, context)
        };
        let result = catch_unwind(AssertUnwindSafe(|| context.enter("content parallel", work)));
        let mut state = super::lock(self.state());
        match result {
            Ok(((), refusals)) => {
                if !refusals.is_empty() {
                    state.overlay_refusals[id].extend(refusals);
                }
                state.remaining -= 1;
                if self.is_cancelled() {
                    state.cancelled = true;
                    state.ready.clear();
                    self.ready_count.store(0, Ordering::Release);
                    self.finished.store(true, Ordering::Release);
                } else if !state.failed && !state.cancelled {
                    let consumers = std::mem::take(&mut state.nodes[id].consumers);
                    for consumer in consumers {
                        let node = &mut state.nodes[consumer];
                        node.remaining -= 1;
                        if node.remaining == 0 {
                            state.ready.push_back(consumer);
                        }
                    }
                    let ready_len = state.ready.len();
                    self.ready_count.store(ready_len, Ordering::Release);
                    if state.remaining == 0 {
                        self.finished.store(true, Ordering::Release);
                    }
                } else {
                    self.ready_count.store(0, Ordering::Release);
                }
            }
            Err(panic) => {
                state.failed = true;
                state.ready.clear();
                state.panic.get_or_insert(panic);
                self.ready_count.store(0, Ordering::Release);
                self.finished.store(true, Ordering::Release);
            }
        }
        let wake = state.ready.len();
        drop(state);
        if wake > 0 {
            self.pool().wake_helpers(wake);
        } else {
            self.pool().notify_all();
        }
        true
    }
}

impl super::WorkBatch for DagBatch {
    fn has_claimable(&self) -> bool {
        !self.finished.load(Ordering::Acquire) && self.ready_count.load(Ordering::Acquire) > 0
    }

    fn run_turn(&self) {
        self.run_one();
    }
}

impl<'a> Dag<'a> {
    pub fn add<T: Send + Sync + 'a>(
        &mut self,
        dependencies: &[usize],
        work: impl FnOnce() -> io::Result<T> + Send + 'a,
    ) -> Task<T> {
        let id = self.nodes.len();
        let mut dependencies = dependencies.to_vec();
        dependencies.sort_unstable();
        dependencies.dedup();
        for &dependency in &dependencies {
            assert!(
                dependency < id,
                "a task depends only on previously declared operands"
            );
            self.nodes[dependency].consumers.push(id);
        }
        let value = Arc::new(OnceLock::new());
        let output = Arc::clone(&value);
        let cancel_output = Arc::clone(&value);
        self.nodes.push(Node {
            work: Some(Box::new(move || {
                assert!(output.set(work()).is_ok(), "a task executes once");
            })),
            cancel: Some(Box::new(move || {
                let _ = cancel_output.set(Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "cancelled",
                )));
            })),
            context: Some(super::CarriedContext::capture()),
            remaining: dependencies.len(),
            consumers: Vec::new(),
        });
        Task { id, value }
    }

    pub fn run(self) {
        self.run_in(super::shared_pool());
    }

    pub(crate) fn run_in(mut self, pool: &super::Pool) {
        let count = self.nodes.len();
        if count == 0 {
            return;
        }
        let alive = super::current_alive();
        if let Some(alive) = &alive {
            for node in &mut self.nodes {
                if let Some(context) = &mut node.context
                    && context.alive.is_none()
                {
                    context.alive = Some(Arc::clone(alive));
                }
            }
        }
        let ready: VecDeque<usize> = self
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(id, node)| (node.remaining == 0).then_some(id))
            .collect();
        let initial_ready = ready.len();
        let root = pool.enter_root();
        let state = Mutex::new(DagState {
            nodes: self.nodes,
            ready,
            remaining: count,
            failed: false,
            cancelled: false,
            panic: None,
            overlay_refusals: (0..count).map(|_| BTreeSet::new()).collect(),
        });
        // SAFETY: `BatchRegistration` closes `DagBatch` and waits until `in_flight == 0` before
        // `state` is dropped on this stack frame.
        let erased_state: *const Mutex<DagState<'static>> = unsafe {
            std::mem::transmute::<*const Mutex<DagState<'a>>, *const Mutex<DagState<'static>>>(
                ptr::from_ref(&state),
            )
        };
        let batch = Arc::new(DagBatch {
            pool: ptr::from_ref(pool),
            state: erased_state,
            ready_count: AtomicUsize::new(initial_ready),
            finished: AtomicBool::new(false),
            alive: alive.clone(),
        });
        let registration = pool.register_batch(
            root.root_id,
            alive,
            Arc::clone(&batch) as Arc<dyn super::WorkBatch>,
            initial_ready.saturating_sub(1),
        );
        let meta = registration.meta();
        loop {
            while batch.run_one() {}
            if batch.finished.load(Ordering::Acquire) && registration.in_flight() == 0 {
                break;
            }
            pool.wait_until(meta, || {
                batch.ready_count.load(Ordering::Acquire) > 0
                    || (batch.finished.load(Ordering::Acquire) && registration.in_flight() == 0)
            });
        }
        drop(registration);
        drop(batch);
        let mut state = state
            .into_inner()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(panic) = state.panic.take() {
            resume_unwind(panic);
        }
        if state.cancelled {
            for node in &mut state.nodes {
                if node.work.take().is_some()
                    && let Some(cancel) = node.cancel.take()
                {
                    cancel();
                }
            }
        }
        for refusals in &state.overlay_refusals {
            crate::content::credit_overlay_observers(refusals);
        }
    }
}
