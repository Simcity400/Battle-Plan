
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Open,
    Close { generation: Option<u64> },
    Destroyed(u64),
    Created { generation: u64, ok: bool },
    Started {
        generation: u64,
        ok: bool,
        restart: bool,
    },
    Released(u64),
    Resized {
        generation: u64,
        width: u32,
        height: u32,
    },
    Restart(u64),
    StatusChanged(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    CreateWindow(u64),
    StartViewer { generation: u64, restart: bool },
    Resize {
        generation: u64,
        width: u32,
        height: u32,
    },
    Raise,
    Release(u64),
    Publish { generation: u64, open: bool },
    PublishStatus(u64),
    AwaitOpen(u64),
    Opened(u64),
    OpenFailed(u64),
    RestartFailed(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Closed,
    Opening {
        cancelled: bool,
        reopen: bool,
    },
    Open,
    Closing {
        reopen: bool,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct Lifecycle {
    generation: u64,
    phase: Phase,
    exiting: bool,
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self {
            generation: 0,
            phase: Phase::Closed,
            exiting: false,
        }
    }
}

impl Lifecycle {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    #[cfg(test)]
    pub fn is_open(&self) -> bool {
        self.phase == Phase::Open
    }

    pub fn is_closed(&self) -> bool {
        self.phase == Phase::Closed
    }

    pub fn handle(&mut self, input: Input) -> Vec<Effect> {
        use {Effect as E, Input as I, Phase as P};
        let current = |generation: u64| generation == self.generation;
        let closes = |generation: Option<u64>| generation.is_none_or(current);
        let next = self.generation + 1;
        if input == (I::Close { generation: None }) {
            self.exiting = true;
        }
        if self.exiting && input == I::Open {
            return vec![E::AwaitOpen(next), E::OpenFailed(next)];
        }
        match (self.phase, input) {
            (P::Closed, I::Open) => {
                let mut effects = self.open();
                effects.push(E::AwaitOpen(self.generation));
                effects
            }
            (
                P::Opening {
                    cancelled: true, ..
                },
                I::Open,
            ) => {
                self.phase = P::Opening {
                    cancelled: true,
                    reopen: true,
                };
                vec![E::AwaitOpen(next)]
            }
            (
                P::Opening {
                    cancelled: false, ..
                },
                I::Open,
            ) => vec![E::AwaitOpen(self.generation)],
            (P::Open, I::Open) => vec![
                E::Raise,
                E::AwaitOpen(self.generation),
                E::Opened(self.generation),
            ],
            (P::Closing { .. }, I::Open) => {
                self.phase = P::Closing { reopen: true };
                vec![E::AwaitOpen(next)]
            }
            (P::Opening { reopen, .. }, I::Close { generation }) if closes(generation) => {
                self.phase = P::Opening {
                    cancelled: true,
                    reopen: false,
                };
                reopen.then_some(E::OpenFailed(next)).into_iter().collect()
            }
            (
                P::Opening {
                    cancelled: false, ..
                },
                I::Destroyed(generation),
            ) if current(generation) => {
                self.phase = P::Opening {
                    cancelled: true,
                    reopen: false,
                };
                Vec::new()
            }
            (P::Open, I::Close { generation }) if closes(generation) => self.release(),
            (P::Open, I::Destroyed(generation)) if current(generation) => self.release(),
            (P::Closing { reopen }, I::Close { generation }) if closes(generation) => {
                self.phase = P::Closing { reopen: false };
                reopen.then_some(E::OpenFailed(next)).into_iter().collect()
            }
            (P::Opening { cancelled, reopen }, I::Created { generation, ok })
                if current(generation) =>
            {
                match (ok, cancelled) {
                    (true, false) => {
                        self.phase = P::Open;
                        vec![E::StartViewer {
                            generation,
                            restart: false,
                        }]
                    }
                    (true, true) => {
                        self.phase = P::Closing { reopen };
                        vec![E::OpenFailed(generation), E::Release(generation)]
                    }
                    (false, _) => {
                        self.phase = P::Closed;
                        let mut effects = vec![E::OpenFailed(generation)];
                        if reopen {
                            effects.extend(self.open());
                        }
                        effects
                    }
                }
            }
            (
                P::Open,
                I::Started {
                    generation,
                    ok,
                    restart,
                },
            ) if current(generation) => match (ok, restart) {
                (true, _) => vec![
                    E::Publish {
                        generation,
                        open: true,
                    },
                    E::Opened(generation),
                ],
                (false, true) => vec![E::RestartFailed(generation)],
                (false, false) => {
                    let mut effects = vec![E::OpenFailed(generation)];
                    effects.extend(self.release());
                    effects
                }
            },
            (P::Closing { reopen }, I::Released(generation)) if current(generation) => {
                self.phase = P::Closed;
                let mut effects = vec![E::Publish {
                    generation,
                    open: false,
                }];
                if reopen {
                    effects.extend(self.open());
                }
                effects
            }
            (
                P::Open,
                I::Resized {
                    generation,
                    width,
                    height,
                },
            ) if current(generation) => vec![E::Resize {
                generation,
                width,
                height,
            }],
            (P::Open, I::StatusChanged(generation)) if current(generation) => {
                vec![E::PublishStatus(generation)]
            }
            (P::Open, I::Restart(generation)) if current(generation) => vec![E::StartViewer {
                generation,
                restart: true,
            }],
            (_, I::Restart(generation)) => vec![E::RestartFailed(generation)],
            _ => Vec::new(),
        }
    }

    fn open(&mut self) -> Vec<Effect> {
        self.generation += 1;
        self.phase = Phase::Opening {
            cancelled: false,
            reopen: false,
        };
        vec![Effect::CreateWindow(self.generation)]
    }

    fn release(&mut self) -> Vec<Effect> {
        self.phase = Phase::Closing { reopen: false };
        vec![Effect::Release(self.generation)]
    }
}

#[cfg(test)]
mod tests {
    use super::{Effect as E, Input as I, *};

    const CLOSE: Input = I::Close { generation: None };

    fn close(generation: u64) -> Input {
        I::Close {
            generation: Some(generation),
        }
    }

    fn run(lifecycle: &mut Lifecycle, inputs: &[Input]) -> Vec<Effect> {
        inputs
            .iter()
            .flat_map(|input| lifecycle.handle(*input))
            .collect()
    }

    fn created(generation: u64) -> Input {
        I::Created {
            generation,
            ok: true,
        }
    }

    fn started(generation: u64) -> Input {
        I::Started {
            generation,
            ok: true,
            restart: false,
        }
    }

    fn opened(lifecycle: &mut Lifecycle) -> u64 {
        run(lifecycle, &[I::Open]);
        let generation = lifecycle.generation();
        run(lifecycle, &[created(generation), started(generation)]);
        assert!(lifecycle.is_open());
        generation
    }

    #[test]
    fn opening_reserves_the_generation_before_the_window_exists() {
        let mut lifecycle = Lifecycle::default();
        assert_eq!(
            lifecycle.handle(I::Open),
            [E::CreateWindow(1), E::AwaitOpen(1)]
        );
        assert_eq!(
            lifecycle.handle(created(1)),
            [E::StartViewer {
                generation: 1,
                restart: false
            }]
        );
        assert_eq!(
            lifecycle.handle(started(1)),
            [
                E::Publish {
                    generation: 1,
                    open: true
                },
                E::Opened(1)
            ]
        );
        assert_eq!(
            lifecycle.handle(I::Open),
            [E::Raise, E::AwaitOpen(1), E::Opened(1)]
        );
    }

    #[test]
    fn a_close_during_opening_releases_the_new_window_instead_of_starting_it() {
        for close in [CLOSE, I::Destroyed(1)] {
            let mut lifecycle = Lifecycle::default();
            run(&mut lifecycle, &[I::Open, close]);
            let effects = run(&mut lifecycle, &[created(1), I::Released(1)]);
            assert_eq!(
                effects,
                [
                    E::OpenFailed(1),
                    E::Release(1),
                    E::Publish {
                        generation: 1,
                        open: false
                    }
                ]
            );
            assert!(!lifecycle.is_open());
        }
    }

    #[test]
    fn a_reopen_during_closing_opens_a_new_generation_once_the_old_is_released() {
        let mut lifecycle = Lifecycle::default();
        let old = opened(&mut lifecycle);
        assert_eq!(lifecycle.handle(close(old)), [E::Release(old)]);
        assert_eq!(lifecycle.handle(I::Open), [E::AwaitOpen(old + 1)]);
        assert!(lifecycle.handle(I::Destroyed(old)).is_empty());
        assert_eq!(
            lifecycle.handle(I::Released(old)),
            [
                E::Publish {
                    generation: old,
                    open: false
                },
                E::CreateWindow(old + 1)
            ]
        );
        assert!(lifecycle.handle(I::Destroyed(old)).is_empty());
        assert!(lifecycle.handle(I::StatusChanged(old)).is_empty());
        assert!(
            lifecycle
                .handle(I::Resized {
                    generation: old,
                    width: 1,
                    height: 1
                })
                .is_empty()
        );
        assert_eq!(
            run(&mut lifecycle, &[created(old + 1), started(old + 1)]),
            [
                E::StartViewer {
                    generation: old + 1,
                    restart: false
                },
                E::Publish {
                    generation: old + 1,
                    open: true
                },
                E::Opened(old + 1)
            ]
        );
    }

    #[test]
    fn open_close_open_before_creation_answers_each_request_by_its_own_window() {
        let mut lifecycle = Lifecycle::default();
        assert_eq!(
            run(&mut lifecycle, &[I::Open, close(1), I::Open]),
            [E::CreateWindow(1), E::AwaitOpen(1), E::AwaitOpen(2)]
        );
        assert_eq!(
            run(&mut lifecycle, &[created(1), I::Released(1)]),
            [
                E::OpenFailed(1),
                E::Release(1),
                E::Publish {
                    generation: 1,
                    open: false
                },
                E::CreateWindow(2)
            ]
        );
        assert_eq!(
            run(&mut lifecycle, &[created(2), started(2)]),
            [
                E::StartViewer {
                    generation: 2,
                    restart: false
                },
                E::Publish {
                    generation: 2,
                    open: true
                },
                E::Opened(2)
            ]
        );
    }

    #[test]
    fn a_reopen_closed_again_before_it_starts_fails_only_its_own_request() {
        let mut lifecycle = Lifecycle::default();
        run(&mut lifecycle, &[I::Open, close(1), I::Open]);
        assert_eq!(lifecycle.handle(close(1)), [E::OpenFailed(2)]);
        assert_eq!(
            run(&mut lifecycle, &[created(1), I::Released(1)]),
            [
                E::OpenFailed(1),
                E::Release(1),
                E::Publish {
                    generation: 1,
                    open: false
                }
            ]
        );
        let mut lifecycle = Lifecycle::default();
        let old = opened(&mut lifecycle);
        run(&mut lifecycle, &[close(old), I::Open]);
        assert_eq!(lifecycle.handle(close(old)), [E::OpenFailed(old + 1)]);
        assert_eq!(
            lifecycle.handle(I::Released(old)),
            [E::Publish {
                generation: old,
                open: false
            }]
        );
    }

    #[test]
    fn only_the_owner_publishes_the_viewers_status_and_only_while_its_window_is_open() {
        let mut lifecycle = Lifecycle::default();
        run(&mut lifecycle, &[I::Open]);
        assert!(lifecycle.handle(I::StatusChanged(1)).is_empty());
        run(&mut lifecycle, &[created(1), started(1)]);
        assert_eq!(lifecycle.handle(I::StatusChanged(1)), [E::PublishStatus(1)]);
        run(&mut lifecycle, &[CLOSE]);
        assert!(lifecycle.handle(I::StatusChanged(1)).is_empty());
    }

    #[test]
    fn a_viewer_crash_during_closing_only_finishes_the_close() {
        let mut lifecycle = Lifecycle::default();
        let generation = opened(&mut lifecycle);
        assert_eq!(lifecycle.handle(CLOSE), [E::Release(generation)]);
        assert!(lifecycle.handle(I::StatusChanged(generation)).is_empty());
        assert!(lifecycle.handle(I::Destroyed(generation)).is_empty());
        assert_eq!(
            lifecycle.handle(I::Restart(generation)),
            [E::RestartFailed(generation)]
        );
        assert_eq!(
            lifecycle.handle(I::Released(generation)),
            [E::Publish {
                generation,
                open: false
            }]
        );
        assert!(!lifecycle.is_open());
    }

    #[test]
    fn a_failed_start_closes_but_a_failed_restart_keeps_the_window() {
        let mut lifecycle = Lifecycle::default();
        run(&mut lifecycle, &[I::Open, created(1)]);
        assert_eq!(
            lifecycle.handle(I::Started {
                generation: 1,
                ok: false,
                restart: false
            }),
            [E::OpenFailed(1), E::Release(1)]
        );
        run(&mut lifecycle, &[I::Released(1)]);
        let generation = opened(&mut lifecycle);
        assert_eq!(
            lifecycle.handle(I::Restart(generation)),
            [E::StartViewer {
                generation,
                restart: true
            }]
        );
        assert_eq!(
            lifecycle.handle(I::Started {
                generation,
                ok: false,
                restart: true
            }),
            [E::RestartFailed(generation)]
        );
        assert!(lifecycle.is_open());
    }

    #[test]
    fn a_window_that_cannot_open_leaves_the_pop_out_closed() {
        let mut lifecycle = Lifecycle::default();
        run(&mut lifecycle, &[I::Open]);
        assert_eq!(
            lifecycle.handle(I::Created {
                generation: 1,
                ok: false
            }),
            [E::OpenFailed(1)]
        );
        assert!(!lifecycle.is_open());
        assert_eq!(
            lifecycle.handle(I::Open),
            [E::CreateWindow(2), E::AwaitOpen(2)]
        );
    }

    #[test]
    fn an_old_windows_destruction_never_cancels_the_reopen_queued_behind_it() {
        let mut lifecycle = Lifecycle::default();
        run(&mut lifecycle, &[I::Open, close(1), I::Open]);
        assert!(lifecycle.handle(I::Destroyed(1)).is_empty());
        assert_eq!(
            run(&mut lifecycle, &[created(1), I::Released(1)]),
            [
                E::OpenFailed(1),
                E::Release(1),
                E::Publish {
                    generation: 1,
                    open: false
                },
                E::CreateWindow(2)
            ]
        );
        assert_eq!(
            run(&mut lifecycle, &[created(2), started(2)]),
            [
                E::StartViewer {
                    generation: 2,
                    restart: false
                },
                E::Publish {
                    generation: 2,
                    open: true
                },
                E::Opened(2)
            ]
        );
    }

    #[test]
    fn a_close_before_a_failed_creation_leaves_the_pop_out_closed() {
        let mut lifecycle = Lifecycle::default();
        run(&mut lifecycle, &[I::Open, CLOSE]);
        assert_eq!(
            lifecycle.handle(I::Created {
                generation: 1,
                ok: false
            }),
            [E::OpenFailed(1)]
        );
        assert!(lifecycle.is_closed());
    }

    #[test]
    fn commands_naming_another_window_change_nothing() {
        let mut lifecycle = Lifecycle::default();
        let old = opened(&mut lifecycle);
        run(&mut lifecycle, &[close(old), I::Released(old)]);
        let current = opened(&mut lifecycle);
        assert!(
            lifecycle
                .handle(I::Close {
                    generation: Some(old)
                })
                .is_empty()
        );
        assert_eq!(lifecycle.handle(I::Restart(old)), [E::RestartFailed(old)]);
        assert!(lifecycle.is_open());
        assert_eq!(
            lifecycle.handle(I::Close {
                generation: Some(current)
            }),
            [E::Release(current)]
        );
    }

    #[test]
    fn a_cancelled_open_is_answered_before_its_release_starts_the_reopen() {
        let mut lifecycle = Lifecycle::default();
        run(&mut lifecycle, &[I::Open, close(1), I::Open]);
        let effects = lifecycle.handle(created(1));
        assert_eq!(effects, [E::OpenFailed(1), E::Release(1)]);
    }

    #[test]
    fn an_app_exit_cancels_a_queued_reopen_and_refuses_new_opens() {
        let mut lifecycle = Lifecycle::default();
        run(&mut lifecycle, &[I::Open, CLOSE]);
        assert_eq!(
            lifecycle.handle(I::Open),
            [E::AwaitOpen(2), E::OpenFailed(2)]
        );
        assert_eq!(
            run(&mut lifecycle, &[created(1), I::Released(1)]),
            [
                E::OpenFailed(1),
                E::Release(1),
                E::Publish {
                    generation: 1,
                    open: false
                }
            ]
        );
        assert!(lifecycle.is_closed());
        let mut lifecycle = Lifecycle::default();
        let old = opened(&mut lifecycle);
        run(
            &mut lifecycle,
            &[
                I::Close {
                    generation: Some(old),
                },
                I::Open,
            ],
        );
        assert_eq!(lifecycle.handle(CLOSE), [E::OpenFailed(old + 1)]);
        assert_eq!(
            lifecycle.handle(I::Released(old)),
            [E::Publish {
                generation: old,
                open: false
            }]
        );
        assert!(lifecycle.is_closed());
    }

    #[test]
    fn a_restart_failure_names_only_its_own_generation() {
        let mut lifecycle = Lifecycle::default();
        let current = opened(&mut lifecycle);
        assert_eq!(
            lifecycle.handle(I::Restart(current + 1)),
            [E::RestartFailed(current + 1)]
        );
        assert!(lifecycle.is_open());
    }
}
