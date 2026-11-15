//! The aircraft the full map's Fly mode flies.
//!
//! The host chooses it by name ([`MapAction::FlyAircraft`]); there is no
//! default. The choice bakes lazily, off the UI thread, from the open map's own
//! aircraft assets ([`AircraftAssets::player`]), so it flies from the content
//! stack the map draws. A map that loads again bakes the choice again from its
//! new content. Each change of state is reported to the host
//! ([`NativeEvent::FlyAircraft`]); Fly mode flies what [`FlyAircraft::ready`]
//! gives for the open map, and nothing while it gives nothing: there is no
//! default aircraft.
//!
//! [`MapAction::FlyAircraft`]: dashboard::protocol::MapAction::FlyAircraft
//! [`NativeEvent::FlyAircraft`]: dashboard::protocol::NativeEvent::FlyAircraft

use job_priority::SpawnCarried as _;
use dashboard::protocol::{FlyAircraftState, FlyAircraftStatus};
use std::sync::{Arc, mpsc};
use viewer_world::map::aircraft::{AircraftAssets, PlayerAircraft};

#[derive(Default)]
pub struct FlyAircraft {
    choice: Option<String>,
    bake: Option<Bake>,
    reported: Option<FlyAircraftStatus>,
}

/// The chosen name's bake for one map's assets.
struct Bake {
    assets: Arc<AircraftAssets>,
    name: String,
    outcome: Outcome,
}

enum Outcome {
    Waiting(mpsc::Receiver<Result<PlayerAircraft, String>>),
    Done(Result<PlayerAircraft, String>),
}

impl FlyAircraft {
    pub fn choose(&mut self, aircraft: Option<String>) {
        if self.choice != aircraft {
            self.choice = aircraft;
            self.bake = None;
        }
    }

    /// The chosen aircraft once it has baked for the map whose aircraft assets
    /// are `assets`: its slot in their [`AircraftAssets::library`]. `None`
    /// while nothing is chosen, while it loads, when it is refused, and for any
    /// other map's assets, so a slot never outlives the map it was baked for.
    pub fn ready(&self, assets: &Arc<AircraftAssets>) -> Option<&PlayerAircraft> {
        match &self.bake {
            Some(Bake {
                assets: baked,
                outcome: Outcome::Done(Ok(player)),
                ..
            }) if Arc::ptr_eq(baked, assets) => Some(player),
            _ => None,
        }
    }

    /// Why the chosen aircraft cannot be flown, once its bake refused it.
    pub fn refusal(&self) -> Option<String> {
        match &self.bake {
            Some(Bake {
                name,
                outcome: Outcome::Done(Err(reason)),
                ..
            }) => Some(format!("{name} cannot be flown: {reason}")),
            _ => None,
        }
    }

    /// Advances the choice for the open map, whose aircraft assets are
    /// `assets` (`None` while no map is open), and returns its status when it
    /// changed since the last one returned. A bake runs on its own thread and
    /// calls `repaint` when it is done.
    pub fn poll(
        &mut self,
        assets: Option<&Arc<AircraftAssets>>,
        repaint: impl Fn() + Send + 'static,
    ) -> Option<FlyAircraftStatus> {
        let state = match (self.choice.clone(), assets) {
            (None, _) => {
                self.bake = None;
                FlyAircraftState::Ready
            }
            (Some(_), None) => {
                self.bake = None;
                FlyAircraftState::Loading
            }
            (Some(name), Some(assets)) => {
                self.advance(name, assets, repaint);
                match self.bake.as_ref().map(|bake| &bake.outcome) {
                    _ if self.ready(assets).is_some() => FlyAircraftState::Ready,
                    Some(Outcome::Done(Err(reason))) => FlyAircraftState::Refused {
                        reason: reason.clone(),
                    },
                    _ => FlyAircraftState::Loading,
                }
            }
        };
        let status = FlyAircraftStatus {
            aircraft: self.choice.clone(),
            state,
        };
        (self.reported.as_ref() != Some(&status)).then(|| {
            self.reported = Some(status.clone());
            status
        })
    }

    /// Starts `name`'s bake for `assets` unless it is already theirs, and
    /// takes its outcome once the bake sent it.
    fn advance(
        &mut self,
        name: String,
        assets: &Arc<AircraftAssets>,
        repaint: impl Fn() + Send + 'static,
    ) {
        if !self
            .bake
            .as_ref()
            .is_some_and(|bake| Arc::ptr_eq(&bake.assets, assets) && bake.name == name)
        {
            self.bake = Some(Bake {
                assets: Arc::clone(assets),
                outcome: start(Arc::clone(assets), name.clone(), repaint),
                name,
            });
        }
        let bake = self.bake.as_mut().expect("the chosen aircraft is baking");
        if let Outcome::Waiting(receive) = &bake.outcome {
            match receive.try_recv() {
                Ok(outcome) => bake.outcome = Outcome::Done(outcome),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    bake.outcome = Outcome::Done(Err("loading the aircraft stopped".into()))
                }
            }
        }
    }
}

fn start(
    assets: Arc<AircraftAssets>,
    name: String,
    repaint: impl Fn() + Send + 'static,
) -> Outcome {
    let (send, receive) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("fly aircraft".into())
        .spawn_carried(move || {
            viewer_foundation::worker_priority::enter(viewer_foundation::worker_priority::Class::Normal);
            let _ = send.send(assets.player(&name));
            repaint();
        });
    match spawned {
        Ok(_) => Outcome::Waiting(receive),
        Err(error) => Outcome::Done(Err(format!(
            "the aircraft could not start loading: {error}"
        ))),
    }
}

#[cfg(test)]
mod tests;
