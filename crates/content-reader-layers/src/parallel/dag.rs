use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Condvar, Mutex, OnceLock},
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
    remaining: usize,
    consumers: Vec<usize>,
}

#[derive(Default)]
pub struct Dag<'a> {
    nodes: Vec<Node<'a>>,
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
        self.nodes.push(Node {
            work: Some(Box::new(super::carried(move || {
                assert!(output.set(work()).is_ok(), "a task executes once");
            }))),
            remaining: dependencies.len(),
            consumers: Vec::new(),
        });
        Task { id, value }
    }

    pub fn run(self) {
        struct State<'a> {
            nodes: Vec<Node<'a>>,
            ready: VecDeque<usize>,
            remaining: usize,
            failed: bool,
        }
        let count = self.nodes.len();
        let ready = self
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(id, node)| (node.remaining == 0).then_some(id))
            .collect();
        let state = Mutex::new(State {
            nodes: self.nodes,
            ready,
            remaining: count,
            failed: false,
        });
        let changed = Condvar::new();
        let workers = std::thread::available_parallelism()
            .map_or(1, |count| count.get())
            .min(count);
        super::map(&vec![(); workers], |_| {
            loop {
                let (id, work) = {
                    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
                    loop {
                        if state.failed || state.remaining == 0 {
                            return;
                        }
                        if let Some(id) = state.ready.pop_front() {
                            break (
                                id,
                                state.nodes[id].work.take().expect("ready task has work"),
                            );
                        }
                        state = changed
                            .wait(state)
                            .unwrap_or_else(|error| error.into_inner());
                    }
                };
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
                let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
                if let Err(panic) = result {
                    state.failed = true;
                    changed.notify_all();
                    drop(state);
                    std::panic::resume_unwind(panic);
                }
                state.remaining -= 1;
                let consumers = std::mem::take(&mut state.nodes[id].consumers);
                for consumer in consumers {
                    let node = &mut state.nodes[consumer];
                    node.remaining -= 1;
                    if node.remaining == 0 {
                        state.ready.push_back(consumer);
                    }
                }
                changed.notify_all();
            }
        });
    }
}
