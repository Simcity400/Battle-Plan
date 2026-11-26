use std::os::windows::io::{AsRawHandle, BorrowedHandle};
use std::sync::{
    OnceLock,
    atomic::{AtomicU64, Ordering},
};
use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadSelectedCpuSets};

const GROUP_PROCESSORS: usize = 64;

const BACKGROUND_CORES_MIN_PHYSICAL: usize = 6;
const BACKGROUND_CORES_WIDE_MIN_PHYSICAL: usize = 8;
const BACKGROUND_CORES_COUNT: usize = 1;
const BACKGROUND_CORES_WIDE_COUNT: usize = 2;

struct Cores {
    sets: [(u8, u8, u32); GROUP_PROCESSORS],
    len: usize,
    physical_cores: usize,
    others: [[u32; GROUP_PROCESSORS]; GROUP_PROCESSORS],
    others_len: [usize; GROUP_PROCESSORS],
    background: [[u32; GROUP_PROCESSORS]; GROUP_PROCESSORS],
    background_len: [usize; GROUP_PROCESSORS],
}
impl Cores {
    fn read() -> Option<Self> {
        use windows_sys::Win32::System::SystemInformation::{
            CpuSetInformation, GetSystemCpuSetInformation, SYSTEM_CPU_SET_INFORMATION,
        };
        let mut buffer = [0u64; 2048];
        let mut length = 0u32;
        if unsafe {
            GetSystemCpuSetInformation(
                buffer.as_mut_ptr().cast(),
                size_of_val(&buffer) as u32,
                &raw mut length,
                std::ptr::null_mut(),
                0,
            )
        } == 0
        {
            return None;
        }
        let mut entries = [(0, 0, 0); GROUP_PROCESSORS];
        let mut len = 0;
        let entry_size = size_of::<SYSTEM_CPU_SET_INFORMATION>();
        let bytes = buffer.as_ptr().cast::<u8>();
        let mut offset = 0usize;
        while offset + entry_size <= length as usize && len < GROUP_PROCESSORS {
            let entry = unsafe { bytes.add(offset).cast::<SYSTEM_CPU_SET_INFORMATION>().read_unaligned() };
            if (entry.Size as usize) < entry_size {
                break;
            }
            if entry.Type == CpuSetInformation {
                let set = unsafe { entry.Anonymous.CpuSet };
                if set.Group == 0 {
                    entries[len] = (set.LogicalProcessorIndex, set.CoreIndex, set.Id);
                    len += 1;
                }
            }
            offset += entry.Size as usize;
        }
        Self::from_sets(&entries[..len])
    }

    fn from_sets(entries: &[(u8, u8, u32)]) -> Option<Self> {
        let mut cores = Self {
            sets: [(0, 0, 0); GROUP_PROCESSORS],
            len: 0,
            physical_cores: 0,
            others: [[0; GROUP_PROCESSORS]; GROUP_PROCESSORS],
            others_len: [0; GROUP_PROCESSORS],
            background: [[0; GROUP_PROCESSORS]; GROUP_PROCESSORS],
            background_len: [0; GROUP_PROCESSORS],
        };
        let mut seen = [false; GROUP_PROCESSORS];
        let mut unique = [0usize; GROUP_PROCESSORS];
        let placed = entries
            .iter()
            .filter(|&&(_, core, _)| usize::from(core) < GROUP_PROCESSORS);
        for &entry in placed.take(GROUP_PROCESSORS) {
            let core = usize::from(entry.1);
            if !seen[core] {
                seen[core] = true;
                unique[cores.physical_cores] = core;
                cores.physical_cores += 1;
            }
            cores.sets[cores.len] = entry;
            cores.len += 1;
        }
        let unique = &mut unique[..cores.physical_cores];
        unique.sort_unstable_by(|a, b| b.cmp(a));
        for core in 0..GROUP_PROCESSORS {
            let mut chosen = [false; GROUP_PROCESSORS];
            let mut picked = 0;
            if cores.physical_cores >= BACKGROUND_CORES_MIN_PHYSICAL {
                let limit = if cores.physical_cores >= BACKGROUND_CORES_WIDE_MIN_PHYSICAL {
                    BACKGROUND_CORES_WIDE_COUNT
                } else {
                    BACKGROUND_CORES_COUNT
                };
                for &candidate in unique.iter() {
                    if candidate != core {
                        chosen[candidate] = true;
                        picked += 1;
                        if picked == limit {
                            break;
                        }
                    }
                }
            }
            for &(_, other, id) in &cores.sets[..cores.len] {
                let other = usize::from(other);
                if other != core {
                    cores.others[core][cores.others_len[core]] = id;
                    cores.others_len[core] += 1;
                    if picked == 0 || chosen[other] {
                        cores.background[core][cores.background_len[core]] = id;
                        cores.background_len[core] += 1;
                    }
                }
            }
        }
        (cores.len != 0).then_some(cores)
    }

    fn away_from(&self, processor: u32) -> Option<&[u32]> {
        let &(_, core, _) = self.sets[..self.len]
            .iter()
            .find(|&&(logical, _, _)| u32::from(logical) == processor)?;
        let core = usize::from(core);
        Some(&self.others[core][..self.others_len[core]]).filter(|ids| !ids.is_empty())
    }

    fn background_away_from(&self, processor: u32) -> Option<&[u32]> {
        let &(_, core, _) = self.sets[..self.len]
            .iter()
            .find(|&&(logical, _, _)| u32::from(logical) == processor)?;
        let core = usize::from(core);
        Some(&self.background[core][..self.background_len[core]]).filter(|ids| !ids.is_empty())
    }
}

static CORES: OnceLock<Option<Cores>> = OnceLock::new();
static AWAY_FAILURES: AtomicU64 = AtomicU64::new(0);

fn away_sets(cores: Option<&Cores>, processor: Option<u32>) -> Option<&[u32]> {
    cores?.away_from(processor?)
}

fn background_sets(cores: Option<&Cores>, processor: Option<u32>) -> Option<&[u32]> {
    cores?.background_away_from(processor?)
}

fn cores() -> Option<&'static Cores> {
    CORES.get_or_init(Cores::read).as_ref()
}

pub fn prepare() {
    let _ = cores();
}

pub fn physical_cores() -> Option<usize> {
    cores().map(|cores| cores.physical_cores)
}

pub(crate) fn away_from_game(processor: Option<u32>) -> Option<&'static [u32]> {
    let processor = processor?;
    away_sets(cores(), Some(processor))
}

pub fn away_covers(processor: Option<u32>, threads: usize) -> bool {
    away_from_game(processor).is_some_and(|sets| sets.len() >= threads)
}

pub(crate) fn background_away_from_game(processor: Option<u32>) -> Option<&'static [u32]> {
    let processor = processor?;
    background_sets(cores(), Some(processor))
}

/// # Safety
/// The caller must own the target thread's scheduling policy and keep its handle live. Change only
/// collector worker affinity as part of that session's policy.
pub unsafe fn select(thread: BorrowedHandle<'_>, sets: Option<&[u32]>) -> bool {
    let (list, count) = sets.map_or((std::ptr::null(), 0), |sets| {
        (sets.as_ptr(), sets.len() as u32)
    });
    // SAFETY: callers name a live collector-owned thread; a null, empty list clears.
    unsafe { SetThreadSelectedCpuSets(thread.as_raw_handle(), list, count) != 0 }
}

/// # Safety
/// The caller must own the target thread's scheduling policy and keep its handle live. Change only
/// collector worker affinity as part of that session's policy.
pub unsafe fn select_away(thread: BorrowedHandle<'_>, sets: Option<&[u32]>) -> bool {
    // SAFETY: The caller owns this collector worker thread's scheduling policy and handle.
    let selected = unsafe { select(thread, sets) };
    if !selected {
        AWAY_FAILURES.fetch_add(1, Ordering::Relaxed);
    }
    selected
}

pub fn away_failures() -> u64 {
    AWAY_FAILURES.load(Ordering::Relaxed)
}

#[derive(Default)]
pub struct AwayFromGame {
    wanted: Option<u32>,
    applied: Option<&'static [u32]>,
}
impl AwayFromGame {
    /// # Safety
    /// The caller must own the current thread's scheduling policy and confine only a designated
    /// collector worker or capture thread as part of that session's policy.
    pub unsafe fn follow(&mut self, processor: Option<u32>) {
        if processor == self.wanted {
            return;
        }
        self.wanted = processor;
        let sets = away_from_game(processor);
        if sets.map(<[u32]>::as_ptr) == self.applied.map(<[u32]>::as_ptr) {
            return;
        }
        // SAFETY: The caller owns the current collector thread's scheduling policy, and the
        // pseudo-handle from GetCurrentThread() is valid for this call.
        if unsafe { select_away(BorrowedHandle::borrow_raw(GetCurrentThread()), sets) } {
            self.applied = sets;
        }
    }

    pub fn confined(&self) -> bool {
        self.applied.is_some()
    }
}

#[cfg(test)]
#[test]
fn a_thread_keeps_off_the_whole_physical_core_of_the_game_processor() {
    let cores = Cores::from_sets(&[(0, 0, 256), (1, 0, 257), (2, 1, 258), (3, 1, 259)]).unwrap();
    assert_eq!(cores.physical_cores, 2);
    assert_eq!(away_sets(Some(&cores), Some(0)), Some(&[258, 259][..]));
    assert_eq!(away_sets(Some(&cores), Some(1)), Some(&[258, 259][..]));
    assert_eq!(away_sets(Some(&cores), Some(3)), Some(&[256, 257][..]));
    assert_eq!(away_sets(Some(&cores), Some(4)), None);
    assert_eq!(away_sets(Some(&cores), None), None);
    assert_eq!(away_sets(None, Some(0)), None);
    let single = Cores::from_sets(&[(0, 0, 256), (1, 0, 257)]).unwrap();
    assert_eq!(single.physical_cores, 1);
    assert_eq!(away_sets(Some(&single), Some(1)), None);
    assert!(Cores::from_sets(&[]).is_none());
    let six = Cores::from_sets(&[
        (0, 0, 256),
        (1, 0, 257),
        (2, 1, 258),
        (3, 1, 259),
        (4, 2, 260),
        (5, 2, 261),
        (6, 3, 262),
        (7, 3, 263),
        (8, 4, 264),
        (9, 4, 265),
        (10, 5, 266),
        (11, 5, 267),
    ])
    .unwrap();
    assert_eq!(six.physical_cores, 6);
    assert_eq!(
        background_sets(Some(&six), Some(0)),
        Some(&[266, 267][..])
    );
    assert_eq!(
        background_sets(Some(&six), Some(10)),
        Some(&[264, 265][..])
    );
    assert_eq!(
        background_sets(Some(&cores), Some(0)),
        Some(&[258, 259][..])
    );
    let eight = Cores::from_sets(&[
        (0, 0, 256),
        (1, 0, 257),
        (2, 1, 258),
        (3, 1, 259),
        (4, 2, 260),
        (5, 2, 261),
        (6, 3, 262),
        (7, 3, 263),
        (8, 4, 264),
        (9, 4, 265),
        (10, 5, 266),
        (11, 5, 267),
        (12, 6, 268),
        (13, 6, 269),
        (14, 7, 270),
        (15, 7, 271),
    ])
    .unwrap();
    assert_eq!(eight.physical_cores, 8);
    assert_eq!(
        background_sets(Some(&eight), Some(0)),
        Some(&[268, 269, 270, 271][..])
    );
}
