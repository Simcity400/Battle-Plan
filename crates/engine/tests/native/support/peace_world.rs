use engine_native_memory::memory::SliceMemory;
use engine_peace::{
    peace_action, peace_conference as c, peace_settlement as p, peace_settlement::Action,
};

pub(crate) struct Fixture {
    pub(crate) bytes: Vec<u8>,
    pub(crate) actions: usize,
    action_data: usize,
    transfers: usize,
    transfer_data: usize,
}
impl Fixture {
    pub(crate) fn rebase_image(&mut self, hour: i32) -> (usize, usize) {
        self.bytes
            .resize(engine_native::cgame_state::SINGLETON_RVA.0 + 8, 0);
        let pointer = self.rebase();
        let base = self.bytes.as_ptr() as usize;
        self.u64(0x100, (base + c::VTABLE_RVA.0) as u64);
        self.u64(
            0x900,
            (base + engine_peace::peace_bidding_turn::VTABLE_RVA.0) as u64,
        );
        for i in 0..self.actions {
            self.u64(
                0x20000 + i * 0x70,
                (base + engine_peace::ctake_state_action::VTABLE_RVA.0) as u64,
            );
        }
        if self.transfers != 0 {
            self.u64(
                0xb00,
                (base + engine_peace::conference_winner_participant::VTABLE_RVA.0) as u64,
            );
            for i in 0..self.transfers {
                self.u64(
                    self.transfer_data + self.transfers * 8 + i * 0x60,
                    (base + engine_native::cstate::VTABLE_RVA.0) as u64,
                );
            }
        }
        self.u64(
            engine_native::cgame_state::SINGLETON_RVA.0,
            (base + 0x10000) as u64,
        );
        self.u32(
            0x10000 + engine_native::cgame_state::CURRENT_HOUR,
            hour as u32,
        );
        (base, pointer)
    }
    pub(crate) fn rebase(&mut self) -> usize {
        let base = self.bytes.as_ptr() as usize;
        let mut offsets = vec![
            0x100 + c::WINNERS,
            0x100 + c::HISTORY,
            0x500,
            0x508,
            0x510,
            0x600,
            0x608,
            0x610,
            0x628,
            0x800,
            0x910,
        ];
        offsets.extend((0..self.actions).map(|i| self.action_data + i * 8));
        if self.transfers != 0 {
            offsets.extend([
                0x100 + c::WINNER_PARTICIPANTS,
                0xa00,
                0xb00 + engine_peace::conference_beneficiary_participant::STATES,
            ]);
            offsets.extend((0..self.transfers).map(|i| self.transfer_data + i * 8));
        }
        for offset in offsets {
            let old = u64::from_le_bytes(self.bytes[offset..offset + 8].try_into().unwrap());
            self.u64(offset, base as u64 + old);
        }
        base + 0x100
    }
    pub(crate) fn new(countries: usize, actions: usize) -> Self {
        let action_data = 0x20000 + actions * 0x70 + countries * 4;
        let mut f = Self {
            bytes: vec![0; action_data + actions * 8],
            actions,
            action_data,
            transfers: 0,
            transfer_data: 0,
        };
        f.u64(0x100, c::VTABLE_RVA.0 as u64);
        f.u64(0x108, 0x123456789);
        f.bytes[0x100 + c::FINALIZED] = 1;
        f.array(0x100 + c::WINNERS, 0x20000 + actions * 0x70, countries, 4);
        for i in 0..countries {
            f.u32(0x20000 + actions * 0x70 + i * 4, 0xa0000000 + i as u32);
        }
        f.u64(0x100 + c::HISTORY, 0x500);
        f.u64(0x108 + c::HISTORY, 1);
        for at in [0x500, 0x508, 0x510] {
            f.u64(at, 0x600);
        }
        f.bytes[0x519] = 1;
        for at in [0x600, 0x608, 0x610] {
            f.u64(at, 0x500);
        }
        f.u32(0x620, 0xa0000000);
        f.array(0x628, 0x800, 1, 8);
        f.u64(0x800, 0x900);
        f.u64(0x900, engine_peace::peace_bidding_turn::VTABLE_RVA.0 as u64);
        f.u32(0x908, 0xa0000000);
        f.array(0x910, action_data, actions, 8);
        for i in 0..actions {
            let at = 0x20000 + i * 0x70;
            f.u64(action_data + i * 8, at as u64);
            f.u64(at, engine_peace::ctake_state_action::VTABLE_RVA.0 as u64);
            f.u32(at + peace_action::TOKEN, 0x90000000 + i as u32);
            f.u64(
                at + peace_action::CACHED_NAME + engine_native_memory::msvc_string::CAPACITY,
                engine_native_memory::msvc_string::INLINE_CAPACITY,
            );
            f.u32(at + peace_action::TAKER, 0xa0000000);
            f.u32(at + peace_action::GIVER, 0xb0000000);
            f.u32(at + peace_action::NEGOTIATOR, 0xc0000000);
            f.u32(at + engine_peace::cstate_peace_action::STATE, i as u32);
            f.bytes[at + engine_peace::cstate_peace_action::FLAGS
                ..at + engine_peace::cstate_peace_action::FLAGS + 4]
                .copy_from_slice(&[1, 2, 3, 4]);
        }
        let registry = f.bytes.len();
        let slots = countries.max(1);
        f.bytes.resize(registry + slots * 8, 0);
        f.array(
            0x10000 + engine_native::cgame_state::COUNTRIES,
            registry,
            slots,
            8,
        );
        f
    }
    pub(crate) fn memory(&self) -> impl engine_native_memory::Memory + '_ {
        struct Memory<'a>(&'a [u8]);
        impl engine_native_memory::Memory for Memory<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == engine_native::cgame_state::SINGLETON_RVA.0 && output.len() == 8 {
                    output.copy_from_slice(&0x10000_u64.to_le_bytes());
                    true
                } else {
                    engine_native_memory::Memory::copy(
                        &SliceMemory {
                            base: 0,
                            bytes: self.0,
                        },
                        address,
                        output,
                    )
                }
            }
        }
        Memory(&self.bytes)
    }
    pub(crate) fn populate_transfers(&mut self, count: usize) {
        self.transfers = count;
        self.transfer_data = self.bytes.len();
        self.bytes
            .resize(self.transfer_data + count * (8 + 0x60), 0);
        self.array(0x100 + c::WINNER_PARTICIPANTS, 0xa00, 1, 8);
        self.u64(0xa00, 0xb00);
        self.u64(
            0xb00,
            engine_peace::conference_winner_participant::VTABLE_RVA.0 as u64,
        );
        self.u32(0xb08, 0xa0000000);
        self.array(
            0xb00 + engine_peace::conference_beneficiary_participant::STATES,
            self.transfer_data,
            count,
            8,
        );
        for i in 0..count {
            let state = self.transfer_data + count * 8 + i * 0x60;
            self.u64(self.transfer_data + i * 8, state as u64);
            self.u64(state, engine_native::cstate::VTABLE_RVA.0 as u64);
            self.u32(state + engine_native::cstate::ID, i as u32);
        }
    }
    pub(crate) fn u64(&mut self, at: usize, value: u64) {
        self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    pub(crate) fn u32(&mut self, at: usize, value: u32) {
        self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    pub(crate) fn array(&mut self, at: usize, data: usize, count: usize, _width: usize) {
        self.u64(at, data as u64);
        self.u32(at + 8, count as u32);
        self.u32(at + 12, count as u32);
    }
    pub(crate) fn copy(
        &self,
        winners: engine_native_memory::walk::Buffer<'_, u32>,
        actions: engine_native_memory::walk::Buffer<'_, Action>,
    ) -> p::Snapshot {
        p::copy(
            &SliceMemory {
                base: 0,
                bytes: &self.bytes,
            },
            0,
            0x100,
            p::Output {
                winners,
                losers: engine_native_memory::walk::Buffer::Count,
                actions,
                transfers: engine_native_memory::walk::Buffer::Count,
                subjects: engine_native_memory::walk::Buffer::Count,
                name_bytes: engine_native_memory::walk::Buffer::Count,
            },
        )
    }
}
