
use crate::{
    document::Document,
    engaged_index::{
        fold_range,
        lanes::{self, PieceStart},
    },
};
use playhead::{Timeline, pages::Pages};
use replay_data::{BranchId, Hour};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
use viewer_engagements::engagement_history::{Covered, History};
use viewer_foundation::perspective::Country;

fn differ(a: &History, b: &History) -> (BTreeMap<&'static str, u64>, Option<String>, u64) {
    let (a, b) = (a.branches.lock().unwrap_or_else(|p| p.into_inner()), b.branches.lock().unwrap_or_else(|p| p.into_inner()));
    let mut counts: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut first = None;
    let mut entries = 0;
    let empty = Covered::new(0);
    for branch in a.keys().chain(b.keys()).collect::<BTreeSet<_>>() {
        let (x, y) = (a.get(branch).unwrap_or(&empty), b.get(branch).unwrap_or(&empty));
        let mut note = |what: &'static str, detail: String| {
            *counts.entry(what).or_default() += 1;
            first.get_or_insert(format!("branch {branch} {what}: {detail}"));
        };
        if (x.first, x.last) != (y.first, y.last) {
            note("hours", format!("{}..{} against {}..{}", x.first, x.last, y.first, y.last));
        }
        if x.viewers != y.viewers {
            note("viewers", format!("{:?} against {:?}", x.viewers, y.viewers));
        }
        if x.players != y.players {
            note("players", format!("{:?} against {:?}", x.players, y.players));
        }
        for (what, xs, ys) in [("sides", x.folds()[0].points(), y.folds()[0].points()), ("recording player's sides", x.folds()[1].points(), y.folds()[1].points())] {
            for (kind, (p, q)) in xs.iter().zip(ys).enumerate() {
                for key in p.keys().chain(q.keys()).collect::<BTreeSet<_>>() {
                    entries += 1;
                    if p.get(key) != q.get(key) {
                        note(what, format!("kind {kind} side {key:?}: {:?} against {:?}", p.get(key), q.get(key)));
                    }
                }
            }
        }
        for (what, xs, ys) in [("icon frames", x.folds()[0].icons(), y.folds()[0].icons()), ("recording player's icon frames", x.folds()[1].icons(), y.folds()[1].icons())] {
            for (kind, (p, q)) in xs.iter().zip(ys).enumerate() {
                for key in p.keys().chain(q.keys()).collect::<BTreeSet<_>>() {
                    entries += 1;
                    if p.get(key) != q.get(key) {
                        note(what, format!("kind {kind} combat {key}: {:?} against {:?}", p.get(key), q.get(key)));
                    }
                }
            }
        }
    }
    (counts, first, entries)
}

fn points(history: &History) -> (usize, usize) {
    let branches = history.branches.lock().unwrap_or_else(|p| p.into_inner());
    let mut out = (0, 0);
    for covered in branches.values() {
        for fold in covered.folds() {
            for kind in fold.points() {
                out.0 += kind.len();
                out.1 += kind.values().map(Vec::len).sum::<usize>();
            }
            for kind in fold.icons() {
                out.1 += kind.values().map(Vec::len).sum::<usize>();
            }
        }
    }
    out
}

pub(super) struct Folded {
    pub lanes: Arc<History>,
    pub today: Arc<History>,
    pub branch: BranchId,
}

pub(super) fn run(document: &Document, pages: &Arc<dyn Pages>, timeline: &Timeline, viewers: &str, last: Hour) -> io::Result<(Value, Option<Folded>)> {
    let replay = document.playback.replay();
    let first = timeline.first();
    let branch = document.playback.branch();
    let mut results = Vec::new();
    let mut folded = None;
    for name in viewers.split(',') {
        let country = if name == "recorded" { Country::Recorded } else { Country::Tag(name.to_owned()) };
        let (today, today_read, today_fold, today_failed) = fold_range(replay, country.clone(), timeline, branch, first, last);
        let shared = Arc::new(History::default());
        let started = |_: usize| PieceStart { branch, load: false, restart: false };
        let mut held = Vec::new();
        let spent = lanes::fold(pages, timeline, &started, Some(&replay.campaign_id), country, Arc::clone(&shared), Some((first, last)), |hour, store, spent| {
            if spent.hours % 1000 == 1 {
                let (rows, lanes) = store.held();
                held.push(json!({"hour": hour, "lane_hours_stepped_skipped": store.steps(), "rows": rows, "cells": lanes.cells, "parts": lanes.parts, "part_bytes": lanes.part_bytes, "tracks": lanes.tracks, "track_bytes": lanes.track_bytes, "sides_and_points": points(&shared)}));
            }
        })?;
        let (counts, first_differing, entries) = differ(&shared, &today);
        let hours = spent.hours.max(1) as f64;
        let today_hours = hours;
        results.push(json!({
            "viewer": name, "hours": spent.hours, "entries": entries, "differing": counts, "first": first_differing,
            "lanes": {"hours_a_second": hours / ((spent.read_ns + spent.fold_ns) as f64 / 1e9), "read_us_an_hour": spent.read_ns as f64 / hours / 1e3, "fold_us_an_hour": spent.fold_ns as f64 / hours / 1e3},
            "today": {"hours_a_second": today_hours / ((today_read + today_fold) as f64 / 1e9), "read_us_an_hour": today_read as f64 / today_hours / 1e3, "fold_us_an_hour": today_fold as f64 / today_hours / 1e3, "blocks_unread": today_failed},
            "held": held, "sides_and_points": points(&shared),
        }));
        eprintln!("{}", results.last().expect("pushed"));
        if name == "recorded" {
            folded = Some(Folded { lanes: shared, today, branch });
        }
    }
    Ok((Value::Array(results), folded))
}
