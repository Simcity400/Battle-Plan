
use super::entity_animation::{NativeSample, Rig};
use super::mesh_pose::Actor;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

pub fn tick(frame: u64) -> Option<i64> {
    Some(frame as i64)
}

pub type Key = ((usize, u32, u32), usize);

pub type Pass = (u64, u64);

struct Job {
    pass: Pass,
    key: Key,
    rig: Arc<Rig>,
    samples: Vec<Option<NativeSample>>,
}

#[derive(Default)]
pub struct Jobs {
    jobs: Vec<Job>,
    spare: Vec<Vec<Option<NativeSample>>>,
}

impl Jobs {
    pub(crate) fn push(
        &mut self,
        pass: Pass,
        key: Key,
        rig: &Arc<Rig>,
        samples: &[Option<NativeSample>],
    ) {
        let mut into = self.spare.pop().unwrap_or_default();
        into.clear();
        into.extend_from_slice(samples);
        self.jobs.push(Job { pass, key, rig: Arc::clone(rig), samples: into });
    }

    pub fn len(&self) -> usize {
        self.jobs.len()
    }
}

#[derive(Default)]
pub struct Set {
    poses: HashMap<Key, Arc<[Actor]>>,
    pub time: f32,
}

impl Set {
    pub fn get(&self, key: Key) -> Option<Arc<[Actor]>> {
        self.poses.get(&key).map(Arc::clone)
    }

    pub fn len(&self) -> usize {
        self.poses.len()
    }
}

pub fn judge(label: &'static str, set: &Set, animation_time: f32, interval: f32) {
    if viewer_scene_base::actor_check::every().is_none() || set.len() == 0 {
        return;
    }
    let allowed = 2.0 * interval.max(1.0 / 240.0);
    let age = animation_time - set.time;
    viewer_scene_base::actor_check::checked("poses", set.len() as u64);
    if !(0.0..=allowed).contains(&age) {
        viewer_scene_base::actor_check::differs(
            "poses",
            &label,
            &format!("a pose {age:.4} s old where its rate allows {allowed:.4} s"),
        );
    }
}

struct Batch {
    time: f32,
    jobs: Vec<Job>,
}

pub struct Made {
    sets: Vec<(Pass, Arc<Set>)>,
    spare: Vec<Vec<Option<NativeSample>>>,
}

impl Made {
    pub fn set_for(&self, pass: Pass) -> Arc<Set> {
        self.sets
            .iter()
            .find(|(at, _)| *at == pass)
            .map_or_else(Default::default, |(_, set)| Arc::clone(set))
    }

    pub fn made(&self) -> u64 {
        self.sets.iter().map(|(_, set)| set.len() as u64).sum()
    }
}

pub struct Worker {
    jobs: Sender<Batch>,
    made: Receiver<Made>,
    working: bool,
    refusal: Option<String>,
}

impl Worker {
    pub fn new() -> Self {
        let (jobs, batches) = std::sync::mpsc::channel();
        let (sets, made) = std::sync::mpsc::channel();
        let mut worker = Self { jobs, made, working: false, refusal: None };
        if let Err(error) = std::thread::Builder::new()
            .name("map poses".into())
            .spawn(move || run(&batches, &sets))
        {
            worker.refuse(&format!("the map poses thread could not start: {error}"));
        }
        worker
    }

    pub fn refusal(&self) -> Option<&str> {
        self.refusal.as_deref()
    }

    fn refuse(&mut self, why: &str) {
        if self.refusal.is_some() {
            return;
        }
        let why = format!("Map poses: {why}; every pose is made on the frame that draws it");
        eprintln!("{why}");
        self.refusal = Some(why);
    }

    pub fn free(&self) -> bool {
        self.refusal.is_none() && !self.working
    }

    pub fn give(&mut self, time: f32, jobs: &mut Jobs) {
        if jobs.jobs.is_empty() || !self.free() {
            return;
        }
        let batch = Batch { time, jobs: std::mem::take(&mut jobs.jobs) };
        if self.jobs.send(batch).is_err() {
            self.refuse("the map poses thread ended");
            return;
        }
        self.working = true;
    }

    pub fn take(&mut self, jobs: &mut Jobs) -> Option<Made> {
        match self.made.try_recv() {
            Ok(mut made) => {
                self.working = false;
                jobs.spare.append(&mut made.spare);
                Some(made)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.working = false;
                self.refuse("the map poses thread ended");
                None
            }
        }
    }
}

fn run(batches: &Receiver<Batch>, sets: &Sender<Made>) {
    while let Ok(batch) = batches.recv() {
        let mut sets_made: Vec<(Pass, Set)> = Vec::new();
        let mut spare = Vec::with_capacity(batch.jobs.len());
        for job in batch.jobs {
            if sets_made.last().map(|(pass, _)| *pass) != Some(job.pass) {
                sets_made.push((job.pass, Set { poses: HashMap::new(), time: batch.time }));
            }
            let set = &mut sets_made.last_mut().expect("pushed above").1;
            if let std::collections::hash_map::Entry::Vacant(slot) = set.poses.entry(job.key) {
                slot.insert(super::mesh_pose::actors_at(&job.rig, &job.samples).into());
            }
            spare.push(job.samples);
        }
        let made = Made {
            sets: sets_made
                .into_iter()
                .map(|(pass, set)| (pass, Arc::new(set)))
                .collect(),
            spare,
        };
        if sets.send(made).is_err() {
            return;
        }
    }
}
