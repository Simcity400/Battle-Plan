//! TLS variable RNG, distinct from event scopes' iterator seed.
use crate::triggers::Failure;

use super::*;
use crate::scripted_text::{EventObject, EventScope, random};

impl Evaluator<'_, '_> {
    /// Lazy failures preserve descriptions that never consume RNG.
    /// The native outer context fixes THIS/FROM before nested checks enter scopes.
    pub(in crate::triggers) fn initialize_variable_random(&self, scopes: Scopes) {
        if self.temps_written().is_some() {
            return; // An unknown writer may have changed random_seed; never reseed it.
        }
        if self
            .local_temps
            .borrow()
            .contains_key(b"random_seed".as_slice())
        {
            return;
        }
        let value = super::super::refusal::deferred(|| self.initial_variable_seed(scopes))
            .map(Value::Raw)
            .map_err(|why| why.context(|why| format!("random_seed: {why}")));
        self.local_temps
            .borrow_mut()
            .insert(b"random_seed".to_vec(), value);
    }

    fn initial_variable_seed(&self, scopes: Scopes) -> Result<i64, Failure> {
        if self.temps() == Temps::Inherited {
            return Err(super::inherited_temps_failure());
        }
        let (this, from) = if let Some(at) = scopes.cursor {
            let binding = self.event_graph.borrow();
            let graph = binding
                .as_ref()
                .ok_or("variable RNG scope graph is missing")?
                .borrow();
            let this = *graph
                .scopes
                .get(at)
                .ok_or("variable RNG cursor is absent")?;
            let from = graph.scopes[super::super::event_scopes::linked(&graph, at, this.from)?];
            (this, from)
        } else {
            if scopes.unknown.from {
                return Err("variable RNG FROM scope is unknown".into());
            }
            let object = super::super::event_scopes::event_object;
            (
                EventScope::of(object(scopes.this)?),
                EventScope::of(object(scopes.from.unwrap_or(scopes.this))?),
            )
        };
        // Variable seed initialisation selects country, state, character,
        // combat leader, then GLOBAL; it does not select the MIO/project store.
        let store = match this.this {
            EventObject::Combat => return Err(
                "bare combat variable RNG needs its combat leader seed, not the side country store"
                    .into(),
            ),
            EventObject::Country(country)
            | EventObject::CountryState { country, .. }
            | EventObject::CountryUnitLeader { country, .. }
                if self.variable_scope_has_country(country)? =>
            {
                Store::Country(country)
            }
            EventObject::State(state) | EventObject::CountryState { state, .. }
                if state > 0 && (state as i32) > 0 =>
            {
                Store::State(state)
            }
            EventObject::Character(id) => Store::Character(id),
            EventObject::UnitLeader(reference)
            | EventObject::CountryUnitLeader {
                leader: reference, ..
            } => Store::Character(
                self.world()
                    .unit_leader(reference)?
                    .ok_or("variable RNG unit leader is not recorded")?
                    .character_id,
            ),
            _ => Store::Global,
        };
        let seed = self.world().variable_seed(store)?;
        // Use immediate FROM (not ROOT). Arithmetic narrows to u32.
        let seed = seed
            .wrapping_add(self.variable_scope_hash(this)?.wrapping_mul(13))
            .wrapping_add(self.variable_scope_hash(from)?.wrapping_mul(17));
        Ok(i64::from(seed))
    }

    // Scope identities use canonical slots, while the country-presence gate uses
    // the signed raw country reference.
    pub(in crate::triggers) fn variable_scope_has_country(
        &self,
        country: u32,
    ) -> Result<bool, Failure> {
        match self.world().raw_country(country) {
            Some(raw) => Ok((raw as i32) > 0),
            None if country == 0 => Ok(false),
            None => Err(format!("variable RNG country slot {country} is not recorded").into()),
        }
    }

    /// Use the object's identity seed, falling back to the stored scope seed.
    fn variable_scope_hash(&self, scope: EventScope) -> Result<u32, Failure> {
        let hash = match scope.this {
            EventObject::Country(country)
            | EventObject::CountryState { country, .. }
            | EventObject::CountryUnitLeader { country, .. }
                if self.variable_scope_has_country(country)? =>
            {
                country.wrapping_mul(31)
            }
            EventObject::State(state) | EventObject::CountryState { state, .. }
                if (state as i32) > 0 =>
            {
                state.wrapping_mul(59)
            }
            EventObject::Character(id) => self.variable_character_hash(id)?,
            EventObject::UnitLeader(reference)
            | EventObject::CountryUnitLeader {
                leader: reference, ..
            } => self.variable_character_hash(
                self.world()
                    .unit_leader(reference)?
                    .ok_or("variable RNG unit leader is not recorded")?
                    .character_id,
            )?,
            EventObject::Unit(reference) => {
                self.world().unit(reference)?;
                ((reference >> 32) as u32).wrapping_mul(3)
            }
            EventObject::IndustrialOrganisation(reference) => {
                self.world().industrial_organisation(reference)?;
                ((reference >> 32) as u32).wrapping_mul(0x52)
            }
            EventObject::Raid(reference) => {
                self.world().raid(reference)?;
                ((reference >> 32) as u32).wrapping_mul(0x79d)
            }
            EventObject::Province(_) => {
                return Err("province is not a variable RNG scope".into());
            }
            _ => scope.require_random_seed()?, // Projects have no identity-seed branch.
        };
        Ok(hash)
    }

    fn variable_character_hash(&self, id: u32) -> Result<u32, Failure> {
        self.world()
            .character(id)?
            .ok_or("variable RNG character is not recorded")?;
        // Reference hashing swaps type/id words; ccharacter::read
        // validates the hard-coded native type before publishing Characters.
        let reference = u64::from(id) << 32 | u64::from(engine_native::ccharacter::REFERENCE_TYPE);
        Ok(super::super::scripted_localisation::reference_hash(
            reference,
        ))
    }

    pub(super) fn next_variable_random(&self, scopes: Scopes) -> Result<i64, Failure> {
        let isolated = self.checking.get() == 0 && self.description_depth.get() == 0;
        if isolated {
            self.local_temps.borrow_mut().clear(); // Open a fresh TLS context.
            self.clear_temp_writer();
        }
        self.initialize_variable_random(scopes);
        let value = self.temp(b"random_seed", scopes)?.unwrap();
        let seed = self.raw(value)? as u32;
        // The counter is fixed from the low word, not its stored high word.
        let counter = 0x5ea6_ba9f_u32.wrapping_sub(seed);
        let fraction = random::scoped_draw(seed, counter) % ONE as i32;
        // Mix seed+1 and seed+2 with that same counter, swap
        // their positions, and clears only bit 63 before storing the TLS value.
        let next = (u64::from(random::scoped_word(seed.wrapping_add(1), counter)) << 32
            | u64::from(random::scoped_word(seed.wrapping_add(2), counter)))
            & 0x7fff_ffff_ffff_ffff;
        self.store_description_temp(b"random_seed".to_vec(), next as i64);
        Ok(i64::from(fraction))
    }
}
