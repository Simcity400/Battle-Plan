
use job_priority::SpawnCarried as _;
use dashboard_core::protocol::{FlyAircraftState, FlyAircraftStatus};
use std::sync::{Arc, mpsc};
use viewer_aircraft::aircraft::{AircraftAssets, PlayerAircraft};

#[derive(Default)]
pub struct FlyAircraft {
    choice: Option<String>,
    bake: Option<Bake>,
    reported: Option<FlyAircraftStatus>,
}

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
    pub fn choice(&self) -> Option<&str> {
        self.choice.as_deref()
    }

    pub fn choose(&mut self, aircraft: Option<String>) {
        if self.choice != aircraft {
            self.choice = aircraft;
            self.bake = None;
        }
    }

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
                outcome: start(
                    {
                        let (assets, name) = (Arc::clone(assets), name.clone());
                        move || assets.player(&name)
                    },
                    repaint,
                ),
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
    bake: impl FnOnce() -> Result<PlayerAircraft, String> + Send + 'static,
    repaint: impl Fn() + Send + 'static,
) -> Outcome {
    let (send, receive) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("fly aircraft".into())
        .spawn_carried(move || {
            let finish = Finish {
                send: Some(send),
                repaint,
            };
            viewer_runtime::worker_priority::enter(viewer_runtime::worker_priority::Class::Normal);
            let outcome = bake();
            if let Some(send) = &finish.send {
                let _ = send.send(outcome);
            }
        });
    match spawned {
        Ok(_) => Outcome::Waiting(receive),
        Err(error) => Outcome::Done(Err(format!(
            "the aircraft could not start loading: {error}"
        ))),
    }
}

struct Finish<F: Fn()> {
    send: Option<mpsc::Sender<Result<PlayerAircraft, String>>>,
    repaint: F,
}

impl<F: Fn()> Drop for Finish<F> {
    fn drop(&mut self) {
        self.send.take();
        (self.repaint)();
    }
}

#[cfg(test)]
pub(super) mod tests;
