//! Skeletal poses made away from the display frame.
//!
//! A figure's limbs are a function of its rig and the samples its player left, and nothing else
//! ([`crate::map::mesh_pose::actors_at`]). So a persistent worker assembles every live actor's
//! pose from those two, at the display's rate, and the display frame takes a finished set by a
//! swap. A frame never waits for the worker: a set that is not ready leaves the last one
//! standing, and an actor a set does not hold is posed where it is read, which is what a figure
//! an hour has just brought costs.

use super::entity_animation::{NativeSample, Rig};
use super::mesh_pose::Actor;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

/// The tick a display frame poses in: its own frame, so every display frame makes its own poses
/// and a frame that draws the same one again draws the limbs standing.
pub(crate) fn tick(frame: u64) -> Option<i64> {
    Some(frame as i64)
}

/// An actor's pose: the player keyed `(mesh, seed, identity)` read with the rig of `rig_slot`.
pub(crate) type Key = ((usize, u32, u32), usize);

/// Which pass's poses a job and a finished set are: the pass among the scene's, and the animation
/// history its samples were read from. A history that starts again takes a new generation
/// ([`crate::map::models::ModelPass::reset_animation`]), so a batch in flight for the history that
/// ended is never installed over the new one's figures.
pub(crate) type Pass = (u64, u64);

/// One actor's pose to make: everything the pose is a function of.
struct Job {
    /// The pass the actor belongs to ([`crate::map::models::ModelPass::pose_pass`]).
    pass: Pass,
    key: Key,
    rig: Arc<Rig>,
    samples: Vec<Option<NativeSample>>,
}

/// The batch a frame fills, with the sample buffers of the batch before it to fill again: a
/// pose asked for on every display frame must not cost an allocation a figure a frame.
#[derive(Default)]
pub(crate) struct Jobs {
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

    pub(crate) fn len(&self) -> usize {
        self.jobs.len()
    }
}

/// One pass's poses, as the worker left them.
#[derive(Default)]
pub(crate) struct Set {
    poses: HashMap<Key, Arc<[Actor]>>,
    /// The presentation clock the samples were taken at.
    pub time: f32,
}

impl Set {
    pub(crate) fn get(&self, key: Key) -> Option<Arc<[Actor]>> {
        self.poses.get(&key).map(Arc::clone)
    }

    pub(crate) fn len(&self) -> usize {
        self.poses.len()
    }
}

/// The bound judge of the poses: a pose a frame draws is no older than one display frame (the
/// set is of the frame that asked for it) plus the interval a worker that misses a frame costs.
/// An empty set is not judged: nothing was taken from it, so every pose was made where it was
/// read. Names the pass and the age it found, and does not stop the frame.
pub(crate) fn judge(label: &'static str, set: &Set, animation_time: f32, interval: f32) {
    if crate::map::actor_check::every().is_none() || set.len() == 0 {
        return;
    }
    // A clock that stood still leaves no room at all, so the bound is never below one frame of
    // a 240 Hz display.
    let allowed = 2.0 * interval.max(1.0 / 240.0);
    let age = animation_time - set.time;
    crate::map::actor_check::checked("poses", set.len() as u64);
    if !(0.0..=allowed).contains(&age) {
        crate::map::actor_check::differs(
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

/// The sets of one batch: one for each pass that had an actor in it, with the batch's sample
/// buffers for the next one to fill.
pub(crate) struct Made {
    sets: Vec<(Pass, Arc<Set>)>,
    spare: Vec<Vec<Option<NativeSample>>>,
}

impl Made {
    /// The set this batch holds for `pass`, or an empty one: a pass it holds none for had no
    /// actor with a play in the batch, and a set made for an animation history that has started
    /// again is not this pass's at all, so its limbs are never drawn over the new history's.
    pub(crate) fn set_for(&self, pass: Pass) -> Arc<Set> {
        self.sets
            .iter()
            .find(|(at, _)| *at == pass)
            .map_or_else(Default::default, |(_, set)| Arc::clone(set))
    }

    /// How many poses the batch made, over every pass.
    pub(crate) fn made(&self) -> u64 {
        self.sets.iter().map(|(_, set)| set.len() as u64).sum()
    }
}

/// The worker and the batch it holds. The worker is given one batch at a time, so a machine
/// that cannot make poses as fast as they are asked for makes them as fast as it can and the
/// frames in between draw the last set.
pub(crate) struct Worker {
    jobs: Sender<Batch>,
    made: Receiver<Made>,
    /// Whether a batch is in the worker.
    working: bool,
    /// Why no worker runs, for the one line that names it.
    refusal: Option<String>,
}

impl Worker {
    pub(crate) fn new() -> Self {
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

    /// Why no worker runs; `None` while one does.
    pub(crate) fn refusal(&self) -> Option<&str> {
        self.refusal.as_deref()
    }

    /// Names, once, why no worker runs. Every pose is then made where it is read, as it was
    /// before the worker existed: the picture is the same and the frame pays for it.
    fn refuse(&mut self, why: &str) {
        if self.refusal.is_some() {
            return;
        }
        let why = format!("Map poses: {why}; every pose is made on the frame that draws it");
        eprintln!("{why}");
        self.refusal = Some(why);
    }

    /// Whether the worker is free to take a batch.
    pub(crate) fn free(&self) -> bool {
        self.refusal.is_none() && !self.working
    }

    /// Hands the worker the batch `jobs` holds, leaving it its spare buffers. A worker that has
    /// gone leaves every pose to the frame.
    pub(crate) fn give(&mut self, time: f32, jobs: &mut Jobs) {
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

    /// The newest finished sets, giving `jobs` the batch's buffers back, or `None` while the
    /// worker is still making them.
    pub(crate) fn take(&mut self, jobs: &mut Jobs) -> Option<Made> {
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
            // An actor placed many times is asked for many times and posed once.
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
