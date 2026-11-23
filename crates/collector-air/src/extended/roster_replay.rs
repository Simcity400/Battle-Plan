use engine_native_memory::Memory;
use engine_native::ccountry;

pub struct RosterReplay<'a, M> {
    pub inner: &'a M,
    pub roster: Option<(usize, usize, engine_native_memory::country_identity::AuthenticatedRoster<'a>)>,
    pub reserved: Option<(usize, usize, engine_native_memory::country_identity::ReservedCountryObservation)>,
}

impl<'a, M> RosterReplay<'a, M> {
    pub fn new(
        inner: &'a M,
        snapshot: &'a ccountry::RosterSnapshot,
        base: usize,
        game_state: usize,
    ) -> Self {
        Self {
            inner,
            roster: snapshot
                .roster(base, game_state)
                .map(|roster| (base, game_state, roster)),
            reserved: snapshot
                .reserved_observation(base, game_state)
                .map(|root| (base, game_state, root)),
        }
    }
}

impl<M: Memory> Memory for RosterReplay<'_, M> {
    fn authenticated_reserved_country(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<Result<engine_native_memory::country_identity::Country, &'static str>> {
        self.authenticated_reserved_country_observation(base, game_state)
            .map(engine_native_memory::country_identity::ReservedCountryObservation::result)
    }

    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<engine_native_memory::country_identity::ReservedCountryObservation> {
        self.reserved
            .filter(|(module, state, _)| (*module, *state) == (base, game_state))
            .map(|(_, _, root)| root)
    }

    #[inline(always)]
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.inner.copy(address, destination)
    }

    fn immutable_view(
        &self,
        address: usize,
        len: usize,
    ) -> Option<engine_native_memory::memory::SliceMemory<'_>> {
        self.inner.immutable_view(address, len)
    }

    fn authenticated_countries(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<engine_native_memory::country_identity::AuthenticatedRoster<'_>> {
        self.roster
            .filter(|(module, state, _)| (*module, *state) == (base, game_state))
            .map(|(_, _, roster)| roster)
    }
}
