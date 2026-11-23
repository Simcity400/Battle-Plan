use super::*;

const SUNK_MASK: u32 = 0x01;

#[derive(Debug, Default)]
pub struct SunkJob {
    pub log: RawLog<SUNK_N>,
    pub game_state: usize,
    pub hour: i32,
    pub(super) active: bool,
}

impl SunkJob {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn fixture() -> Self {
        Self {
            log: RawLog::new(1 << 13, 2 << 20),
            game_state: 0,
            hour: 0,
            active: false,
        }
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.hour = 0;
        self.active = false;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    pub fn grow(&mut self) {
        self.log.grow();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SunkCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; SUNK_N],
}

#[derive(Debug, Clone, Copy)]
pub struct SunkCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
}

fn sunk_pool_u64(log: &RawLog<SUNK_N>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn kspan(
    log: &mut RawLog<SUNK_N>,
    memory: &impl engine_native_memory::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

fn copy_sunk_class(
    log: &mut RawLog<SUNK_N>,
    memory: &impl engine_native_memory::Memory,
    base: usize,
    pointer: u64,
) -> bool {
    let address = match usize::try_from(pointer)
        .ok()
        .filter(|&pointer| pointer != 0)
    {
        Some(address) => address,
        None => return false,
    };
    let header = match kspan(
        log,
        memory,
        Some(address),
        csub_unit_definition::NAME + msvc_string::STRIDE,
        SUNK_MASK,
    ) {
        Some(start) => start,
        None => return false,
    };
    let vtable = sunk_pool_u64(log, header);
    let expected = match engine_sites::at(base, csub_unit_definition::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    let expected_null = match engine_sites::at(base, csub_unit_definition::NULL_VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    if vtable != expected && vtable != expected_null {
        return false;
    }
    copy_sunk_string(
        log,
        memory,
        address + csub_unit_definition::NAME,
        header + csub_unit_definition::NAME,
    )
}

fn copy_sunk_string(
    log: &mut RawLog<SUNK_N>,
    memory: &impl engine_native_memory::Memory,
    address: usize,
    head: usize,
) -> bool {
    let mut header = [0u8; msvc_string::STRIDE];
    header.copy_from_slice(&log.bytes[head..head + msvc_string::STRIDE]);
    let len = u64::from_le_bytes(
        header[msvc_string::LENGTH..msvc_string::LENGTH + 8]
            .try_into()
            .unwrap(),
    );
    let capacity = u64::from_le_bytes(
        header[msvc_string::CAPACITY..msvc_string::CAPACITY + 8]
            .try_into()
            .unwrap(),
    );
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return false;
    }
    let len = match usize::try_from(len) {
        Ok(len) => len,
        Err(_) => return false,
    };
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        address
    } else {
        match usize::try_from(u64::from_le_bytes(
            header[msvc_string::DATA..msvc_string::DATA + 8]
                .try_into()
                .unwrap(),
        ))
        .ok()
        .filter(|&source| source != 0)
        {
            Some(source) => source,
            None => return false,
        }
    };
    let terminator = match source.checked_add(len) {
        Some(terminator) => terminator,
        None => return false,
    };
    let term_start = match kspan(log, memory, Some(terminator), 1, SUNK_MASK) {
        Some(start) => start,
        None => return false,
    };
    if log.bytes[term_start] != 0 {
        return false;
    }
    if len != 0 && kspan(log, memory, Some(source), len, SUNK_MASK).is_none() {
        return false;
    }
    true
}

fn copy_sunk_record(
    log: &mut RawLog<SUNK_N>,
    memory: &impl engine_native_memory::Memory,
    base: usize,
    current_hour: i32,
    pointer: usize,
) -> bool {
    let record = match kspan(log, memory, Some(pointer), csunk_ship_info::SIZE, SUNK_MASK) {
        Some(start) => start,
        None => return false,
    };
    let mut staged = [0u8; csunk_ship_info::SIZE];
    staged.copy_from_slice(&log.bytes[record..record + csunk_ship_info::SIZE]);
    let u64_at = |offset: usize| u64::from_le_bytes(staged[offset..offset + 8].try_into().unwrap());
    let u32_at = |offset: usize| u32::from_le_bytes(staged[offset..offset + 4].try_into().unwrap());
    let expected = match engine_sites::at(base, csunk_ship_info::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    if u64_at(0) != expected {
        return false;
    }
    let expected_date = match engine_sites::at(base, cgame_date::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    let expected_persistent = match engine_sites::at(base, cgame_date::PERSISTENT_VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => return false,
    };
    if u64_at(csunk_ship_info::SINK_DATE) != expected_date
        || u64_at(csunk_ship_info::SINK_DATE_PERSISTENT) != expected_persistent
    {
        return false;
    }
    let sink_hour = u32_at(csunk_ship_info::SINK_HOUR) as i32;
    if sink_hour <= 0 || sink_hour > current_hour {
        return false;
    }
    if staged[csunk_ship_info::CAUSE_RECORD] > 1 || staged[csunk_ship_info::ASSISTED_KILL] > 1 {
        return false;
    }
    if !copy_sunk_class(log, memory, base, u64_at(csunk_ship_info::SHIP_CLASS)) {
        return false;
    }
    let attribution = u64_at(csunk_ship_info::ATTRIBUTION_CLASS);
    if attribution != 0 && !copy_sunk_class(log, memory, base, attribution) {
        return false;
    }
    for offset in [
        csunk_ship_info::SHIP_NAME,
        csunk_ship_info::ATTRIBUTION_SOURCE_NAME,
    ] {
        if !copy_sunk_string(log, memory, pointer + offset, record + offset) {
            return false;
        }
        if log.overflowed {
            return false;
        }
    }
    let province_pointer = u64_at(csunk_ship_info::PROVINCE);
    if province_pointer != 0 {
        let province = match usize::try_from(province_pointer).ok() {
            Some(province) => province,
            None => return false,
        };
        let vtable = match kspan(log, memory, Some(province), 8, SUNK_MASK) {
            Some(start) => start,
            None => return false,
        };
        let expected_province = match engine_sites::at(base, cprovince::VTABLE_RVA) {
            Some(expected) => expected as u64,
            None => return false,
        };
        if sunk_pool_u64(log, vtable) != expected_province {
            return false;
        }
        if kspan(
            log,
            memory,
            province.checked_add(cprovince::ID),
            4,
            SUNK_MASK,
        )
        .is_none()
        {
            return false;
        }
    }
    true
}

fn copy_sunk_pass(
    job: &mut SunkJob,
    memory: &impl engine_native_memory::Memory,
    base: usize,
    game_state: usize,
    current_hour: i32,
    ticks: &mut [u64; SUNK_N],
) {
    copy_sunk_pass_mode(job, memory, base, game_state, current_hour, ticks);
}

fn copy_sunk_pass_mode(
    job: &mut SunkJob,
    memory: &impl engine_native_memory::Memory,
    base: usize,
    game_state: usize,
    current_hour: i32,
    ticks: &mut [u64; SUNK_N],
) {
    let _reads = read_accounting::table(TableId::SunkShips);
    job.clear();
    job.active = true;
    job.game_state = game_state;
    job.hour = current_hour;
    let started = qpc();
    copy_sunk_tables(job, memory, base, game_state, current_hour);
    ticks[0] += elapsed(started);
}

fn copy_sunk_tables(
    job: &mut SunkJob,
    memory: &impl engine_native_memory::Memory,
    base: usize,
    game_state: usize,
    current_hour: i32,
) {
    if game_state == 0 || current_hour <= 0 {
        return;
    }
    let log = &mut job.log;
    let header = match kspan(
        log,
        memory,
        game_state.checked_add(cgame_state::SUNK_SHIP_HISTORY),
        16,
        SUNK_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header..header + 16]);
    let array = match Array::from_header(raw, 8) {
        Some(array) => array,
        None => return,
    };
    for index in 0..array.count {
        let addr = (index as usize)
            .checked_mul(8)
            .and_then(|offset| array.data.checked_add(offset));
        let slot = match kspan(log, memory, addr, 8, SUNK_MASK) {
            Some(start) => start,
            None => return,
        };
        let pointer = match usize::try_from(sunk_pool_u64(log, slot))
            .ok()
            .filter(|&pointer| pointer != 0)
        {
            Some(pointer) => pointer,
            None => return,
        };
        if !copy_sunk_record(log, memory, base, current_hour, pointer) {
            return;
        }
        if log.overflowed {
            return;
        }
    }
}

pub fn copy_sunk_refresh(
    memory: &impl Memory,
    ctx: SunkCopyContext,
    job: &mut SunkJob,
) -> SunkCopyReport {
    if !replay_data::cadence::should_walk(TableId::SunkShips, ctx.hour, ctx.force_full) {
        job.clear();
        return SunkCopyReport::default();
    }
    let mut ticks = [0u64; SUNK_N];
    copy_sunk_pass(job, memory, ctx.base, ctx.game_state, ctx.hour, &mut ticks);
    SunkCopyReport {
        raw_ok: !job.log.overflowed,
        copy_ticks: ticks,
    }
}

impl Captured {
    pub fn fail_sunk_capture(&mut self, reason: &'static str) {
        self.sunk.outcome = failed(self.active[13], reason);
        self.sunk_names.reset();
        self.sunk_raw_mode = false;
    }

    pub fn decode_sunk_payload(
        &mut self,
        base: usize,
        job: &SunkJob,
        decode_ticks: &mut [u64; SUNK_N],
    ) {
        self.active[13] = job.active;
        if !job.active {
            self.fail_sunk_capture(replay_data::cadence::SKIPPED_REASON);
            return;
        }
        let recorded = match collector_raw::raw::Recorded::build(&job.log) {
            Ok(recorded) => recorded,
            Err(reason) => {
                self.fail_sunk_capture(reason);
                return;
            }
        };
        let started = qpc();
        let first = csunk_ship_info::walk(
            &recorded,
            base,
            job.game_state,
            job.hour,
            &mut self.sunk.rows,
            &mut self.sunk_names.first,
        );
        self.sunk.outcome = bounded(first.outcome, self.sunk.rows.len());
        self.sunk_names.len = first.bytes_len;
        self.sunk_names.required = first.required_bytes;
        if !valid_sunk(
            self.sunk.outcome,
            &self.sunk.rows,
            &self.sunk_names.first,
            first.bytes_len,
            first.required_bytes,
        ) {
            self.sunk.outcome = invalid_names();
        }
        decode_ticks[0] += elapsed(started);
        self.sunk_raw_mode = false;
    }
}
