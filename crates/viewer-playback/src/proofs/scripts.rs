
use viewer_layers_political::scripts::Scripts;
use crate::{document::Document, map_open::MapSide};
use viewer_layer_core::playhead_host::{Entry, views::Recording};
use replay::campaign::SeekResult;
use replay_data::Hour;
use replay_frame::frame::DecodedFrame;
use serde_json::{Value, json};
use std::{io, io::Write, sync::Arc};
use viewer_foundation::perspective;

pub struct Line {
    entry: Arc<Entry>,
    recording: Recording,
    on: bool,
    consumer: playhead::Consumer,
    reader: playhead::Reader,
    steps: Vec<Arc<playhead::Step>>,
    step_ns: Vec<u64>,
    age: Vec<f64>,
    totals: std::collections::BTreeMap<String, (u64, u64, u64)>,
}

impl Line {
    pub fn open(entry: &Arc<Entry>, recording: Recording, on: bool, primed: Option<&str>) -> io::Result<Self> {
        let mut needs = Vec::new();
        if let Some(file) = primed {
            for line in std::fs::read_to_string(file)?.lines().map(str::trim).filter(|line| !line.is_empty()) {
                needs.push(need(line).ok_or_else(|| io::Error::other(format!("{file}: {line} names no table and fields of it")))?);
            }
        }
        if on {
            viewer_layers_political::scripts::prime(needs);
        }
        let reader = entry.playhead().declare(Vec::new(), None)?;
        let consumer = entry.playhead().consumer(reader);
        Ok(Self { entry: Arc::clone(entry), recording, on, consumer, reader, steps: Vec::new(), step_ns: Vec::new(), age: Vec::new(), totals: Default::default() })
    }

    pub fn frame(&mut self, side: &MapSide, head: Hour, playing: bool) {
        self.steps.clear();
        self.consumer.take(head, &mut self.steps);
        self.step_ns.extend(self.steps.iter().filter(|step| !step.reset()).map(|step| step.built().ns));
        if self.on
            && playing
            && let Some(product) = viewer_layers_political::scripts::drawn(side.layer_parts()).and_then(Scripts::product)
        {
            self.age.push(f64::from(head - product.hour));
        }
    }

    pub fn compare(&mut self, side: &MapSide, document: &Document, hour: Hour, why: &str, out: &mut impl Write) -> io::Result<()> {
        if !self.on {
            return Ok(());
        }
        let ours = document.hour()?.vision(&perspective::Choice::default())?.ours();
        let today: Arc<DecodedFrame> = match document.playback.replay().seek_selected(document.playback.branch(), hour, None)? {
            SeekResult::Frame(frame) => Arc::new(*frame),
            SeekResult::Gap { reason, .. } => return Err(io::Error::other(format!("Today's reader finds a gap at {hour}: {reason}"))),
        };
        let layer = viewer_layers_political::scripts::drawn(side.layer_parts());
        let tables = layer.map_or(0, |layer| layer.needs().len());
        let reached = layer.and_then(Scripts::product).filter(|product| product.hour == hour && product.complete).and_then(|product| {
            let raids = product.raids.clone().filter(|icons| icons.built_for.is_some_and(|(branch, at, country)| (branch, at, country) == (self.recording.branch, hour, ours)))?;
            let (_, music) = product.music.clone().filter(|(at, _)| *at == hour)?;
            Some((raids, music))
        });
        let Some((raids, music)) = reached else {
            return self.note("scripts:loading".into(), (0, 0, None), json!({"tables": tables}), hour, why, out);
        };
        let content = &document.content;
        let wanted_raids = {
            let assets = &document.icons.raids;
            let paths = viewer_raids::map::raids::paths::HourPaths::new(assets, &today);
            viewer_raids::map::raids::icons(assets, &today, ours, &paths)
        };
        let wanted_music = content.products.audio.as_ref().map(|audio| {
            let world = dashboard_text::triggers::World::read(&*today, content.products.triggers.facts());
            map_products::music_inputs::Memo::new().chances(&today, audio, ours, &world)
        });
        let today_refusals: Vec<String> = wanted_music.as_ref().map(|music| music.refusals.clone()).unwrap_or_default();
        let head_only: Vec<&String> = music.refusals.iter().filter(|refusal| !today_refusals.contains(refusal)).take(4).collect();
        let pairs = [
            ("scripts:raids:icons", format!("{:?}", wanted_raids.icons), format!("{:?}", raids.icons)),
            ("scripts:raids:report", format!("{:?}", wanted_raids.report), format!("{:?}", raids.report)),
            ("scripts:music", format!("{:?}", wanted_music.map(|music| (music.weights, music.refusals))), format!("{:?}", Some((music.weights.clone(), music.refusals.clone())))),
        ];
        for (name, wanted, got) in pairs {
            let differs = wanted != got;
            let first = differs.then(|| first_difference(&wanted, &got));
            let entries = if name == "scripts:raids:icons" { wanted_raids.icons.len().max(1) as u64 } else { 1 };
            let notes = if name == "scripts:music" && differs { json!({"tables": tables, "refused_at_the_head_only": head_only}) } else { json!({"tables": tables}) };
            self.note(name.into(), (entries, u64::from(differs), first), notes, hour, why, out)?;
        }
        Ok(())
    }

    fn note(&mut self, name: String, (entries, differing, first): (u64, u64, Option<String>), notes: Value, hour: Hour, why: &str, out: &mut impl Write) -> io::Result<()> {
        let total = self.totals.entry(name.clone()).or_default();
        total.0 += entries;
        total.1 += differing;
        total.2 += 1;
        writeln!(out, "{}", json!({"hour": hour, "why": why, "compare": name, "entries": entries, "differing": differing, "first": first, "notes": notes}))
    }

    pub fn differing(&self) -> u64 {
        self.totals.values().map(|(_, differing, _)| differing).sum()
    }

    pub fn summary(&self, side: &MapSide, distribution: fn(Vec<f64>) -> Value) -> Value {
        let ms = |values: &[u64]| distribution(values.iter().map(|&ns| ns as f64 / 1e6).collect());
        let layer = viewer_layers_political::scripts::drawn(side.layer_parts());
        json!({
            "on": self.on,
            "compared": self.totals.iter().map(|(name, (entries, differing, samples))| json!({"compare": name, "samples": samples, "entries": entries, "differing": differing})).collect::<Vec<_>>(),
            "step_ms": ms(&self.step_ns), "steps": self.step_ns.len(),
            "icon_age_hours": distribution(self.age.clone()),
            "layer": viewer_layers_political::scripts::summary(side),
            "needs": layer.map(|layer| layer.needs().iter().map(|need| json!({"table": format!("{:?}", need.table), "every": need.columns == u64::MAX, "fields": (0..64).filter(|bit| need.columns & (1 << bit) != 0).filter_map(|bit| need.table.field_name(bit)).collect::<Vec<_>>()})).collect::<Vec<_>>()),
            "outside_reads": replay_frame::frame::outside_reads(),
        })
    }
}

impl Drop for Line {
    fn drop(&mut self) {
        self.entry.playhead().retire(self.reader);
    }
}

fn need(line: &str) -> Option<viewer_layer_core::playhead_host::views::Need> {
    use viewer_layer_core::playhead_host::views::Need;
    let (name, fields) = line.split_once(' ').map_or((line, None), |(name, fields)| (name, Some(fields.trim())));
    let table = replay_data::rows::TableId::ALL.into_iter().find(|table| format!("{table:?}") == name)?;
    match fields {
        None => Some(Need::keys(table)),
        Some("*") => Some(Need::every(table)),
        Some(fields) => fields.split(',').map(|field| (0..64).find(|&bit| table.field_name(bit) == Some(field.trim())).map(|bit| 1_u64 << bit)).sum::<Option<u64>>().map(|mask| Need::columns(table, mask)),
    }
}

fn first_difference(wanted: &str, got: &str) -> String {
    let at = wanted.bytes().zip(got.bytes()).position(|(a, b)| a != b).unwrap_or(wanted.len().min(got.len()));
    let around = |text: &str| {
        let start = text.char_indices().map(|(at, _)| at).take_while(|&from| from + 160 <= at).last().unwrap_or(0);
        text[start..].chars().take(400).collect::<String>()
    };
    format!("at byte {at} of {} / {}: today {} / at the head {}", wanted.len(), got.len(), around(wanted), around(got))
}
