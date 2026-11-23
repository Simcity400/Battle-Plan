use super::*;
use map_display_data::{
    landscape::{ColourOp, LayerProgram, ProvinceIds, TargetOp, TargetPrograms},
    retained::Retained,
    value_index::ValueIndex,
};
use replay_data::rows::TableId;
use replay_frame::frame::ViewIdentity;
use std::{io, sync::Weak};

#[derive(Default)]
pub(crate) struct Fold {
    bank: Option<Weak<content_reader_map::map::MapData>>,
    active: Option<Active>,
}
struct Active {
    mode: Retained<String>,
    viewer: u32,
    branch: u64,
    campaign: ViewIdentity,
    bank: Weak<content_reader_map::map::MapData>,
    hour: i32,
    layers: [Option<State>; 2],
}
#[derive(Clone, PartialEq, Eq)]
struct Callback {
    operation: ColourOp,
    refusals: Retained<Vec<String>>,
}
#[derive(Default)]
struct State {
    groups: Option<targets::Groups>,
    chosen: Option<Result<Option<Retained<Vec<u32>>>, Vec<String>>>,
    selected: ValueIndex<TargetId, ProvinceIds>,
    published_groups: ValueIndex<TargetId, ProvinceIds>,
    published_chosen: Option<Retained<Vec<u32>>>,
    callbacks: ValueIndex<TargetId, Callback>,
    counts: ValueIndex<Retained<String>, usize>,
    program: LayerProgram,
    routes: routes::Routes,
    pending: ValueIndex<Option<TargetId>, ()>,
    temps: Option<Temps>,
    rerun: ValueIndex<TargetId, ()>,
    #[cfg(test)]
    evaluated: usize,
}
impl Fold {
    pub(crate) fn bind(&mut self, map: &std::sync::Arc<content_reader_map::map::MapData>) {
        self.bank = Some(std::sync::Arc::downgrade(map));
    }
    pub(crate) fn close(
        &mut self,
        mode: &str,
        side: &str,
        layer: &Layer,
        inputs: &Inputs,
    ) -> io::Result<LayerProgram> {
        let what = format!("Scripted map mode {mode}: its {side} layer");
        let refused = |why| LayerProgram {
            targets: Default::default(),
            refusals: vec![why],
            refused: true,
        };
        let Some(world) = inputs.world else {
            return Ok(refused(format!(
                "{what}'s colour script has no recorded hour to run on"
            )));
        };
        self.close_lazy(mode, side, layer, inputs, &|| world)
    }
    pub(crate) fn close_lazy<'a, 'w>(
        &mut self,
        mode: &str,
        side: &str,
        layer: &Layer,
        inputs: &Inputs<'a, 'w>,
        world: &dyn Fn() -> &'a dashboard_text::triggers::World<'w>,
    ) -> io::Result<LayerProgram> {
        let what = format!("Scripted map mode {mode}: its {side} layer");
        let refused = |why| LayerProgram {
            targets: Default::default(),
            refusals: vec![why],
            refused: true,
        };
        let Some(viewer) = inputs.selected else {
            return Ok(refused(format!(
                "{what}'s colour script needs a viewing country"
            )));
        };
        let bank = self
            .bank
            .as_ref()
            .filter(|bank| bank.strong_count() != 0 && bank.as_ptr() == inputs.map as *const _);
        if self.active.as_ref().is_none_or(|active| {
            active.mode.as_str() != mode
                || active.viewer != viewer
                || active.branch != inputs.frame.branch
                || !active
                    .campaign
                    .is(&inputs.frame.tables[TableId::Campaign.index()])
                || active.hour > inputs.frame.hour
                || bank.is_none_or(|bank| !bank.ptr_eq(&active.bank))
        }) {
            self.active = Some(Active {
                mode: Retained::new(mode.to_owned()),
                viewer,
                branch: inputs.frame.branch,
                campaign: inputs.frame.tables[TableId::Campaign.index()].identity(),
                bank: bank.cloned().unwrap_or_default(),
                hour: inputs.frame.hour,
                layers: Default::default(),
            });
        }
        let active = self.active.as_mut().unwrap();
        active.hour = inputs.frame.hour;
        let state = active.layers[usize::from(side == "top")].get_or_insert_with(State::default);
        let temps = match forced_refresh(mode, inputs) {
            Ok(false) => Temps::Fresh,
            Ok(true) | Err(_) => Temps::Inherited,
        };
        state.close(layer, inputs, world, viewer, temps, &what)
    }
}
impl State {
    fn close<'a, 'w>(
        &mut self,
        layer: &Layer,
        inputs: &Inputs<'a, 'w>,
        world: &dyn Fn() -> &'a dashboard_text::triggers::World<'w>,
        viewer: u32,
        temps: Temps,
        what: &str,
    ) -> io::Result<LayerProgram> {
        let evaluator = std::cell::OnceCell::new();
        let evaluate_with = || {
            evaluator.get_or_init(|| {
                dashboard_text::triggers::inputs::unrecorded(|| {
                    Evaluator::new(world(), None).with_temps(temps)
                })
            })
        };
        let changed = self.routes.advance(inputs.frame)?;
        for key in changed.keys() {
            self.pending = self.pending.inserted(*key, ())?;
            if let Some(id) = key {
                self.rerun = self.rerun.inserted(*id, ())?;
            }
        }
        if self.temps != Some(temps) {
            for &id in self.callbacks.keys() {
                self.pending = self.pending.inserted(Some(id), ())?;
                self.rerun = self.rerun.inserted(id, ())?;
            }
            self.temps = Some(temps);
        }
        if self.chosen.is_none() || self.pending.contains_key(&None) {
            let ((chosen, failures), reads) =
                dashboard_text::triggers::inputs::capture(inputs.frame, || {
                    if layer
                        .targets
                        .as_ref()
                        .is_some_and(|targets| !targets.is_empty())
                    {
                        evaluate_with()
                            .capture_colour_inputs(|| choose(layer, evaluate_with(), viewer))
                    } else {
                        (Ok(None), Default::default())
                    }
                });
            self.routes.replace(None, &reads, inputs.frame)?;
            self.pending = self.pending.removed(&None)?;
            self.chosen = Some(if !failures.is_empty() {
                Err(failures
                    .into_iter()
                    .map(|why| format!("{what}'s target selection is refused for 1 layer: {why}"))
                    .collect())
            } else {
                match chosen {
                    Ok(chosen) => Ok(chosen.map(Retained::new)),
                    Err(why) => Err(vec![format!("{what}'s targets are refused: {why}")]),
                }
            });
        }
        let chosen = match self.chosen.as_ref().unwrap() {
            Ok(chosen) => chosen.clone(),
            Err(reasons) => {
                return Ok(LayerProgram {
                    targets: Default::default(),
                    refusals: reasons.clone(),
                    refused: true,
                });
            }
        };
        let groups = (|| {
            let mut groups =
                targets::Groups::read(layer.kind, inputs, self.groups.as_ref(), false)?;
            if layer.kind == LayerType::StateController
                && groups.states_exist(chosen.as_deref().map(|chosen| chosen.as_slice()))
            {
                groups = targets::Groups::read(layer.kind, inputs, Some(&groups), true)?;
            }
            Ok::<_, io::Error>(groups)
        })();
        let groups = match groups {
            Ok(groups) => groups,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::OutOfMemory
                ) =>
            {
                return Err(error);
            }
            Err(error) => {
                return Ok(LayerProgram {
                    targets: Default::default(),
                    refusals: vec![format!("{what}'s targets are refused: {error}")],
                    refused: true,
                });
            }
        };
        let error = if let Some(chosen) = &chosen {
            chosen.iter().find_map(|state| groups.errors.get(state))
        } else {
            groups.errors.iter().next().map(|(_, reason)| reason)
        };
        if let Some(why) = error.cloned() {
            self.groups = Some(groups);
            return Ok(LayerProgram {
                targets: Default::default(),
                refusals: vec![format!("{what}'s targets are refused: {why}")],
                refused: true,
            });
        }
        let mut dirty = ValueIndex::default();
        let mut error = None;
        let old_targets = self.published_groups.clone();
        groups.targets.visit_changes(&old_targets, |id, _, _| {
            if error.is_none() {
                match dirty.inserted(*id, ()) {
                    Ok(next) => dirty = next,
                    Err(why) => error = Some(why),
                }
            }
        });
        if let Some(why) = error {
            return Err(why);
        }
        {
            let previous = self.published_chosen.clone();
            if previous != chosen {
                if let (Some(before), Some(after)) = (&previous, &chosen) {
                    let mut a = before.iter().peekable();
                    let mut b = after.iter().peekable();
                    while a.peek().is_some() || b.peek().is_some() {
                        let (value, changed) = match (a.peek(), b.peek()) {
                            (Some(x), Some(y)) if x == y => {
                                a.next();
                                b.next();
                                continue;
                            }
                            (Some(x), Some(y)) if x < y => (*a.next().unwrap(), true),
                            (_, Some(_)) => (*b.next().unwrap(), true),
                            _ => (*a.next().unwrap(), true),
                        };
                        if changed {
                            groups.select(layer.kind, value, |id| {
                                dirty = dirty.inserted(id, ())?;
                                Ok(())
                            })?;
                            if let Some(old) = &self.groups {
                                old.select(layer.kind, value, |id| {
                                    dirty = dirty.inserted(id, ())?;
                                    Ok(())
                                })?;
                            }
                        }
                    }
                } else {
                    for id in groups.targets.keys().chain(self.selected.keys()) {
                        dirty = dirty.inserted(*id, ())?;
                    }
                }
            }
        }
        for &id in dirty.keys() {
            let selector = match id {
                TargetId::Country(slot) | TargetId::State(slot) => slot,
                TargetId::Portion { state, .. } => state,
            };
            self.selected = match groups.targets.get(&id).filter(|_| {
                chosen
                    .as_ref()
                    .is_none_or(|chosen| chosen.binary_search(&selector).is_ok())
            }) {
                Some(members) => self.selected.inserted(id, members.clone())?,
                None => self.selected.removed(&id)?,
            };
            self.pending = self.pending.inserted(Some(id), ())?;
        }
        self.published_groups = groups.targets.clone();
        self.published_chosen = chosen;
        self.groups = Some(groups);
        let old_counts = self.counts.clone();
        let mut output = self.program.targets.0.clone();
        for &id in self.pending.keys().flatten() {
            let Some(members) = self.selected.get(&id) else {
                if let Some(old) = self.callbacks.get(&id) {
                    self.counts = counts(&self.counts, &old.refusals, false)?;
                }
                self.callbacks = self.callbacks.removed(&id)?;
                self.routes.remove(Some(id))?;
                output = output.removed(&id)?;
                continue;
            };
            let rerun = self.callbacks.get(&id).is_none() || self.rerun.contains_key(&id);
            if rerun {
                #[cfg(test)]
                {
                    self.evaluated += 1;
                }
                let (result, reads) =
                    dashboard_text::triggers::inputs::capture(inputs.frame, || {
                        evaluate(layer, evaluate_with(), viewer, id)
                    });
                let (operation, refusals) = result?;
                if let Some(old) = self.callbacks.get(&id) {
                    self.counts = counts(&self.counts, &old.refusals, false)?;
                }
                self.counts = counts(&self.counts, &refusals, true)?;
                let result = Callback {
                    operation,
                    refusals: Retained::new(refusals),
                };
                self.callbacks = self.callbacks.inserted(id, result)?;
                self.routes.replace(Some(id), &reads, inputs.frame)?;
            }
            let operation = self
                .callbacks
                .get(&id)
                .expect("selected script target callback")
                .operation
                .clone();
            let target = Retained::new(TargetOp {
                id,
                provinces: members.clone(),
                operation,
            });
            output = output.inserted(id, target)?;
        }
        self.pending = ValueIndex::default();
        self.rerun = ValueIndex::default();
        if !self.counts.ptr_eq(&old_counts) || self.program.refused {
            self.program.refusals = self
                .counts
                .iter()
                .map(|(reason, count)| {
                    format!("{what}'s colour script is refused for {count} targets: {reason}")
                })
                .collect();
        }
        self.program.targets = TargetPrograms(output);
        self.program.refused = false;
        Ok(self.program.clone())
    }
}
fn counts(
    old: &ValueIndex<Retained<String>, usize>,
    reasons: &[String],
    add: bool,
) -> io::Result<ValueIndex<Retained<String>, usize>> {
    let mut next = old.clone();
    for reason in reasons {
        let key = Retained::new(reason.clone());
        let count = next.get(&key).copied().unwrap_or(0);
        next = if add {
            next.inserted(key, count + 1)?
        } else if count > 1 {
            next.inserted(key, count - 1)?
        } else {
            next.removed(&key)?
        };
    }
    Ok(next)
}
fn evaluate(
    layer: &Layer,
    evaluator: &Evaluator<'_, '_>,
    viewer: u32,
    id: TargetId,
) -> io::Result<(ColourOp, Vec<String>)> {
    let (object, controller) = match id {
        TargetId::Country(slot) => (EventObject::Country(slot), None),
        TargetId::State(state) => (EventObject::State(state), None),
        TargetId::Portion { state, controller } => (EventObject::State(state), Some(controller)),
    };
    let mut scopes = Vec::with_capacity(if controller.is_some() { 3 } else { 2 });
    scopes.extend([
        EventScope {
            from: Link::Scope(1),
            ..EventScope::of(EventObject::Country(viewer))
        },
        EventScope {
            from: if controller.is_some() {
                Link::Scope(2)
            } else {
                Link::Itself
            },
            ..EventScope::of(object)
        },
    ]);
    if let Some(controller) = controller {
        scopes.push(EventScope::of(EventObject::Country(controller)));
    }
    let _bound = evaluator.bind_event_scopes(Rc::new(RefCell::new(EventScopes::new(scopes))));
    let result = evaluator
        .event_scopes(0)
        .map_err(|why| vec![why.to_string()])
        .and_then(|scopes| colour(evaluator, layer, scopes));
    Ok(match result {
        Ok(Some(style)) => (ColourOp::Set { style }, Vec::new()),
        Ok(None) => (ColourOp::Keep {}, Vec::new()),
        Err(reasons) => (ColourOp::Clear {}, reasons),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map_mode::{self, tests::{content, frame, map}};
    use dashboard_text::triggers::{ContentFacts, World};
    use content_reader_map::scripted_map_modes::{self, MapModes};
    use replay_data::rows::{CountryLinkRow, CountryRow, ProvinceRow, TableRows};
    use replay_frame::frame::{FrameDecoder, FrameEncoder, TableView};
    use std::{collections::BTreeSet, sync::Arc};

    #[test]
    fn scripted_sparse_targets_share_quiet_roots_and_remap_only_changed_memberships() {
        viewer_scene_base::test_game::enter();
        let mut modes = MapModes::default();
        scripted_map_modes::parse_into(
            &"synthetic.txt".into(),
            br#"scripted_map_modes = {
            folded = { top = { type = country color = {
                set_temp_variable = { alpha = 1 }
                set_temp_variable = { red = 1 }
                set_temp_variable = { green = 0 }
                set_temp_variable = { blue = 0 }
                set_temp_variable = { highlighted = 0 }
            } } }
        }"#,
            &mut modes,
        )
        .unwrap();
        let content = content(modes);
        let bank = Arc::new(map());
        let mut fold = Fold::default();
        fold.bind(&bank);
        let mut input = frame();
        input.tables[TableId::Campaign.index()] = viewer_scene_base::test_support::test_campaign();
        input.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            (1..=1501)
                .map(|id| CountryRow {
                    id,
                    canonical_slot: id - 1,
                    tag: vec![
                        b'A' + ((id / 676) % 26) as u8,
                        b'A' + ((id / 26) % 26) as u8,
                        b'A' + (id % 26) as u8,
                    ],
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
        input.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
            (1..=1501)
                .map(|id| CountryLinkRow {
                    id,
                    canonical_slot: id - 1,
                })
                .collect(),
        ))
        .into();
        input.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
            (1..=1501)
                .map(|id| ProvinceRow {
                    id,
                    state: 1,
                    controller: id,
                    owner: id,
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
        let names = content_reader_script::trigger_keys::TriggerNames {
            scripted: Default::default(),
            counts: Some(Default::default()),
        };
        let dlcs = BTreeSet::new();
        let aliases = Arc::default();
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new().with_change_facts();
        let mut previous: Option<LayerProgram> = None;
        for step in 0..6 {
            input.hour += 1;
            if step == 2 {
                let TableView::Complete(TableRows::Provinces(rows)) =
                    &mut *input.tables[TableId::Provinces.index()]
                else {
                    unreachable!()
                };
                rows[776].controller = 2;
            }
            if step == 3 {
                let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut *input.tables[TableId::CountryLinks.index()]
                else {
                    unreachable!()
                };
                rows[1].canonical_slot = 1500;
            }
            let encoded = encoder.encode(input.clone(), step == 0).unwrap();
            let frame = decoder.advance(&encoded).unwrap();
            let world = World::read(frame, ContentFacts::content_only(&dlcs, &names, &aliases));
            let inputs = Inputs {
                content: &content,
                map: &bank,
                frame,
                selected: Some(0),
                world: Some(&world),
                colour: None,
            };
            let layer = &content.scripted.get("folded").unwrap().top;
            let world_reads = std::cell::Cell::new(0);
            let program = fold
                .close_lazy("folded", "top", layer, &inputs, &|| {
                    world_reads.set(world_reads.get() + 1);
                    &world
                })
                .unwrap();
            assert_eq!(world_reads.get(), usize::from(step == 0));
            let expected = map_mode::colour_oracle::borders(
                "folded",
                "top",
                layer,
                &inputs,
                None,
                true,
                &mut Vec::new(),
            );
            assert!(expected.borders[&TargetId::Country(1000)].style.is_some());
            assert_eq!(
                map_display_data::landscape::apply_layer(&program, None),
                expected
            );
            let state = fold.active.as_ref().unwrap().layers[1].as_ref().unwrap();
            assert_eq!(
                state.evaluated, 1501,
                "membership-only edits never rerun an existing constant callback"
            );
            if let Some(previous) = &previous {
                if step == 1 || step >= 4 {
                    assert!(program.targets.0.ptr_eq(&previous.targets.0));
                }
                assert!(
                    program
                        .targets
                        .0
                        .get(&TargetId::Country(1000))
                        .unwrap()
                        .ptr_eq(previous.targets.0.get(&TargetId::Country(1000)).unwrap())
                );
            }
            previous = Some(program);
        }
    }
    #[test]
    fn split_controller_refusal_recovery_keeps_pending_membership_and_callbacks() {
        viewer_scene_base::test_game::enter();
        let mut modes = MapModes::default();
        scripted_map_modes::parse_into(
            &"synthetic.txt".into(),
            br#"scripted_map_modes = {
                split = { top = { type = state_controller
                    targets = { targets = { 1 } }
                    color = {
                        set_temp_variable = { alpha = 1 }
                        set_temp_variable = { red = 1 }
                        set_temp_variable = { green = 0 }
                        set_temp_variable = { blue = 0 }
                        set_temp_variable = { highlighted = 0 }
                    }
                } }
            }"#,
            &mut modes,
        )
        .unwrap();
        let content = content(modes);
        let bank = Arc::new(map());
        let mut fold = Fold::default();
        fold.bind(&bank);
        let mut input = frame();
        input.tables[TableId::Campaign.index()] = viewer_scene_base::test_support::test_campaign();
        let names = content_reader_script::trigger_keys::TriggerNames::default();
        let dlcs = BTreeSet::new();
        let aliases = Arc::default();
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new().with_change_facts();
        for step in 0..8 {
            input.hour += 1;
            let TableView::Complete(TableRows::Provinces(provinces)) =
                &mut *input.tables[TableId::Provinces.index()]
            else {
                unreachable!()
            };
            provinces[1].controller = if step < 2 { 100 } else { 200 };
            let TableView::Complete(TableRows::States(states)) =
                &mut *input.tables[TableId::States.index()]
            else {
                unreachable!()
            };
            for state in states {
                state.controller = 100;
                state.single_controller = Some(0).into();
            }
            let TableView::Complete(TableRows::States(states)) =
                &mut *input.tables[TableId::States.index()]
            else {
                unreachable!()
            };
            states[0].single_controller = if (1..=3).contains(&step) {
                replay_data::rows::Recorded::Value(None)
            } else {
                Some(u32::from(step == 6)).into()
            };
            let encoded = encoder.encode(input.clone(), step == 0).unwrap();
            let frame = decoder.advance(&encoded).unwrap();
            let world = World::read(frame, ContentFacts::content_only(&dlcs, &names, &aliases));
            let inputs = Inputs {
                content: &content,
                map: &bank,
                frame,
                selected: Some(0),
                world: Some(&world),
                colour: None,
            };
            let layer = &content.scripted.get("split").unwrap().top;
            let actual = fold.close("split", "top", layer, &inputs).unwrap();
            let expected = map_mode::colour_oracle::borders(
                "split",
                "top",
                layer,
                &inputs,
                None,
                true,
                &mut Vec::new(),
            );
            if !actual.refused {
                assert!(expected.borders.values().any(|border| border.style.is_some()));
            }
            assert_eq!(
                map_display_data::landscape::apply_layer(&actual, None),
                expected,
                "refusal/recovery step {step}"
            );
            assert_eq!(actual.refused, (1..=3).contains(&step));
        }
    }
}
