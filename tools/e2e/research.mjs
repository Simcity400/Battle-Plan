import { MAX_PLAYBACK_AGE_HOURS, playbackPace } from './verdicts.mjs';

const KINDS = ['active-research', 'research', 'special-projects', 'nuclear-arsenal', 'doctrine-mastery'];
const p95 = values => values.length ? values.toSorted((a, b) => a - b)[Math.ceil(values.length * .95) - 1] : null;
const gaps = times => times.slice(1).map((at, i) => at - times[i]);
const rate = times => times.length < 2 ? null : 1000 * (times.length - 1) / (times.at(-1) - times[0]);
const mean = xs => xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : null;
/** Two frames at 60 Hz: the worker's own presentation of a new anchor. */
const WORKER_GRACE_MS = 34;

export function inputPictures(frames, visibility, records, pinWidget) {
  const events = records.filter(r => r.t === 'picture' || r.t === 'tick' || r.t === 'ink' || r.t === 'workerFrame').toSorted((a, b) => a.at - b.at);
  const states = new Map(), pictures = new Map(), hidden = new Set(), examples = [];
  const hasWorkerFrames = events.some(r => r.t === 'workerFrame' && r.at >= frames[0] && r.at <= frames.at(-1));
  const judged = new Set();
  let i = 0, v = 0, anchor, shown = [], initial = null, workerApplied = !hasWorkerFrames,
    checked = 0, future = 0, pending = 0, wrongSeek = 0, stale = 0, lost = 0;
  for (const at of frames) {
    while (v < visibility.length && visibility[v].at <= at) shown = visibility[v++].shown;
    while (i < events.length && events[i].at <= at) {
      const r = events[i++];
      if (r.t === 'tick') {
        const pause = !r.playing && (!anchor || anchor.playing || anchor.seek !== r.seek || Math.floor(anchor.hour) !== Math.floor(r.hour));
        if (hasWorkerFrames && pause) { workerApplied = false; states.clear(); }
        anchor = r;
      }
      else if (r.t === 'workerFrame') {
        if (anchor && r.seek === anchor.seek && r.playing === anchor.playing &&
          (anchor.playing || Math.floor(r.head) === Math.floor(anchor.hour))) workerApplied = true;
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
    // The worker has WORKER_GRACE_MS to present a new anchor; after that every check applies whether or not it has.
    if (!workerApplied && at - anchor.at <= WORKER_GRACE_MS) continue;
    const visible = new Set(shown.map(([id]) => id));
    // A seek can briefly unmount every canvas before the held position remounts. Do not
    // make that transition frame the baseline for the input's presented placement set.
    if (initial === null && visible.size > 0) initial = visible;
    if (initial === null) continue;
    for (const id of states.keys()) if (!visible.has(id)) states.delete(id);
    for (const [id, pin, widget] of shown) {
      if (!initial.has(id) && pin == null) { if (ahead(id, pin)) future++; continue; }
      judged.add(id);
      const r = hidden.has(id) ? undefined : pictures.get(id), target = Math.floor(pin ?? head);
      const fixed = pin != null || !anchor.playing;
      const key = [anchor.seek, pin, fixed ? target : 'playing'].join('/');
      const previous = states.get(id);
      const state = previous?.key === key ? previous : { key, settled: false };
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
      if (fixed && ink && !valid) stale++;
      if (fixed && state.settled && !valid) lost++;
      if (!valid && examples.length < 10) examples.push({ at, id, widget, target, hour: r?.hour, seek: r?.seek, expectedSeek: anchor.seek });
      state.settled ||= valid;
    }
  }
  const missing = [...states].filter(([, state]) => !state.settled).map(([id]) => id);
  const pinned = !pinWidget || shown.some(([, pin, widget]) => pin != null && widget === pinWidget);
  return { pass: frames.length > 0 && judged.size > 0 && future === 0 && !wrongSeek && !stale && !lost && !missing.length && pinned,
    checked, future, wrongSeek, stale, lost, missing, pending, pinned, examples };
}

export function researchVerdict(windows, records) {
  const reasons = [], metrics = [], placements = [];
  const sorted = records.toSorted((a, b) => a.at - b.at);
  for (const w of windows) {
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
    const fps = rate(frames), pageFps = rate(page.map(r => r.at)), presents = rate(worker.map(r => r.at));
    const pace = playbackPace(first, last, 1440);
    if (!pace.pass) reasons.push('clock did not advance at 1,440 h/s');
    if (fps === null || fps < 60) reasons.push('presented FPS ' + fps);
    if (pageFps === null) reasons.push('missing page frame measurements');
    if (presents === null || Math.round(presents) !== 60) reasons.push('worker presents/s ' + presents);
    if (w.pressureMerges !== 0) reasons.push('pressure merges ' + w.pressureMerges);
    const pageCount = page.reduce((sum, r) => sum + r.commits, 0);
    const firstCommit = page[0]?.commitSequence - page[0]?.commits + 1;
    const counted = sorted.filter(r => r.t === 'commit' && (Number.isFinite(firstCommit) ?
      r.serial >= firstCommit && r.serial <= page.at(-1)?.commitSequence : timed(r)));
    const commits = sorted.filter(r => r.t === 'commit' && timed(r));
    const commitCount = commits.length;
    const verified = c => c.verified && Number.isFinite(c.hour) && JSON.stringify(c.before) !== JSON.stringify(c.after);
    const structural = commits.filter(r => r.widgets?.length && r.widgets.every(widget =>
      r.changes?.some(c => c.widget === widget && verified(c))));
    const input = commits.filter(r => r.input && !structural.includes(r));
    const unexplained = commitCount - structural.length - input.length;
    if (unexplained !== 0) reasons.push('React commits without verified structure or input ' + unexplained);
    if (counted.length !== pageCount) reasons.push('React commit audit does not reconcile with the page frame counts');
    const idle = w.idleCost ? sorted.filter(r => r.t === 'workerFrame' && r.playing && r.changed === 0
      && r.at >= w.idleCost.from && r.at <= w.idleCost.to) : worker.filter(r => r.changed === 0);
    const emptyMs = mean(idle.map(r => r.workMs));
    const changing = worker.filter(r => r.changed > 0);
    const changedWidgetMs = emptyMs === null ? null : mean(changing.map(r => (r.workMs - emptyMs) / r.changed));
    const structureBySource = {};
    for (const r of structural) for (const source of new Set(r.changes.filter(verified).map(c => c.source)))
      structureBySource[source] = (structureBySource[source] ?? 0) + 1;
    metrics.push({ fps, pageFps, presents, hps: pace.hps, commitsPerSecond: commitCount / seconds, structureCommitsPerSecond: structural.length / seconds,
      inputCommitsPerSecond: input.length / seconds, unexplainedCommitsPerSecond: unexplained / seconds, structureBySource, frames: frames.length,
      compositorGapP95: p95(gaps(frames)), workerGapP95: p95(gaps(worker.map(r => r.at))), emptyMs, emptyFrames: idle.length,
      changedWidgetMs, changingFrames: changing.length, pressureMerges: w.pressureMerges });
    if (w.inputs) {
      const checks = [];
      let pinWidget;
      for (const [i, action] of w.inputs.actions.entries()) {
        const times = w.inputs.presented.filter(at => at >= action.at && at < (w.inputs.actions[i + 1]?.at ?? w.inputs.to));
        pinWidget = action.pinWidget ?? pinWidget;
        checks.push({ kind: action.kind, ...inputPictures(times, w.inputs.visibility, sorted, pinWidget) });
      }
      for (const kind of ['rewind','scrub','pin','speed','scroll','pause']) {
        if (!checks.some(c => c.kind === kind && c.pass && (kind !== 'pin' || w.inputs.actions.some(a => a.kind === kind && a.pinWidget)))) reasons.push('Missing input coverage for ' + kind);
      }
      if (checks.some(c => c.future > 0)) reasons.push('Future picture during Research input sequence');
      metrics.at(-1).inputs = checks;
    } else reasons.push('Missing Research input sequence');
    for (const widget of w.widgets ?? []) {
      if (!widget.visible || !KINDS.includes(widget.kind)) continue;
      const reports = sorted.filter(r => r.t === 'picture' && r.widget === widget.id && r.at >= (w.seekStart ?? w.from) && r.at <= w.to);
      const visible = new Set(widget.surfaces), required = new Set(visible);
      const changes = (w.visibility ?? []).filter(e => e.id === widget.id && e.surface != null).toSorted((a, b) => a.at - b.at);
      const held = new Map(), hours = new Set(), ages = [], eligibleAges = [], observed = new Set(), byPlacement = new Map();
      const boundaries = new Map();
      for (const r of sorted) if (r.t === 'commit' && r.at >= w.from && r.at <= w.to) for (const c of r.changes ?? []) {
        if (c.widget !== widget.id || !verified(c)) continue;
        const surface = c.source === 'project list' ? 'special-projects' : c.source;
        if (!boundaries.has(surface)) boundaries.set(surface, []);
        boundaries.get(surface).push(c.hour);
      }
      let change = 0, report = 0, tick = 0, future = 0, missing = 0, late = 0, structureCrossings = 0;
      for (const r of reports) if (Number.isFinite(r.hour) && r.hour > Math.floor(r.fixedHour ?? r.head)) future++;
      for (const at of frames) {
        while (report < reports.length && reports[report].at <= at) {
          const r = reports[report++];
          if (r.detached) held.delete(r.placement); else held.set(r.placement, r);
        }
        while (change < changes.length && changes[change].at <= at) {
          const e = changes[change++];
          if (e.visible) { visible.add(e.surface); required.add(e.surface); } else visible.delete(e.surface);
        }
        while (tick + 1 < ticks.length && ticks[tick + 1].at <= at) tick++;
        const anchor = ticks[tick];
        const head = Math.floor(anchor.hour + Math.max(0, at - (anchor.composedAt ?? anchor.at)) * anchor.speed / 1000);
        const seen = new Set(), lag = [], drawn = [], available = [];
        let crossed = false;
        for (const r of held.values()) {
          if (!visible.has(r.surface) || r.renderer !== 'worker') continue;
          seen.add(r.surface);
          observed.add(r.placement);
          const target = r.fixedHour ?? head;
          const hour = r.hour;
          const eligibleAge = Number.isFinite(r.eligibleHour) ? target - r.eligibleHour : Infinity;
          available.push(eligibleAge);
          if (eligibleAge < 0 || r.eligibleHour > r.head) future++;
          if (!byPlacement.has(r.placement)) byPlacement.set(r.placement, { surface: r.surface, drawn: [], eligible: [] });
          const samples = byPlacement.get(r.placement);
          samples.drawn.push(Number.isFinite(hour) ? target - hour : Infinity);
          samples.eligible.push(eligibleAge);
          if (Number.isFinite(hour) && target - hour > MAX_PLAYBACK_AGE_HOURS && eligibleAge <= MAX_PLAYBACK_AGE_HOURS)
            crossed ||= boundaries.get(r.surface)?.some(h => hour + (r.offset ?? 0) < h && h <= r.eligibleHour + (r.offset ?? 0));
          if (Number.isFinite(hour)) { drawn.push(hour); if (hour > target) future++; lag.push(target - hour); }
          else lag.push(Infinity);
        }
        if (!lag.length || seen.size !== visible.size || lag.includes(Infinity)) { missing++; ages.push(Infinity); }
        else {
          const worst = Math.max(...lag);
          ages.push(worst); hours.add(Math.min(...drawn));
          if (worst > MAX_PLAYBACK_AGE_HOURS) late++;
          if (crossed) structureCrossings++;
        }
        eligibleAges.push(available.length && seen.size === visible.size ? Math.max(...available) : Infinity);
      }
      const samples = (w.audit ?? []).filter(s => s.widget === widget.id && required.has(s.surface));
      const exact = required.size > 0 && [...required].every(surface => samples.some(s => s.surface === surface)) &&
        [...observed].every(id => samples.some(s => s.placement === id)) && samples.every(s => s.error === null);
      const age = p95(ages), eligible = p95(eligibleAges);
      const surfaces = [...byPlacement].map(([id, s]) => ({ id, surface: s.surface, p95: p95(s.drawn), eligibleP95: p95(s.eligible), frames: s.drawn.length }));
      const pass = Number.isFinite(age) && Number.isFinite(eligible) && eligible <= MAX_PLAYBACK_AGE_HOURS &&
        surfaces.every(s => s.eligibleP95 <= MAX_PLAYBACK_AGE_HOURS) &&
        (widget.kind !== 'nuclear-arsenal' || age <= MAX_PLAYBACK_AGE_HOURS) && future === 0 && exact;
      placements.push({ from: w.from, widget: widget.id, kind: widget.kind, scroll: w.scroll, p95: Number.isFinite(age) ? age : null,
        eligibleP95: Number.isFinite(eligible) ? eligible : null, surfaces, distinctHoursPerSecond: hours.size / seconds,
        samples: samples.length, missing, late, structureCrossings, future, exact, pass });
    }
  }
  const widgets = KINDS.map(kind => {
    const kept = placements.filter(p => p.kind === kind);
    const pass = kept.length > 0 && kept.every(p => p.pass);
    if (!pass) reasons.push(kind + ' missed its dated presentation or exact-read target');
    return { kind, p95: kept.length && kept.every(p => p.p95 !== null) ? Math.max(...kept.map(p => p.p95)) : null,
      eligibleP95: kept.length && kept.every(p => p.eligibleP95 !== null) ? Math.max(...kept.map(p => p.eligibleP95)) : null,
      structureCrossings: kept.reduce((sum, p) => sum + p.structureCrossings, 0),
      future: kept.reduce((sum, p) => sum + p.future, 0), exact: kept.length > 0 && kept.every(p => p.exact), pass };
  });
  return { pass: reasons.length === 0, reasons, metrics, widgets, placements };
}
