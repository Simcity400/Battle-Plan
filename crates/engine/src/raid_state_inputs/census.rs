use super::*;

/// The native IDs of every state the state array holds (a state's ID is its
/// slot), into `output`: the owners the sliced state tables are scanned
/// over. One guarded copy per 512 slots; a state is not authenticated here,
/// the walk does that when it reads one.
pub fn state_census(
    memory: &impl Memory,
    game_state: usize,
    output: &mut Vec<u32>,
) -> Result<(), &'static str> {
    output.clear();
    let root = address(game_state, cgame_state::STATES)
        .ok()
        .and_then(|at| Array::read(memory, at, 8))
        .filter(|array| array.count >= 2)
        .ok_or("state array unreadable or invalid")?;
    let mut bytes = [0_u8; 512 * 8];
    let mut first = 1;
    while first < root.count {
        let count = ((root.count - first) as usize).min(512);
        let start = root.element(first, 8).ok_or("state pointers unreadable")?;
        if !memory.copy(start, &mut bytes[..count * 8]) {
            return Err("state pointers unreadable");
        }
        for index in 0..count {
            if u64_at(&bytes, index * 8) != 0 {
                output.push(first + index as u32);
            }
        }
        first += count as u32;
    }
    Ok(())
}

/// One state's core countries (`CState + 0xB0`, count `+0xBC`): the raw
/// country references `CIsCoreOfTrigger::Evaluate@140463740` compares, in
/// stored order, at `links[start..start + len]`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CoreRow {
    pub state: u32,
    pub start: usize,
    pub len: usize,
}

/// The core lists of the census states `selected` accepts, one row per state
/// (an empty list included), in census order. Each state is authenticated
/// (vtable and ID) with its list header in one record batch of 64; each
/// nonempty list is one guarded copy. A state whose list does not fit in
/// `links` has no row and is counted; `required` is the links every
/// selected state needs.
#[allow(clippy::too_many_arguments)]
pub fn state_cores(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    census: &[u32],
    selected: &dyn Fn(u32) -> bool,
    rows: &mut [CoreRow],
    links: &mut [u32],
    required: &mut usize,
) -> Outcome {
    const RECORD: usize = 32;
    const FIELDS: [RecordField; 3] = [
        field(0, 0, 8),
        field(cstate::ID, 8, 4),
        field(cstate::CORES, 16, 16),
    ];
    const LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&FIELDS, RECORD).expect("valid record layout");
    *required = 0;
    let result = (|| {
        let root = address(game_state, cgame_state::STATES)
            .ok()
            .and_then(|at| Array::read(memory, at, 8))
            .ok_or("state array unreadable or invalid")?;
        let vtable = address(base, cstate::VTABLE_RVA)? as u64;
        let (mut len, mut used, mut unread) = (0, 0, 0_u32);
        let mut ids = [0_u32; BATCH];
        let mut pointers = [0_usize; BATCH];
        let mut copied = [0_u8; BATCH * RECORD];
        let mut chosen = census.iter().copied().filter(|&state| selected(state));
        loop {
            let mut count = 0;
            for state in chosen.by_ref().take(BATCH) {
                ids[count] = state;
                count += 1;
            }
            if count == 0 {
                break;
            }
            let mut words = [0_u8; 8];
            for index in 0..count {
                let at = root
                    .element(ids[index], 8)
                    .ok_or("state pointer unreadable")?;
                if !memory.copy(at, &mut words) {
                    return Err("state pointer unreadable");
                }
                pointers[index] = usize::try_from(u64::from_le_bytes(words))
                    .ok()
                    .filter(|&pointer| pointer != 0)
                    .ok_or("state pointer null")?;
            }
            records(
                memory,
                &pointers[..count],
                &LAYOUT,
                &mut copied,
                "state core list unreadable",
            )?;
            for index in 0..count {
                let record = &copied[index * RECORD..(index + 1) * RECORD];
                if u64_at(record, 0) != vtable || u32_at(record, 8) != ids[index] {
                    return Err("state record disagrees with its census slot");
                }
                let cores =
                    Array::from_header(header16(record, 16), 4).ok_or("state core list invalid")?;
                let count = cores.count as usize;
                *required += count;
                let (Some(row), Some(slot)) =
                    (rows.get_mut(len), links.get_mut(used..used + count))
                else {
                    unread += 1;
                    continue;
                };
                if count != 0 {
                    let mut bytes = [0_u8; 4 * 64];
                    for (chunk, out) in slot.chunks_mut(64).enumerate() {
                        let bytes = &mut bytes[..out.len() * 4];
                        let copied = cores
                            .element((chunk * 64) as u32, 4)
                            .is_some_and(|at| memory.copy(at, bytes));
                        if !copied {
                            return Err("state core list unreadable");
                        }
                        for (link, word) in out.iter_mut().zip(bytes.chunks_exact(4)) {
                            *link = u32::from_le_bytes(word.try_into().unwrap());
                        }
                    }
                }
                *row = CoreRow {
                    state: ids[index],
                    start: used,
                    len: count,
                };
                len += 1;
                used += count;
            }
        }
        Ok(if unread == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: unread,
            }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}
