use viewer_layers_forces::units::{models::{Kept, Select}, meshes::{self, Chosen}};
use replay_frame::frame::DecodedFrame;
use std::{collections::{BTreeSet, HashMap, HashSet}, io};
use viewer_foundation::perspective::Choice;
use viewer_unit_models::{trade_routes::ConvoyLine, unit_models::{KeptChoices, PreparedModels, SelectInputs, StackModel, Told}};

impl Select for crate::document::Content {
    fn choose_meshes(&self, frame: &DecodedFrame, ours: u32, (kept, divisions): (&KeptChoices, Option<&BTreeSet<u64>>), stacks: &HashMap<u64, meshes::Asked>, asked: &HashSet<u64>, told: &Told, parts: &mut dyn FnMut(&'static str, u64)) -> io::Result<Chosen> {
        engine_executable::executable_tables::scope(self.tables, || {
            let mut since = std::time::Instant::now();
            let mut lap = |name: &'static str| {
                let now = std::time::Instant::now();
                parts(name, now.duration_since(since).as_nanos() as u64);
                since = now;
            };
            let hour = crate::document::Hour::new(self, frame, 0.0);
            let values = viewer_unit_models::equipment_graphics::LimitValues::default();
            let evaluate = |items: &[content_reader_parser::parser::Item], scopes| hour.trigger_world().evaluate(items, scopes);
            let (mut standing, mut choosing) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
            let inputs = SelectInputs {
                frame,
                limits: viewer_unit_models::equipment_graphics::Limits::new(&values, &evaluate),
                ours,
                terrain: &self.counters.terrain.combat_terrain,
                localisation: &self.names.localisation,
                divisions,
                lap: Some(&mut lap),
            };
            let (models, made) = self.unit_models.land_models(inputs, kept, told, |models| {
                let mut chosen = Vec::new();
                for (&oldest, &(province, key)) in stacks {
                    let started = std::time::Instant::now();
                    let stands = !asked.contains(&oldest) && models.stands(province, key);
                    standing += started.elapsed();
                    if !stands {
                        let started = std::time::Instant::now();
                        chosen.push((oldest, models.choose(province, key)));
                        choosing += started.elapsed();
                    }
                }
                (chosen, models.made())
            })?;
            parts("um_session_stands", standing.as_nanos() as u64);
            parts("um_session_choose", choosing.as_nanos() as u64);
            viewer_runtime::view_record::add("units", "stacks_walked", stacks.len() as u64);
            viewer_runtime::view_record::add("units", "stacks_asked", asked.len() as u64);
            viewer_runtime::view_record::add("units", "stacks_sessions", 1);
            let mut since = std::time::Instant::now();
            let entities: Vec<String> = models.iter().filter_map(|(_, model)| match model {
                StackModel::Chosen(chosen) => Some(chosen.entity.clone()),
                _ => None,
            }).collect();
            let mut slots = self.unit_models.chosen_meshes(&entities).into_iter();
            parts("bake", since.elapsed().as_nanos() as u64);
            since = std::time::Instant::now();
            let meshes = models
                .into_iter()
                .map(|(oldest, model)| {
                    let mesh = match model {
                        StackModel::Chosen(_) => slots.next().map_or_else(|| Err("a chosen model was not baked".to_owned()), |slot| slot.map(Some)),
                        StackModel::UnknownIdentity | StackModel::Untemplated => Ok(None),
                        StackModel::Withheld(reason) => Err(reason),
                    };
                    (oldest, mesh)
                })
                .collect();
            parts("slots", since.elapsed().as_nanos() as u64);
            Ok(Chosen { meshes, made })
        })
    }

    fn prepare(&self, frame: &DecodedFrame, perspective: &Choice, layered: Option<&[ConvoyLine]>, naval: bool, kept: &mut Kept) -> io::Result<PreparedModels> {
        engine_executable::executable_tables::scope(self.tables, || {
            let (convoys, parts) = kept.state_and_timings();
            let mut since = std::time::Instant::now();
            let mut lap = |name: &'static str| {
                let now = std::time::Instant::now();
                parts.push((name, now.duration_since(since).as_nanos() as u64));
                since = now;
            };
            let hour = crate::document::Hour::new(self, frame, 0.0);
            lap("hour");
            let ours = viewer_layers_forces::intel_icons::ours(frame, perspective)?;
            lap("viewer");
            let mut units = hour.prepare_unit_models_without_land(perspective, naval, ours, &mut lap)?;
            let _cost = replay_frame::cpu::scope("viewer", Some("actors/units"));
            units.add_raid_units(&self.unit_models, &hour.raid_arrows(ours).0.raid_units);
            lap("raid_units");
            if let Ok(defines) = &self.trade_routes {
                let made = layered.is_none().then(|| hour.trade_route_convoys_kept(ours, convoys));
                lap("convoy_lines");
                let lines = match (&made, layered) {
                    (Some(made), _) => (**made).clone(),
                    (None, Some(lines)) => lines.to_vec(),
                    (None, None) => Vec::new(),
                };
                lap("convoy_copy");
                units.add_convoys(lines, defines.convoys);
            }
            lap("convoy_add");
            Ok(units)
        })
    }
}
