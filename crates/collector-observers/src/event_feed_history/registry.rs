use super::*;
use engine_events::{event_feed_sites::provenance::World, script_event_registry as native};
use replay_data::event_feed_rows::EventFeedScriptChange as Change;

pub(super) fn empty(memory: &impl Memory) -> engine_native_memory::msvc_string::Descriptor {
    let mut header = [0; 32];
    header[24..].copy_from_slice(&15_u64.to_le_bytes());
    engine_native_memory::msvc_string::Descriptor::from_header(memory, 0, header)
        .expect("empty inline token")
}
pub(crate) fn marker<M: Memory>(
    _world: &World<'_, M>,
    memory: &M,
    change: Change,
    instance: Option<u32>,
) -> Result<ScriptFact, &'static str> {
    Ok(marker_for(memory, change, instance))
}
pub(crate) fn marker_for(
    memory: &impl Memory,
    change: Change,
    instance: Option<u32>,
) -> ScriptFact {
    let absent = engine_events::event_feed_sites::CountryIdentity::default();
    ScriptFact {
        source: EventFeedSource::ScriptState,
        actor: absent,
        target: absent,
        token: empty(memory),
        scope: None,
        dispatch_frame: 0,
        dispatch_definition: 0,
        cause: Default::default(),
        script: EventFeedScript {
            change: Some(change),
            instance,
            ..Default::default()
        },
    }
}
pub(super) fn country_operand<M: Memory>(
    world: &World<'_, M>,
    reference: u32,
) -> Result<engine_events::event_feed_sites::CountryIdentity, &'static str> {
    if (reference as i32) < 0 {
        Ok(engine_events::event_feed_sites::CountryIdentity {
            reference,
            id: 0,
            instance: 0,
        })
    } else {
        world.country(reference)
    }
}
pub(crate) fn pending<M: Memory>(
    world: &World<'_, M>,
    memory: &M,
    base: usize,
    row: native::Pending,
) -> Result<ScriptFact, &'static str> {
    let (id, token) = crate::script_event_observer::definition(memory, base, row.definition)?;
    let reference = row
        .scope
        .checked_add(engine_events::cevent_scope::COUNTRY_TAG)
        .and_then(|at| engine_native_memory::read::<u32>(memory, at))
        .ok_or("pending event scope country unreadable")?;
    Ok(ScriptFact {
        source: EventFeedSource::ScriptState,
        actor: country_operand(world, row.originator)?,
        target: country_operand(world, reference)?,
        token,
        scope: Some(row.scope),
        dispatch_frame: 0,
        dispatch_definition: 0,
        cause: Default::default(),
        script: EventFeedScript {
            native_event: Some(id),
            instance: Some(row.instance),
            change: Some(Change::PendingStored {
                due_hour: row.due_hour,
            }),
            ..Default::default()
        },
    })
}
pub(crate) fn fired<M: Memory>(
    world: &World<'_, M>,
    memory: &M,
    base: usize,
    definition: usize,
) -> Result<ScriptFact, &'static str> {
    let (id, token) = crate::script_event_observer::definition(memory, base, definition)?;
    let mut fact = marker(world, memory, Change::FiredMember, None)?;
    fact.token = token;
    fact.script.native_event = Some(id);
    Ok(fact)
}
impl Store {
    pub(super) fn registry_baseline(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        hour: i32,
    ) -> Result<(), &'static str> {
        if self.registry_baseline == Some(generation) {
            return Ok(());
        }
        let Some(cut) = super::Cut::take() else {
            return self.registry_crossed();
        };
        if !Self::sample_holds(cut) {
            return self.registry_crossed();
        }
        match self.registry_sample(memory, base, generation, hour, cut) {
            Err(_) if !cut.holds() => self.registry_crossed(),
            result => result,
        }
    }
    fn registry_sample(
        &mut self,
        memory: &impl Memory,
        base: usize,
        generation: u64,
        hour: i32,
        cut: super::Cut,
    ) -> Result<(), &'static str> {
        self.registry_baseline = Some(generation);
        self.registry_complete = false;
        let rejected = self.counts.rejected;
        let world = World::new(memory, base)?;
        if world.hour()? != hour {
            return Err("event registry baseline hour differs");
        }
        let state = world.state_pointer();
        self.registry_state = Some(state);
        self.script_known(
            memory,
            base,
            generation,
            hour,
            marker(&world, memory, Change::PendingBaseline, None)?,
        )?;
        native::walk_pending(memory, state, |row| {
            self.script_known(
                memory,
                base,
                generation,
                hour,
                pending(&world, memory, base, row)?,
            )
        })?;
        if !Self::sample_holds(cut) {
            return self.registry_crossed();
        }
        self.script_known(
            memory,
            base,
            generation,
            hour,
            marker(&world, memory, Change::PendingReady, None)?,
        )?;
        self.script_known(
            memory,
            base,
            generation,
            hour,
            marker(&world, memory, Change::FiredBaseline, None)?,
        )?;
        native::walk_fired(memory, state, |definition| {
            self.script_known(
                memory,
                base,
                generation,
                hour,
                fired(&world, memory, base, definition)?,
            )
        })?;
        if !Self::sample_holds(cut) {
            return self.registry_crossed();
        }
        self.script_known(
            memory,
            base,
            generation,
            hour,
            marker(&world, memory, Change::FiredReady, None)?,
        )?;
        if self.counts.rejected == rejected {
            self.registry_complete = true;
        }
        Ok(())
    }
    fn registry_crossed(&mut self) -> Result<(), &'static str> {
        super::crossed_sample(super::REGISTRY_SAMPLE);
        self.registry_baseline = None;
        self.registry_complete = false;
        Ok(())
    }
}
