import { MAX_PLAYBACK_AGE_HOURS, playbackPace } from './verdicts.mjs';

const shownFrom = r => Math.max(r.at, r.shownAt ?? r.at);
const p95 = values => values.length ? values.toSorted((a, b) => a - b)[Math.ceil(values.length * .95) - 1] : null;
const drawCost = (reports, id) => {
  const ms = reports.filter(r => r.placement === id && !r.detached && Number.isFinite(r.drawMs)).map(r => r.drawMs);
  return ms.length ? { meanMs: ms.reduce((a, b) => a + b, 0) / ms.length, p95Ms: p95(ms), maxMs: Math.max(...ms) } : null;
};
const gaps = times => times.slice(1).map((at, i) => at - times[i]);
const rate = times => times.length < 2 ? null : 1000 * (times.length - 1) / (times.at(-1) - times[0]);
const mean = xs => xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : null;
const WORKER_GRACE_MS = 34;
const RENDERER_OWNER = { table: -1, race: -2 };
const rendered = sample => !(sample.surface in RENDERER_OWNER) || sample.owner === RENDERER_OWNER[sample.surface];
export const CANONICAL_INPUTS = Object.freeze(['rewind', 'scrub', 'pin', 'speed', 'scroll', 'pause']);

export function inputPictures(frames, visibility, records, pinWidget) {
  const events = records.filter(r => r.t === 'picture' || r.t === 'tick' || r.t === 'ink' || r.t === 'workerFrame').toSorted((a, b) => a.at - b.at);
  const states = new Map(), pictures = new Map(), hidden = new Set(), examples = [];
  const lanes = new Set(events.filter(r => r.t === 'workerFrame' && r.at >= frames[0] && r.at <= frames.at(-1)).map(r => r.lane ?? 0));
  const hasWorkerFrames = lanes.size > 0, applied = new Set();
  const settledBeforeSeek = new Set(), judged = new Set();
  let heldAtPause = new Map();
  let i = 0, v = 0, anchor, shown = [], initial = null, workerApplied = !hasWorkerFrames,
    checked = 0, future = 0, pending = 0, wrongSeek = 0, stale = 0, lost = 0;
  for (const at of frames) {
    while (v < visibility.length && visibility[v].at <= at) shown = visibility[v++].shown;
    while (i < events.length && events[i].at <= at) {
      const r = events[i++];
      if (r.t === 'tick') {
        const pause = !r.playing && (!anchor || anchor.playing || anchor.seek !== r.seek || Math.floor(anchor.hour) !== Math.floor(r.hour));
        const newSeek = anchor && Number.isFinite(anchor.seek) && Number.isFinite(r.seek) && anchor.seek !== r.seek;
        if (pause) heldAtPause = new Map([...pictures].map(([id, picture]) => [id, picture.face ?? picture.entry]));
        if (hasWorkerFrames && pause) {
          if (newSeek) for (const [id, state] of states) if (state.settled) settledBeforeSeek.add(id);
          workerApplied = false; applied.clear(); states.clear();
        }
        anchor = r;
      }
      else if (r.t === 'workerFrame') {
        if (anchor && r.seek === anchor.seek && r.playing === anchor.playing &&
          (anchor.playing || Math.floor(r.head) === Math.floor(anchor.hour))) applied.add(r.lane ?? 0);
        workerApplied ||= [...lanes].every(lane => applied.has(lane));
      }
      else if (r.t === 'ink') { if (r.visible) hidden.delete(r.placement); else hidden.add(r.placement); }
      else if (r.detached) { pictures.delete(r.placement); states.delete(r.placement); }
      else pictures.set(r.placement, r);
    }
    if (!anchor) continue;
    const head = anchor.hour + (anchor.playing ? Math.max(0, at - (anchor.composedAt ?? anchor.at)) * anchor.speed / 1000 : 0);
    const ahead = (id, pin) => {
      const r = hidden.has(id) ? undefined : pictures.get(id);
      return Number.isFinite(r?.hour) && r.hour > Math.floor(pin ?? head);
    };
    if (!workerApplied && at - anchor.at <= WORKER_GRACE_MS) continue;
    const visible = new Set(shown.map(([id]) => id));
    if (initial === null && visible.size > 0) initial = visible;
    if (initial === null) continue;
    for (const id of states.keys()) if (!visible.has(id)) states.delete(id);
    for (const [id, pin, widget] of shown) {
      judged.add(id);
      const r = hidden.has(id) ? undefined : pictures.get(id), target = Math.floor(pin ?? head);
      const fixed = pin != null || !anchor.playing;
      const key = [anchor.seek, pin, fixed ? target : 'playing'].join('/');
      const previous = states.get(id);
      const state = previous?.key === key ? previous : { key, pin, fixed, settled: false, invalid: false, reportedInvalid: false, held: anchor.playing ? undefined : heldAtPause.get(id) };
      states.set(id, state);
      const ink = Number.isFinite(r?.hour);
      if (ink) {
        checked++;
        if (r.seek !== anchor.seek || !Number.isFinite(anchor.seek)) wrongSeek++;
        if (r.hour > target) future++;
      } else pending++;
      if (hidden.has(id)) continue;
      const valid = ink && r.seek === anchor.seek && Number.isFinite(r.seek) && (r.fixedHour ?? null) === pin &&
        r.since <= r.hour && r.hour <= r.through && (fixed ? r.hour === target : r.hour <= target);
      const held = fixed && ink && !valid && r.dim === true && r.hour <= target && state.held !== undefined && (r.face ?? r.entry) === state.held;
      if (settledBeforeSeek.has(id) && fixed && ink && !valid && !held) { stale++; settledBeforeSeek.delete(id); }
      if (fixed && ink && !valid && !held) state.invalid = true;
      if (fixed && state.settled && ink && !valid) stale++;
      if (fixed && state.settled && !valid) lost++;
      if (!valid && examples.length < 10) examples.push({ at, id, widget, target, hour: r?.hour, seek: r?.seek, expectedSeek: anchor.seek });
      if (valid && state.invalid && !state.reportedInvalid) { stale++; state.reportedInvalid = true; }
      state.settled ||= valid;
    }
  }
  const missing = [...states].filter(([, state]) => !state.settled).map(([id]) => id);
  const pinned = !pinWidget || shown.some(([, pin, widget]) => pin != null && widget === pinWidget);
  return { pass: frames.length > 0 && judged.size > 0 && future === 0 && !wrongSeek && !stale && !lost && !missing.length && pinned,
    checked, future, wrongSeek, stale, lost, missing, pending, pinned, examples };
}

export function datedPlaybackVerdict(windows, records, { legacy = false } = {}) {
  const reasons = [], metrics = [], placements = [];
  const sorted = records.toSorted((a, b) => a.at - b.at);
  if (!windows.length) reasons.push('nothing measured');
  for (const w of windows) {
    const visibleWidgets = (w.widgets ?? []).filter(widget => widget.visible);
    if (!visibleWidgets.length) reasons.push('no visible dashboard placements');
    if (!visibleWidgets.length || visibleWidgets.every(widget => widget.notMigrated)) reasons.push('nothing measured');
    const ticks = sorted.filter(r => r.t === 'tick' && r.playing && r.speed === 1440 && r.at >= w.from && r.at <= w.to);
    const first = ticks[0], last = ticks.at(-1);
    const inside = at => at >= first?.at && at <= last?.at;
    const presented = (w.presented ?? []).filter(inside);
    const end = presented.findIndex(at => at >= presented[0] + w.seconds * 1000);
    const frames = end < 0 ? presented : presented.slice(0, end + 1);
    if (end < 0 || !w.seconds || !w.complete) reasons.push('incomplete presented playing window');
    const timed = r => r.at >= frames[0] && r.at <= frames.at(-1);
    const seconds = (frames.at(-1) - frames[0]) / 1000;
    const page = sorted.filter(r => r.t === 'frame' && r.playing && timed(r));
    const worker = sorted.filter(r => r.t === 'workerFrame' && r.playing && timed(r));
    const laneCount = Math.max(0, ...worker.map(r => r.lanes ?? (r.lane ?? 0) + 1));
    const byLane = Map.groupBy(worker, r => r.lane ?? 0);
    const lanes = Array.from({ length: laneCount }, (_, lane) => byLane.has(lane) && seconds > 0 ? (byLane.get(lane).length - 1) / seconds : null);
    const fps = rate(frames), pageFps = rate(page.map(r => r.at)), presents = !lanes.length || lanes.includes(null) ? null : Math.min(...lanes);
    if (fps === null) reasons.push('nothing measured');
    const pace = playbackPace(first, last, 1440);
    if (!pace.pass) reasons.push('clock did not advance at 1,440 h/s');
    if (fps === null || fps < 60) reasons.push('presented FPS ' + fps);
    if (pageFps === null) reasons.push('missing page frame measurements');
    const outOfBounds = lanes.filter(lane => lane === null || Math.round(lane) < 60 || Math.round(lane) > Math.max(60, Math.round(fps ?? 0)));
    if (!lanes.length || outOfBounds.length) reasons.push('worker presents/s ' + lanes.map(lane => lane === null ? 'none' : lane.toFixed(1)).join(', '));
    if (w.pressureMerges !== 0) reasons.push('pressure merges ' + w.pressureMerges);
    const pageCount = page.reduce((sum, r) => sum + r.commits, 0);
    const firstCommit = page[0]?.commitSequence - page[0]?.commits + 1;
    const counted = sorted.filter(r => r.t === 'commit' && (Number.isFinite(firstCommit) ? r.serial >= firstCommit && r.serial <= page.at(-1)?.commitSequence : timed(r)));
    const commits = sorted.filter(r => r.t === 'commit' && timed(r));
    const commitCount = commits.length;
    const verified = c => c.verified && Number.isFinite(c.hour) && JSON.stringify(c.before) !== JSON.stringify(c.after);
    const boundariesByPlacement = new Map();
    const picturesByWidget = new Map(), boundariesByWidget = new Map(), visibilityByWidget = new Map(), auditByWidget = new Map();
    for (const event of w.visibility ?? []) {
      if (!visibilityByWidget.has(event.id)) visibilityByWidget.set(event.id, []);
      visibilityByWidget.get(event.id).push(event);
    }
    for (const sample of w.audit ?? []) {
      if (!auditByWidget.has(sample.widget)) auditByWidget.set(sample.widget, []);
      auditByWidget.get(sample.widget).push(sample);
    }
    for (const r of sorted) {
      if (r.t === 'picture' && r.at >= (w.seekStart ?? w.from) && r.at <= w.to) {
        if (!picturesByWidget.has(r.widget)) picturesByWidget.set(r.widget, []);
        picturesByWidget.get(r.widget).push(r);
      }
      if (r.t !== 'commit' || r.at < w.from || r.at > w.to) continue;
      for (const c of r.changes ?? []) {
        if (!verified(c)) continue;
        if (Number.isFinite(c.placement)) {
          if (!boundariesByPlacement.has(c.placement)) boundariesByPlacement.set(c.placement, []);
          boundariesByPlacement.get(c.placement).push({ hour: c.hour, previousHour: c.previousHour });
          continue;
        }
        if (!boundariesByWidget.has(c.widget)) boundariesByWidget.set(c.widget, new Map());
        const boundaries = boundariesByWidget.get(c.widget);
        if (!boundaries.has(c.source)) boundaries.set(c.source, []);
        boundaries.get(c.source).push({ hour: c.hour, previousHour: c.previousHour });
      }
    }
    const structural = commits.filter(r => r.widgets?.length && r.widgets.every(widget => r.changes?.some(c => c.widget === widget && verified(c))));
    const input = commits.filter(r => r.input && !structural.includes(r));
    const unexplained = commitCount - structural.length - input.length;
    if (unexplained !== 0) reasons.push('React commits without verified structure or input ' + unexplained);
    if (counted.length !== pageCount) reasons.push('React commit audit does not reconcile with the page frame counts');
    const idle = w.idleCost ? sorted.filter(r => r.t === 'workerFrame' && r.playing && r.changed === 0 && r.at >= w.idleCost.from && r.at <= w.idleCost.to) : worker.filter(r => r.changed === 0);
    const emptyMs = mean(idle.map(r => r.workMs));
    const changing = worker.filter(r => r.changed > 0);
    const changedWidgetMs = emptyMs === null ? null : mean(changing.map(r => (r.workMs - emptyMs) / r.changed));
    const structureBySource = {};
    for (const r of structural) for (const source of new Set(r.changes.filter(verified).map(c => c.source))) structureBySource[source] = (structureBySource[source] ?? 0) + 1;
    metrics.push({ fps, pageFps, presents, hps: pace.hps, commitsPerSecond: commitCount / seconds, structureCommitsPerSecond: structural.length / seconds,
      inputCommitsPerSecond: input.length / seconds, unexplainedCommitsPerSecond: unexplained / seconds, structureBySource, frames: frames.length,
      compositorGapP95: p95(gaps(frames)), workerGapP95: p95(gaps(worker.map(r => r.at))), emptyMs, emptyFrames: idle.length,
      changedWidgetMs, changingFrames: changing.length, pressureMerges: w.pressureMerges });
    const hasRecordedInputs = Array.isArray(w.inputs?.actions) && w.inputs.actions.length > 0;
    if (w.inputs && (!legacy || hasRecordedInputs)) {
      const checks = [];
      const actions = Array.isArray(w.inputs.actions) ? w.inputs.actions : [];
      const presentedInputs = Array.isArray(w.inputs.presented) ? w.inputs.presented : [];
      let pinWidget;
      for (const [i, action] of actions.entries()) {
        const notApplicable = typeof action.notApplicable === 'string' && action.notApplicable.trim() ? action.notApplicable.trim() : null;
        if (notApplicable) { checks.push({ kind: action.kind, notApplicable, pass: true }); continue; }
        const times = presentedInputs.filter(at => at >= action.at && at < (actions[i + 1]?.at ?? w.inputs.to));
        pinWidget = action.pinWidget ?? pinWidget;
        checks.push({ kind: action.kind, ...inputPictures(times, w.inputs.visibility ?? [], sorted, pinWidget) });
      }
      for (const kind of CANONICAL_INPUTS) {
        const ofKind = checks.filter(c => c.kind === kind);
        const checked = ofKind.some(c => !c.notApplicable && c.pass &&
          (kind !== 'pin' || actions.some(action => action.kind === kind && action.pinWidget)));
        const notApplicable = ofKind.length > 0 && ofKind.every(c => c.notApplicable);
        if (!checked && !notApplicable) reasons.push('Missing input coverage for ' + kind);
      }
      if (checks.some(c => c.future > 0)) reasons.push('Future picture during dashboard input sequence');
      const rewindUnsettled = Array.isArray(w.inputs.rewindUnsettled) ? w.inputs.rewindUnsettled : [];
      if (rewindUnsettled.length) {
        const details = rewindUnsettled.map(entry => typeof entry === 'string' ? entry : `${entry.widget}: ${entry.reason}`).join('; ');
        reasons.push('Dashboard rewind did not settle: ' + details);
        checks.push(...rewindUnsettled.map(entry => typeof entry === 'string'
          ? { kind: 'rewind-settle', widget: entry, pass: false }
          : { kind: 'rewind-settle', widget: entry.widget, reason: entry.reason, pass: false }));
      }
      const unsettled = Array.isArray(w.inputs.unsettled) ? w.inputs.unsettled : [];
      if (unsettled.length) {
        const details = unsettled.map(entry => typeof entry === 'string' ? entry : `${entry.widget}: ${entry.reason}`).join('; ');
        reasons.push('Dashboard scrub did not settle: ' + details);
        checks.push(...unsettled.map(entry => typeof entry === 'string'
          ? { kind: 'scrub-settle', widget: entry, pass: false }
          : { kind: 'scrub-settle', widget: entry.widget, reason: entry.reason, pass: false }));
      }
      metrics.at(-1).inputs = checks;
      if (Number.isFinite(w.inputs.rewindSettledMs)) metrics.at(-1).rewindSettledMs = w.inputs.rewindSettledMs;
      if (Number.isFinite(w.inputs.scrubSettledMs)) metrics.at(-1).scrubSettledMs = w.inputs.scrubSettledMs;
    } else if (!legacy && (w.widgets ?? []).some(widget => widget.visible && !widget.notMigrated)) reasons.push('Missing dashboard input sequence');
    const expectedWidgets = (w.expectedWidgets ?? []).map(expected => typeof expected === 'string' ? { id: expected } : expected).filter(expected => expected?.id != null);
    const inventory = new Map((w.widgets ?? []).map(widget => [widget.id, widget]));
    for (const expected of expectedWidgets) if (!inventory.has(expected.id)) reasons.push('Missing expected widget inventory entry');
    const placementStart = placements.length;
    for (const widget of w.widgets ?? []) {
      if (!widget.visible) continue;
      if (widget.notMigrated) { placements.push({ from: w.from, page: w.page, widget: widget.id, title: widget.title, kind: widget.kind, scroll: w.scroll, status: 'not migrated', pass: null }); continue; }
      const reports = (picturesByWidget.get(widget.id) ?? []).toSorted((a, b) => shownFrom(a) - shownFrom(b));
      const declared = [...new Set((widget.surfaces ?? []).filter(surface => surface != null))];
      const visible = new Set(declared), required = new Set(visible);
      const visibility = visibilityByWidget.get(widget.id) ?? [];
      const changes = visibility.filter(e => e.surface != null).toSorted((a, b) => a.at - b.at);
      const placementChanges = visibility.filter(e => Number.isFinite(e.placement)).toSorted((a, b) => a.at - b.at);
      const placementMode = placementChanges.length > 0;
      const visiblePlacements = new Set();
      const held = new Map(), hours = new Set(), ages = [], eligibleAges = [], observedSurfaces = new Set(), observedPairs = new Set(), byPlacement = new Map();
      const boundaries = boundariesByWidget.get(widget.id) ?? new Map();
      const surfaceNames = [...new Set([...declared, ...changes.map(e => e.surface)].filter(Boolean))];
      const boundarySurfaces = new Map();
      const boundaryErrors = [];
      for (const source of boundaries.keys()) {
        const normal = String(source).toLowerCase().replace(/[^a-z0-9]/g, '');
        const exact = surfaceNames.filter(surface => surface === source);
        const matches = exact.length ? exact : surfaceNames.filter(surface =>
          String(surface).toLowerCase().replace(/[^a-z0-9]/g, '') === normal);
        if (matches.length === 1) boundarySurfaces.set(source, matches);
        else if (!matches.length) boundaryErrors.push(`Unmapped commit source "${source}" for widget "${widget.id}"`);
        else boundaryErrors.push(`Ambiguous commit source "${source}" for widget "${widget.id}" matches ${matches.join(', ')}`);
      }
      const boundariesBySurface = new Map();
      for (const [source, hoursForSource] of boundaries) {
        for (const surface of boundarySurfaces.get(source) ?? [source])
          boundariesBySurface.set(surface, [...(boundariesBySurface.get(surface) ?? []), ...hoursForSource]);
      }
      let change = 0, placementChange = 0, report = 0, tick = 0, future = 0, missing = 0, late = 0, lateWithoutCrossing = 0, structureCrossings = 0;
      for (const r of reports) if (Number.isFinite(r.hour) && r.hour > Math.floor(r.fixedHour ?? r.head)) future++;
      for (const at of frames) {
        while (report < reports.length && shownFrom(reports[report]) <= at) { const r = reports[report++]; if (r.detached) held.delete(r.placement); else held.set(r.placement, r); }
        while (change < changes.length && changes[change].at <= at) { const e = changes[change++]; if (e.visible) { visible.add(e.surface); required.add(e.surface); } else visible.delete(e.surface); }
        while (placementChange < placementChanges.length && placementChanges[placementChange].at <= at) {
          const e = placementChanges[placementChange++]; if (e.visible) visiblePlacements.add(e.placement); else visiblePlacements.delete(e.placement);
        }
        while (tick + 1 < ticks.length && ticks[tick + 1].at <= at) tick++;
        const anchor = ticks[tick], head = Math.floor(anchor.hour + Math.max(0, at - (anchor.composedAt ?? anchor.at)) * anchor.speed / 1000);
        const seen = new Set(), seenSurfaces = new Set(), lag = [], drawn = [], available = [];
        let crossed = false, uncrossedLate = false;
        for (const r of held.values()) {
          if ((placementMode ? !visiblePlacements.has(r.placement) : !visible.has(r.surface)) || r.renderer !== 'page') continue;
          seen.add(r.placement); seenSurfaces.add(r.surface); observedSurfaces.add(r.surface); observedPairs.add(`${r.surface}\u0000${r.placement}`);
          const target = r.fixedHour ?? head, hour = r.hour, eligibleAge = Number.isFinite(r.eligibleHour) ? target - r.eligibleHour : Infinity;
          available.push(eligibleAge);
          if (eligibleAge < 0 || r.eligibleHour > r.head) future++;
          if (!byPlacement.has(r.placement)) byPlacement.set(r.placement, { surface: r.surface, drawn: [], eligible: [] });
          const samples = byPlacement.get(r.placement); samples.drawn.push(Number.isFinite(hour) ? target - hour : Infinity); samples.eligible.push(eligibleAge);
          let crossedPlacement = false;
          if (Number.isFinite(hour) && target - hour > MAX_PLAYBACK_AGE_HOURS)
            crossedPlacement = (boundariesByPlacement.get(r.placement) ?? boundariesBySurface.get(r.surface))?.some(boundary => {
              const displayed = hour + (r.offset ?? 0), upper = boundary.hour + (r.offset ?? 0);
              if (Number.isFinite(boundary.previousHour)) return boundary.previousHour + (r.offset ?? 0) < displayed && displayed <= upper;
              return eligibleAge <= MAX_PLAYBACK_AGE_HOURS && displayed < upper && upper <= r.eligibleHour + (r.offset ?? 0);
            }) ?? false;
          crossed ||= crossedPlacement;
          if (Number.isFinite(hour) && target - hour > MAX_PLAYBACK_AGE_HOURS && !crossedPlacement) uncrossedLate = true;
          if (Number.isFinite(hour)) { drawn.push(hour); if (hour > target) future++; lag.push(target - hour); } else lag.push(Infinity);
        }
        const expected = placementMode ? visiblePlacements.size : visible.size;
        const visibleCount = placementMode ? seen.size : seenSurfaces.size;
        if (!lag.length || visibleCount !== expected || lag.includes(Infinity)) { missing++; ages.push(Infinity); } else { const worst = Math.max(...lag); ages.push(worst); hours.add(Math.min(...drawn)); if (worst > MAX_PLAYBACK_AGE_HOURS) { late++; if (uncrossedLate) lateWithoutCrossing++; } if (crossed) structureCrossings++; }
        eligibleAges.push(available.length && visibleCount === expected ? Math.max(...available) : Infinity);
      }
      const audited = auditByWidget.get(widget.id) ?? [], samples = audited.filter(s => Number.isFinite(s.at) && timed(s));
      const auditedPairs = new Set(samples.filter(rendered).map(s => `${s.surface}\u0000${s.placement}`));
      const exact = required.size > 0 && [...required].every(surface => observedSurfaces.has(surface))
        && [...required].every(surface => samples.some(s => s.surface === surface && rendered(s)))
        && [...observedPairs].every(pair => auditedPairs.has(pair)) && audited.every(s => s.error === null);
      const age = p95(ages), eligible = p95(eligibleAges);
      const surfaces = [...byPlacement].map(([id, s]) => ({ id, surface: s.surface, p95: p95(s.drawn), eligibleP95: p95(s.eligible), frames: s.drawn.length, draw: drawCost(reports, id) }));
      const pass = Number.isFinite(age) && age <= MAX_PLAYBACK_AGE_HOURS && Number.isFinite(eligible) && eligible <= MAX_PLAYBACK_AGE_HOURS &&
        surfaces.every(s => Number.isFinite(s.p95) && s.p95 <= MAX_PLAYBACK_AGE_HOURS && Number.isFinite(s.eligibleP95) && s.eligibleP95 <= MAX_PLAYBACK_AGE_HOURS) &&
        future === 0 && missing === 0 && exact && boundaryErrors.length === 0;
      placements.push({ from: w.from, page: w.page, widget: widget.id, title: widget.title, kind: widget.kind, scroll: w.scroll, p95: Number.isFinite(age) ? age : null,
        eligibleP95: Number.isFinite(eligible) ? eligible : null, surfaces, samples: samples.length, distinctHoursPerSecond: hours.size / seconds,
        missing, late, lateWithoutCrossing, structureCrossings, future, exact, boundaryErrors, pass });
      reasons.push(...boundaryErrors);
    }
    for (const expected of expectedWidgets) {
      if (!inventory.has(expected.id)) continue;
      if (!placements.slice(placementStart).some(placement => placement.widget === expected.id)) reasons.push('Missing judged placement for expected widget');
    }
  }
  const kinds = [...new Set(placements.map(p => p.kind))];
  const widgets = kinds.map(kind => {
    const kept = placements.filter(p => p.kind === kind), migrated = kept.filter(p => p.status !== 'not migrated');
    const pass = migrated.length === 0 ? null : migrated.every(p => p.pass);
    if (pass === false) reasons.push('A widget missed its dated presentation or exact-read target');
    return { kind, p95: migrated.length && migrated.every(p => p.p95 !== null) ? Math.max(...migrated.map(p => p.p95)) : null,
      eligibleP95: migrated.length && migrated.every(p => p.eligibleP95 !== null) ? Math.max(...migrated.map(p => p.eligibleP95)) : null,
      structureCrossings: migrated.reduce((sum, p) => sum + p.structureCrossings, 0), future: migrated.reduce((sum, p) => sum + p.future, 0),
      exact: migrated.length > 0 && migrated.every(p => p.exact),
      ...(kept.some(p => p.status === 'not migrated') && { notMigrated: kept.filter(p => p.status === 'not migrated').length }), pass };
  });
  return { pass: reasons.length === 0, reasons, metrics, widgets, placements };
}
