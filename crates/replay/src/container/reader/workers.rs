use super::*;
use std::{sync::mpsc, thread, time::Duration};

type Job = Box<dyn FnOnce() + Send>;

pub(super) struct Workers {
    senders: Vec<mpsc::SyncSender<Job>>,
    threads: Vec<thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    #[cfg(test)]
    sent: Option<mpsc::Sender<()>>,
}

impl Workers {
    pub(super) fn new(count: usize, stop: Arc<AtomicBool>) -> io::Result<Self> {
        let mut pool = Self {
            senders: Vec::new(),
            threads: Vec::new(),
            stop,
            #[cfg(test)]
            sent: None,
        };
        if count <= 1 {
            return Ok(pool);
        }
        let priority = job_priority::Caller::current();
        for _ in 0..count {
            let (tx, rx) = mpsc::sync_channel::<Job>(1);
            let priority = priority.clone();
            let thread = thread::Builder::new()
                .name("replay-table".into())
                .spawn(move || {
                    priority.enter();
                    while let Ok(job) = rx.recv() {
                        job();
                    }
                })?;
            pool.senders.push(tx);
            pool.threads.push(thread);
        }
        Ok(pool)
    }

    pub(super) fn start<T: Send + 'static, R: Send + 'static>(
        &self,
        items: Vec<T>,
        work: impl Fn(T, &mut Check<'_>) -> R + Send + Sync + 'static,
    ) -> io::Result<Pending<R>> {
        #[cfg(test)]
        let _memory = ownership::category(ownership::Kind::Jobs);
        #[cfg(test)]
        let context = ownership::Context::capture();
        let count = self.senders.len().min(items.len()).max(1);
        let mut batches = allocation::collect_vec((0..count).map(|_| Vec::new()))?;
        for (index, item) in items.into_iter().enumerate() {
            allocation::reserve(&mut batches[index % count], 1)?;
            batches[index % count].push((index, item));
        }
        let (tx, rx) = mpsc::sync_channel(count);
        let work = Arc::new(work);
        for (worker, batch) in batches.into_iter().enumerate() {
            let work = Arc::clone(&work);
            let tx = tx.clone();
            let stop = Arc::clone(&self.stop);
            #[cfg(test)]
            let sent = self.sent.clone();
            #[cfg(test)]
            let context = context.clone();
            let job: Job = Box::new(move || {
                #[cfg(test)]
                let _memory = context.enter(ownership::Kind::Results);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut cancel = || {
                        if stop.load(Ordering::Acquire) {
                            Err(io::ErrorKind::Interrupted.into())
                        } else {
                            Ok(())
                        }
                    };
                    let mut check = Check::new(&mut cancel);
                    batch
                        .into_iter()
                        .map(|(index, item)| (index, work(item, &mut check)))
                        .collect::<Vec<_>>()
                }))
                .map_err(|_| io::Error::other("replay table worker panicked"));
                let _ = tx.send(result);
                #[cfg(test)]
                if let Some(sent) = sent {
                    let _ = sent.send(());
                }
            });
            if self.senders.is_empty() {
                job();
            } else {
                self.senders[worker]
                    .send(job)
                    .map_err(|_| io::Error::other("replay table worker disconnected"))?;
            }
        }
        Ok(Pending { rx, count })
    }

    pub(super) fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.senders.clear();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub(super) struct Pending<R> {
    rx: mpsc::Receiver<io::Result<Vec<(usize, R)>>>,
    count: usize,
}

impl<R> Pending<R> {
    pub(super) fn finish(self, stop: &AtomicBool, check: &mut Check<'_>) -> io::Result<Vec<R>> {
        let mut out = Vec::new();
        for _ in 0..self.count {
            loop {
                if let Err(error) = check.run() {
                    stop.store(true, Ordering::Release);
                    return Err(error);
                }
                match self.rx.recv_timeout(Duration::from_millis(10)) {
                    Ok(batch) => {
                        out.extend(batch?);
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        return Err(io::Error::other("replay table worker disconnected"));
                    }
                }
            }
        }
        out.sort_unstable_by_key(|(index, _)| *index);
        allocation::collect_vec(out.into_iter().map(|(_, value)| value))
    }
}

type Load = (Arc<Replay>, usize, [bool; TABLE_COUNT], chunks::Credit);

pub(super) struct Ahead {
    send: Option<mpsc::SyncSender<Load>>,
    receive: Option<mpsc::Receiver<io::Result<chunks::BlockChunks>>>,
    thread: Option<thread::JoinHandle<()>>,
    pub pending: Option<usize>,
    stop: Arc<AtomicBool>,
}

impl Ahead {
    pub(super) fn new(stop: Arc<AtomicBool>) -> io::Result<Self> {
        let (send, requests) = mpsc::sync_channel::<Load>(1);
        let (results, receive) = mpsc::sync_channel(1);
        let stopped = Arc::clone(&stop);
        let priority = job_priority::Caller::current();
        #[cfg(test)]
        let context = ownership::Context::capture();
        let thread = thread::Builder::new()
            .name("replay-inflate".into())
            .spawn(move || {
                priority.enter();
                #[cfg(test)]
                let _memory = context.enter(ownership::Kind::Raw);
                while let Ok((replay, position, selected, credit)) = requests.recv() {
                    let mut cancel = || {
                        if stopped.load(Ordering::Acquire) {
                            Err(io::ErrorKind::Interrupted.into())
                        } else {
                            Ok(())
                        }
                    };
                    let result = chunks::BlockChunks::load(
                        &replay,
                        position,
                        &selected,
                        true,
                        credit,
                        &mut Check::new(&mut cancel),
                    );
                    if results.send(result).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            send: Some(send),
            receive: Some(receive),
            thread: Some(thread),
            pending: None,
            stop,
        })
    }
    pub(super) fn request(
        &mut self,
        replay: Arc<Replay>,
        position: usize,
        selected: [bool; TABLE_COUNT],
        credit: chunks::Credit,
    ) -> io::Result<()> {
        if self.pending.is_some() {
            return Err(io::Error::other("read-ahead slot already occupied"));
        }
        self.send
            .as_ref()
            .unwrap()
            .send((replay, position, selected, credit))
            .map_err(|_| io::Error::other("replay inflate worker disconnected"))?;
        self.pending = Some(position);
        Ok(())
    }
    pub(super) fn take(&mut self, check: &mut Check<'_>) -> io::Result<chunks::BlockChunks> {
        loop {
            if let Err(error) = check.run() {
                self.stop.store(true, Ordering::Release);
                return Err(error);
            }
            match self
                .receive
                .as_ref()
                .unwrap()
                .recv_timeout(Duration::from_millis(10))
            {
                Ok(result) => {
                    self.pending = None;
                    return result;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(io::Error::other("replay inflate worker disconnected"));
                }
            }
        }
    }
}

impl Drop for Ahead {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.send.take();
        self.receive.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_panic_and_disconnect_release_all_owned_jobs() {
        let stop = Arc::new(AtomicBool::new(false));
        let mut pool = Workers::new(2, Arc::clone(&stop)).unwrap();
        let credits = Arc::new(chunks::Credits::default());
        let jobs = vec![credits.acquire(2).unwrap(), credits.acquire(2).unwrap()];
        let pending = pool
            .start(jobs, |_credit, _check| -> () {
                panic!("synthetic worker failure");
            })
            .unwrap();
        let error = pending.finish(&stop, &mut Check::disabled()).unwrap_err();
        assert_eq!(error.to_string(), "replay table worker panicked");
        pool.shutdown();
        assert_eq!(credits.counts(), (0, 2));
    }

    #[test]
    fn cancellation_drops_a_full_result_queue_before_joining() {
        let audit = ownership::Audit::new();
        let scope = audit.enter();
        let stop = Arc::new(AtomicBool::new(false));
        let mut pool = Workers::new(2, Arc::clone(&stop)).unwrap();
        let credits = Arc::new(chunks::Credits::default());
        let jobs = vec![credits.acquire(2).unwrap(), credits.acquire(2).unwrap()];
        let (sent, receives) = mpsc::channel();
        pool.sent = Some(sent);
        let pending = pool.start(jobs, |credit, _| credit).unwrap();
        for _ in 0..2 {
            receives.recv().unwrap();
        }
        assert_eq!(credits.counts(), (2, 2));
        let mut cancel = || Err(io::ErrorKind::Interrupted.into());
        assert_eq!(
            pending
                .finish(&stop, &mut Check::new(&mut cancel))
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::Interrupted
        );
        pool.shutdown();
        assert_eq!(credits.counts(), (0, 2));
        assert!(audit.peak(ownership::Kind::Jobs) > 0 && audit.peak(ownership::Kind::Results) > 0);
        drop((pool, credits, receives, stop, scope));
        audit.clear_dead_witnesses();
        assert_eq!(audit.live_total(), 0);
    }
    #[test]
    fn disconnected_results_drop_worker_owned_payloads_after_the_handshake() {
        let audit = ownership::Audit::new();
        let scope = audit.enter();
        let stop = Arc::new(AtomicBool::new(false));
        let mut pool = Workers::new(2, Arc::clone(&stop)).unwrap();
        let credits = Arc::new(chunks::Credits::default());
        let jobs = vec![credits.acquire(2).unwrap(), credits.acquire(2).unwrap()];
        let gate = Arc::new(std::sync::Barrier::new(3));
        let worker_gate = Arc::clone(&gate);
        let (ready, arrived) = mpsc::channel();
        let pending = pool
            .start(jobs, move |credit, _| {
                ready.send(()).unwrap();
                worker_gate.wait();
                (credit, vec![1u8; 4096])
            })
            .unwrap();
        for _ in 0..2 {
            arrived.recv().unwrap();
        }
        assert_eq!(credits.counts(), (2, 2));
        drop(pending);
        gate.wait();
        pool.shutdown();
        assert_eq!(credits.counts(), (0, 2));
        assert!(audit.peak(ownership::Kind::Results) >= 4096);
        drop((pool, credits, gate, arrived, stop, scope));
        assert_eq!(audit.live_total(), 0);
    }
}
