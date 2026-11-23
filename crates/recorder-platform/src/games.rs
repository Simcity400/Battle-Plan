use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessState {
    Running,
    Frozen,
    Exiting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameProcess {
    pub pid: u32,
    pub start_time: u64,
    pub state: ProcessState,
}

#[derive(Clone, Copy, Debug)]
pub struct Thread {
    pub state: u32,
    pub wait_reason: u32,
}

const WAITING: u32 = 5;
const SUSPENDED: u32 = 5;
const WR_SUSPENDED: u32 = 12;

pub fn classify(threads: &[Thread], deleting: bool, signalled: bool) -> ProcessState {
    if deleting || signalled {
        ProcessState::Exiting
    } else if threads.iter().all(|thread| {
        thread.state == WAITING && matches!(thread.wait_reason, SUSPENDED | WR_SUSPENDED)
    }) {
        ProcessState::Frozen
    } else {
        ProcessState::Running
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Discovery {
    None,
    One(GameProcess),
    Several(SeveralGames),
}

#[derive(Debug, PartialEq, Eq)]
pub struct SeveralGames(pub Vec<GameProcess>);

pub fn choose_game(found: &[GameProcess]) -> Discovery {
    let mut running = found
        .iter()
        .filter(|game| game.state == ProcessState::Running)
        .copied();
    match (running.next(), running.next()) {
        (None, _) => Discovery::None,
        (Some(one), None) => Discovery::One(one),
        (Some(first), Some(second)) => Discovery::Several(SeveralGames(
            [first, second].into_iter().chain(running).collect(),
        )),
    }
}

fn named(games: &[GameProcess], clock: impl Fn(u64) -> Option<(u16, u16)>) -> String {
    let each: Vec<String> = games
        .iter()
        .map(|game| match clock(game.start_time) {
            Some((hour, minute)) => format!("PID {}, started {hour:02}:{minute:02}", game.pid),
            None => format!("PID {}, start time unknown", game.pid),
        })
        .collect();
    match each.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

impl SeveralGames {
    pub fn text(&self, clock: impl Fn(u64) -> Option<(u16, u16)>) -> String {
        let (count, close) = match self.0.len() {
            2 => ("Two".to_owned(), "Close one"),
            count => (count.to_string(), "Close all but one"),
        };
        format!(
            "{count} games are running: {}. {close} to record.",
            named(&self.0, clock)
        )
    }
}

impl fmt::Display for SeveralGames {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text(crate::windows::local_clock))
    }
}

#[derive(Default)]
pub struct OtherGames(Vec<u32>);

impl OtherGames {
    pub fn notice(
        &mut self,
        chosen: u32,
        found: &[GameProcess],
        clock: impl Fn(u64) -> Option<(u16, u16)>,
    ) -> Option<String> {
        let mut others: Vec<GameProcess> = found
            .iter()
            .filter(|game| game.state == ProcessState::Running && game.pid != chosen)
            .copied()
            .collect();
        others.sort_by_key(|game| game.pid);
        let pids: Vec<u32> = others.iter().map(|game| game.pid).collect();
        if pids == self.0 {
            return None;
        }
        self.0 = pids;
        match others.len() {
            0 => None,
            1 => Some(format!(
                "Another game is running and will not be recorded: {}.",
                named(&others, clock)
            )),
            _ => Some(format!(
                "Other games are running and will not be recorded: {}.",
                named(&others, clock)
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ProcessState::*;

    const INITIALIZED: u32 = 0;
    const READY: u32 = 1;
    const USER_REQUEST: u32 = 6;

    fn game(pid: u32, start_time: u64, state: ProcessState) -> GameProcess {
        GameProcess {
            pid,
            start_time,
            state,
        }
    }

    fn clock(start_time: u64) -> Option<(u16, u16)> {
        Some(((start_time / 100) as u16, (start_time % 100) as u16))
    }

    #[test]
    fn one_running_game_is_chosen_beside_frozen_and_exiting_ones() {
        let running = game(58524, 2216, Running);
        for other in [game(75600, 2046, Frozen), game(75600, 2046, Exiting)] {
            assert_eq!(choose_game(&[other, running]), Discovery::One(running));
            assert_eq!(choose_game(&[running, other]), Discovery::One(running));
        }
        assert_eq!(
            choose_game(&[game(1, 1, Frozen), running, game(2, 2, Exiting)]),
            Discovery::One(running)
        );
    }

    #[test]
    fn no_running_game_is_none() {
        assert_eq!(choose_game(&[]), Discovery::None);
        assert_eq!(choose_game(&[game(75600, 2046, Frozen)]), Discovery::None);
        assert_eq!(choose_game(&[game(75600, 2046, Exiting)]), Discovery::None);
    }

    #[test]
    fn two_running_games_are_several_and_named_by_pid_and_local_start() {
        let first = game(75600, 2046, Running);
        let second = game(58524, 2216, Running);
        let Discovery::Several(several) = choose_game(&[first, game(1, 1, Frozen), second]) else {
            panic!("two running games must not be chosen between");
        };
        assert_eq!(several.0, [first, second]);
        assert_eq!(
            several.text(clock),
            "Two games are running: PID 75600, started 20:46, and PID 58524, started 22:16. Close one to record."
        );
        let third = game(9, 905, Running);
        assert_eq!(
            SeveralGames(vec![first, second, third]).text(|_| None),
            "3 games are running: PID 75600, start time unknown, PID 58524, start time unknown, and PID 9, start time unknown. Close all but one to record."
        );
    }

    #[test]
    fn another_live_game_is_named_once_and_frozen_ones_never() {
        let mut others = OtherGames::default();
        let chosen = game(58524, 2216, Running);
        assert_eq!(
            others.notice(chosen.pid, &[chosen, game(75600, 2046, Frozen)], clock),
            None
        );
        let live = game(7, 2301, Running);
        assert_eq!(
            others.notice(chosen.pid, &[chosen, live], clock).as_deref(),
            Some("Another game is running and will not be recorded: PID 7, started 23:01.")
        );
        assert_eq!(others.notice(chosen.pid, &[live, chosen], clock), None);
        assert_eq!(others.notice(chosen.pid, &[chosen], clock), None);
        assert!(others.notice(chosen.pid, &[chosen, live], clock).is_some());
    }

    #[test]
    fn a_process_is_classified_from_its_threads_and_exit() {
        let suspended = Thread {
            state: WAITING,
            wait_reason: SUSPENDED,
        };
        let wr_suspended = Thread {
            state: WAITING,
            wait_reason: WR_SUSPENDED,
        };
        let initialized = Thread {
            state: INITIALIZED,
            wait_reason: 0,
        };
        let ready = Thread {
            state: READY,
            wait_reason: 0,
        };
        let resumed = Thread {
            state: READY,
            wait_reason: SUSPENDED,
        };
        let blocked = Thread {
            state: WAITING,
            wait_reason: USER_REQUEST,
        };
        assert_eq!(classify(&[suspended, wr_suspended], false, false), Frozen);
        assert_eq!(classify(&[], false, false), Frozen);
        assert_eq!(classify(&[initialized], false, false), Running);
        assert_eq!(classify(&[resumed], false, false), Running);
        assert_eq!(classify(&[suspended, ready], false, false), Running);
        assert_eq!(classify(&[blocked], false, false), Running);
        assert_eq!(classify(&[ready], true, false), Exiting);
        assert_eq!(classify(&[suspended], false, true), Exiting);
    }
}
