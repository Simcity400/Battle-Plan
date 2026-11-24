import { MAX_PLAYBACK_AGE_HOURS, freshness, playbackPace, quantiles, windowLosses } from './verdicts.mjs';

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
        if (!r.cancelled && anchor && r.seek === anchor.seek && r.playing === anchor.playing &&
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

const forSession = (records, session) => (session ? records.filter(r => !r.session || r.session === session) : records);
const hasSummaryLosses = losses => {
  if (!losses) return false;
  if (Array.isArray(losses)) return losses.length > 0;
  if (typeof losses === 'object') return Object.values(losses).some(entry => entry && (entry.count ?? 0) > 0);
  return false;
};

export function datedPlaybackVerdict(windows, records, { legacy = false } = {}) {
  const reasons = [], metrics = [], placements = [];
  const sorted = records.toSorted((a, b) => a.at - b.at);
  if (!windows.length) reasons.push('nothing measured');
  for (const w of windows) {
    const winRecords = forSession(sorted, w.session);
    const visibleWidgets = (w.widgets ?? []).filter(widget => widget.visible);
    if (!visibleWidgets.length) reasons.push('no visible dashboard placements');
    if (!visibleWidgets.length || visibleWidgets.every(widget => widget.notMigrated)) reasons.push('nothing measured');
    if (Array.isArray(w.playbackReasons)) for (const reason of w.playbackReasons) reasons.push(reason);
    const ticks = winRecords.filter(r => r.t === 'tick' && r.playing && r.speed === 1440 && r.at >= w.from && r.at <= w.to);
    const first = ticks[0], last = ticks.at(-1);
    const inside = at => at >= first?.at && at <= last?.at;
    const presented = (w.presented ?? []).filter(inside);
    const end = presented.findIndex(at => at >= presented[0] + w.seconds * 1000);
    const frames = end < 0 ? presented : presented.slice(0, end + 1);
    const playLost = w.from != null && w.to != null ? windowLosses(winRecords, w.from, w.to) : { count: 0, sources: [] };
    const inputFrom = w.inputs?.actions?.[0]?.at ?? w.to;
    const inputLost = w.inputs && inputFrom != null && w.inputs.to != null ? windowLosses(winRecords, inputFrom, w.inputs.to) : { count: 0, sources: [] };
    const idleLost = w.idleCost?.from != null && w.idleCost?.to != null ? windowLosses(winRecords, w.idleCost.from, w.idleCost.to) : { count: 0, sources: [] };
    if (playLost.count > 0) reasons.push(`perf log lost ${playLost.count} records during ${w.page ?? 'playback'} playing window (${playLost.sources.join(', ')})`);
    if (inputLost.count > 0) reasons.push(`perf log lost ${inputLost.count} records during ${w.page ?? 'playback'} input sequence (${inputLost.sources.join(', ')})`);
    if (idleLost.count > 0) reasons.push(`perf log lost ${idleLost.count} records during ${w.page ?? 'playback'} idle cost window (${idleLost.sources.join(', ')})`);
    const hasScopedLoss = playLost.count > 0 || inputLost.count > 0 || idleLost.count > 0;
    if (end < 0 || !w.seconds || (w.complete === false && !hasScopedLoss && !hasSummaryLosses(w.losses))) reasons.push('incomplete presented playing window');
    const timed = r => r.at >= frames[0] && r.at <= frames.at(-1);
    const seconds = (frames.at(-1) - frames[0]) / 1000;
    const page = winRecords.filter(r => r.t === 'frame' && r.playing && timed(r));
    const allWorker = winRecords.filter(r => r.t === 'workerFrame' && r.playing && timed(r));
    const worker = allWorker.filter(r => !r.cancelled);
    const laneCount = Math.max(0, ...allWorker.map(r => r.lanes ?? (r.lane ?? 0) + 1));
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
    const counted = winRecords.filter(r => r.t === 'commit' && (Number.isFinite(firstCommit) ? r.serial >= firstCommit && r.serial <= page.at(-1)?.commitSequence : timed(r)));
    const commits = winRecords.filter(r => r.t === 'commit' && timed(r));
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
    for (const r of winRecords) {
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
    const idle = w.idleCost
      ? winRecords.filter(r => r.t === 'workerFrame' && r.playing && r.changed === 0 && !r.cancelled && r.at >= w.idleCost.from && r.at <= w.idleCost.to)
      : worker.filter(r => r.changed === 0 && !r.cancelled);
    const emptyMs = mean(idle.map(r => r.workMs));
    const changing = worker.filter(r => r.changed > 0);
    const changedWidgetMs = emptyMs === null ? null : mean(changing.map(r => (r.workMs - emptyMs) / r.changed));
    const structureBySource = {};
    for (const r of structural) for (const source of new Set(r.changes.filter(verified).map(c => c.source))) structureBySource[source] = (structureBySource[source] ?? 0) + 1;
    metrics.push({ fps, pageFps, presents, hps: pace.hps, commitsPerSecond: commitCount / seconds, structureCommitsPerSecond: structural.length / seconds,
      inputCommitsPerSecond: input.length / seconds, unexplainedCommitsPerSecond: unexplained / seconds, structureBySource, frames: frames.length,
      compositorGapP95: p95(gaps(frames)), workerGapP95: p95(gaps(worker.map(r => r.at))), emptyMs, emptyFrames: idle.length,
      changedWidgetMs, changingFrames: changing.length, pressureMerges: w.pressureMerges, playLost: playLost.count, inputLost: inputLost.count, idleLost: idleLost.count });
    const hasRecordedInputs = Array.isArray(w.inputs?.actions) && w.inputs.actions.length > 0;
    if (w.inputs && (!legacy || hasRecordedInputs)) {
      const checks = [];
      const actions = Array.isArray(w.inputs.actions) ? w.inputs.actions : [];
      const presentedInputs = Array.isArray(w.inputs.presented) ? w.inputs.presented : [];
      let pinWidget;
      for (const [i, action] of actions.entries()) {
        const failed = typeof action.failed === 'string' && action.failed.trim() ? action.failed.trim() : null;
        if (failed) {
          checks.push({ kind: action.kind, pass: false, reason: failed });
          reasons.push(`Dashboard ${action.kind} failed: ${failed}`);
          continue;
        }
        const notApplicable = typeof action.notApplicable === 'string' && action.notApplicable.trim() ? action.notApplicable.trim() : null;
        if (notApplicable) { checks.push({ kind: action.kind, notApplicable, pass: true }); continue; }
        const times = presentedInputs.filter(at => at >= action.at && at < (actions[i + 1]?.at ?? w.inputs.to));
        pinWidget = action.pinWidget ?? pinWidget;
        checks.push({ kind: action.kind, ...inputPictures(times, w.inputs.visibility ?? [], winRecords, pinWidget) });
      }
      for (const kind of CANONICAL_INPUTS) {
        const ofKind = checks.filter(c => c.kind === kind);
        const checked = ofKind.some(c => !c.notApplicable && c.pass &&
          (kind !== 'pin' || actions.some(action => action.kind === kind && action.pinWidget)));
        const notApplicable = ofKind.length > 0 && ofKind.every(c => c.notApplicable);
        const explicitFailure = ofKind.some(c => c.reason && !c.pass);
        if (!checked && !notApplicable && !explicitFailure) reasons.push('Missing input coverage for ' + kind);
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
      if (widget.notMigrated) { placements.push({ from: w.from, page: w.page, widget: widget.id, title: widget.title, kind: widget.kind, scroll: w.scroll, status: 'not migrated', reason: 'not migrated', pass: false }); continue; }
      if (widget.outcome === 'refusal') {
        const detail = widget.detail ?? 'refused';
        placements.push({ from: w.from, page: w.page, widget: widget.id, title: widget.title, kind: widget.kind, scroll: w.scroll, status: 'refused', reason: detail, pass: false });
        reasons.push(`${widget.kind} refused: ${detail}`);
        continue;
      }
      if (widget.outcome === 'error') {
        const detail = widget.detail ?? 'error';
        placements.push({ from: w.from, page: w.page, widget: widget.id, title: widget.title, kind: widget.kind, scroll: w.scroll, status: 'error', reason: detail, pass: false });
        reasons.push(`${widget.kind} error: ${detail}`);
        continue;
      }
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
      const paintedInWindow = reports.filter(r => r.mode === 'picture' && !r.detached && Number.isFinite(r.hour) && timed(r));
      const unauditedPictures = countUnauditedPictures(paintedInWindow, samples);
      const exact = required.size > 0 && [...required].every(surface => observedSurfaces.has(surface))
        && [...required].every(surface => samples.some(s => s.surface === surface && rendered(s)))
        && [...observedPairs].every(pair => auditedPairs.has(pair)) && unauditedPictures === 0 && audited.every(s => s.error === null);
      const age = p95(ages), eligible = p95(eligibleAges);
      const surfaces = [...byPlacement].map(([id, s]) => ({ id, surface: s.surface, p95: p95(s.drawn), eligibleP95: p95(s.eligible), frames: s.drawn.length, draw: drawCost(reports, id) }));
      const pass = playLost.count === 0 && Number.isFinite(age) && age <= MAX_PLAYBACK_AGE_HOURS && Number.isFinite(eligible) && eligible <= MAX_PLAYBACK_AGE_HOURS &&
        surfaces.every(s => Number.isFinite(s.p95) && s.p95 <= MAX_PLAYBACK_AGE_HOURS && Number.isFinite(s.eligibleP95) && s.eligibleP95 <= MAX_PLAYBACK_AGE_HOURS) &&
        future === 0 && missing === 0 && exact && boundaryErrors.length === 0;
      placements.push({ from: w.from, page: w.page, widget: widget.id, title: widget.title, kind: widget.kind, scroll: w.scroll, p95: Number.isFinite(age) ? age : null,
        eligibleP95: Number.isFinite(eligible) ? eligible : null, surfaces, samples: samples.length, distinctHoursPerSecond: hours.size / seconds,
        missing, late, lateWithoutCrossing, structureCrossings, future, exact, ...(unauditedPictures > 0 && { unauditedPictures }), boundaryErrors,
        ...(playLost.count > 0 && { lostRecords: playLost.count, reason: `perf log lost ${playLost.count} records during playing window` }), pass });
      reasons.push(...boundaryErrors);
    }
    for (const expected of expectedWidgets) {
      if (!inventory.has(expected.id)) continue;
      if (!placements.slice(placementStart).some(placement => placement.widget === expected.id)) reasons.push('Missing judged placement for expected widget');
    }
  }
  const notMigratedPlacements = placements.filter(p => p.status === 'not migrated');
  const notMigratedKinds = [...new Set(notMigratedPlacements.map(p => p.kind))];
  if (notMigratedPlacements.length) {
    reasons.unshift(`${notMigratedPlacements.length} placement${notMigratedPlacements.length === 1 ? '' : 's'} not migrated (${notMigratedKinds.length} kind${notMigratedKinds.length === 1 ? '' : 's'}: ${notMigratedKinds.join(', ')})`);
  }
  const kinds = [...new Set(placements.map(p => p.kind))];
  const widgets = kinds.map(kind => {
    const kept = placements.filter(p => p.kind === kind), migrated = kept.filter(p => p.status !== 'not migrated');
    const notMigratedCount = kept.length - migrated.length;
    const realFailed = migrated.some(p => !p.pass && p.status !== 'refused' && p.status !== 'error');
    const pass = notMigratedCount === 0 && migrated.length > 0 && migrated.every(p => p.pass);
    if (realFailed) reasons.push('A widget missed its dated presentation or exact-read target');
    return { kind, p95: migrated.length && migrated.every(p => p.p95 !== null && p.p95 !== undefined) ? Math.max(...migrated.map(p => p.p95)) : null,
      eligibleP95: migrated.length && migrated.every(p => p.eligibleP95 !== null && p.eligibleP95 !== undefined) ? Math.max(...migrated.map(p => p.eligibleP95)) : null,
      structureCrossings: migrated.reduce((sum, p) => sum + (p.structureCrossings ?? 0), 0), future: migrated.reduce((sum, p) => sum + (p.future ?? 0), 0),
      exact: migrated.length > 0 && migrated.every(p => p.exact === true),
      ...(notMigratedCount > 0 && { notMigrated: notMigratedCount }), pass };
  });
  return { pass: reasons.length === 0, reasons, notMigrated: notMigratedPlacements.length, notMigratedKinds, metrics, widgets, placements };
}

export const COST_FORMULA = 'mainDelta.p95 + worker.p95 + chartWorker.p95';
export const BASELINE_MIN_FRAMES = 30;
export const WIDGET_LOW_FRAMES = 30;
export const BASELINE_MAX_P50_SPREAD_MS = 3;
export const BASELINE_MAX_P95_SPREAD_MS = 8;
export const BASELINE_MAX_TAIL_GAP_MS = 15;

export function rankWidgetCosts(entries) {
  const tie = entry => (Number.isFinite(entry.mainDelta?.p50) ? entry.mainDelta.p50 : -Infinity) + (entry.worker?.mean ?? 0);
  return [...entries].sort((a, b) => {
    const aMeasured = Number.isFinite(a.costScoreMs), bMeasured = Number.isFinite(b.costScoreMs);
    if (aMeasured !== bMeasured) return aMeasured ? -1 : 1;
    if (aMeasured && bMeasured && b.costScoreMs !== a.costScoreMs) return b.costScoreMs - a.costScoreMs;
    return tie(b) - tie(a)
      || (b.requestsPerSec ?? 0) - (a.requestsPerSec ?? 0)
      || (b.commitsPerSec ?? 0) - (a.commitsPerSec ?? 0)
      || String(a.kind).localeCompare(String(b.kind));
  });
}

function checkCostClock(w, winRecords) {
  if (!w || w.from == null || w.to == null || !(w.to > w.from)) {
    return { ok: false, hps: null, elapsedSec: 0, reason: 'missing cost window timestamps' };
  }
  if (w.playingAtSnap === false) {
    return { ok: false, hps: null, elapsedSec: 0, reason: 'clock stopped playing before end of window' };
  }
  const allTicks = winRecords.filter(r => r.t === 'tick' && r.at >= w.from && r.at <= w.to);
  const pausedEarly = allTicks.find(r => !r.playing && r.at < w.to - 50);
  if (pausedEarly) {
    return { ok: false, hps: null, elapsedSec: 0, reason: 'clock paused inside cost window' };
  }
  const ticks = allTicks.filter(r => r.playing && r.speed === 1440);
  if (ticks.length < 2) {
    return { ok: false, hps: null, elapsedSec: 0, reason: 'clock recorded fewer than 2 ticks at 1,440 h/s' };
  }
  const first = ticks[0], last = ticks.at(-1);
  const elapsedSec = (last.at - first.at) / 1000;
  const recordedSec = (w.to - w.from) / 1000;
  const requestedSec = Number.isFinite(w.seconds) ? w.seconds : recordedSec;
  const requiredSec = Math.max(requestedSec, recordedSec);
  const pace = playbackPace(first, last, 1440);
  if (recordedSec < requestedSec * 0.75) {
    return { ok: false, hps: pace.hps, elapsedSec, reason: `recorded window was only ${recordedSec.toFixed(2)} s of ${requestedSec.toFixed(1)} s requested` };
  }
  if (elapsedSec < Math.max(0.5, requiredSec * 0.75)) {
    return { ok: false, hps: pace.hps, elapsedSec, reason: `clock ticked over only ${elapsedSec.toFixed(2)} s of ${requiredSec.toFixed(1)} s window` };
  }
  if (!pace.pass) {
    return { ok: false, hps: pace.hps, elapsedSec, reason: `measured clock speed ${pace.hps?.toFixed(1) ?? 'unknown'} h/s, expected 1,440 h/s` };
  }
  return { ok: true, hps: pace.hps, elapsedSec, reason: null };
}

function countUnauditedPictures(paintedInWindow, auditedSamples) {
  const auditedKeys = new Set(auditedSamples.filter(rendered).map(s => `${s.generation ?? 0}\u0000${s.placement}\u0000${s.owner}\u0000${s.at}`));
  const paintedChecks = new Map();
  for (const r of paintedInWindow) {
    const gen = r.generation ?? 0;
    if (r.surface in RENDERER_OWNER) {
      const wantOwner = RENDERER_OWNER[r.surface];
      const groupKey = `${gen}\u0000${r.placement}\u0000${wantOwner}\u0000${r.at}`;
      const prev = paintedChecks.get(groupKey);
      const missingHere = r.owner === wantOwner && r.auditMissing === true;
      paintedChecks.set(groupKey, { auditMissing: (prev?.auditMissing ?? false) || missingHere });
    } else {
      const key = `${gen}\u0000${r.placement}\u0000${r.owner}\u0000${r.at}`;
      const prev = paintedChecks.get(key);
      paintedChecks.set(key, { auditMissing: (prev?.auditMissing ?? false) || r.auditMissing === true });
    }
  }
  let unauditedPictures = 0;
  for (const [key, state] of paintedChecks) {
    if (state.auditMissing || !auditedKeys.has(key)) unauditedPictures++;
  }
  return unauditedPictures;
}

export function summarizeWidgetCosts(windows, records, { baseline = null, baselines = null, profile = null, confinement = null, ungated = false } = {}) {
  const sorted = (records ?? []).toSorted((a, b) => a.at - b.at);
  const diff = (a, b) => Number.isFinite(a) && Number.isFinite(b) ? a - b : null;
  const frameStatsOf = (w, winRecords) => {
    if (!w || w.from == null || w.to == null) return { main: { n: 0, p50: null, p95: null, max: null }, interval: { n: 0, p50: null, p95: null, max: null }, frames: [], lost: 0 };
    const frames = winRecords.filter(r => r.t === 'frame' && r.playing && r.at >= w.from && r.at <= w.to);
    return { main: quantiles(frames.map(r => r.main)), interval: quantiles(frames.map(r => r.dt)), frames, lost: windowLosses(winRecords, w.from, w.to).count };
  };
  const baselineWindows = Array.isArray(baselines) ? baselines.filter(Boolean) : (baseline ? [baseline] : []);
  const baselineReasons = [];
  if (!baselineWindows.length) baselineReasons.push('no empty-page baseline recorded');
  const pooledFrames = [], pooledWorker = [], baselineSamples = [];
  let baselineSeconds = 0, baselineLost = 0;
  for (const [index, bw] of baselineWindows.entries()) {
    const sampleLabel = bw.sampleLabel ?? (baselineWindows.length === 1 ? 'baseline' : `sample-${index + 1}`);
    const winRecords = forSession(sorted, bw.session);
    const stats = frameStatsOf(bw, winRecords);
    const worker = bw.from != null && bw.to != null
      ? winRecords.filter(r => r.t === 'workerFrame' && r.playing && !r.cancelled && r.at >= bw.from && r.at <= bw.to) : [];
    const clock = checkCostClock(bw, winRecords);
    const seconds = bw.from != null && bw.to != null && bw.to > bw.from ? (bw.to - bw.from) / 1000 : (bw.seconds ?? 0);
    const sampleReasons = [];
    if (bw.complete === false) sampleReasons.push('window incomplete');
    if (Array.isArray(bw.reasons) && bw.reasons.length) sampleReasons.push(...bw.reasons);
    if (stats.lost > 0) sampleReasons.push(`perf log lost ${stats.lost} records`);
    if (stats.main.n < BASELINE_MIN_FRAMES) sampleReasons.push(`only ${stats.main.n} frames (minimum ${BASELINE_MIN_FRAMES})`);
    if (!clock.ok) sampleReasons.push(clock.reason);
    if (Number.isFinite(stats.main.p95) && Number.isFinite(stats.main.p50) && stats.main.p95 - stats.main.p50 > BASELINE_MAX_TAIL_GAP_MS) {
      sampleReasons.push(`unstable tail: p95 ${stats.main.p95.toFixed(1)} ms vs p50 ${stats.main.p50.toFixed(1)} ms (gap ${(stats.main.p95 - stats.main.p50).toFixed(1)} ms > ${BASELINE_MAX_TAIL_GAP_MS} ms)`);
    }
    for (const reason of sampleReasons) baselineReasons.push(`baseline (${sampleLabel}): ${reason}`);
    pooledFrames.push(...stats.frames);
    pooledWorker.push(...worker);
    baselineSeconds += seconds;
    baselineLost += stats.lost;
    baselineSamples.push({
      label: sampleLabel,
      seconds,
      main: stats.main,
      interval: stats.interval,
      workerMeanMs: mean(worker.map(r => r.workMs)),
      lostRecords: stats.lost,
      clockHps: clock.hps,
      usable: sampleReasons.length === 0,
      reasons: sampleReasons,
    });
  }
  const hasStart = baselineSamples.some(sample => sample.label === 'start');
  const hasEnd = baselineSamples.some(sample => sample.label === 'end');
  if (!hasStart) baselineReasons.push('missing start empty-page baseline sample');
  if (!hasEnd) baselineReasons.push('missing end empty-page baseline sample');
  const pooledMain = quantiles(pooledFrames.map(r => r.main));
  const pooledInterval = quantiles(pooledFrames.map(r => r.dt));
  const p50Values = baselineSamples.map(s => s.main.p50).filter(Number.isFinite);
  const p95Values = baselineSamples.map(s => s.main.p95).filter(Number.isFinite);
  const p50SpreadMs = p50Values.length >= 2 ? Math.max(...p50Values) - Math.min(...p50Values) : 0;
  const p95SpreadMs = p95Values.length >= 2 ? Math.max(...p95Values) - Math.min(...p95Values) : 0;
  if (p50SpreadMs > BASELINE_MAX_P50_SPREAD_MS) {
    baselineReasons.push(`baseline p50 spread ${p50SpreadMs.toFixed(1)} ms across samples exceeds ${BASELINE_MAX_P50_SPREAD_MS} ms`);
  }
  if (p95SpreadMs > BASELINE_MAX_P95_SPREAD_MS) {
    baselineReasons.push(`baseline p95 spread ${p95SpreadMs.toFixed(1)} ms across samples exceeds ${BASELINE_MAX_P95_SPREAD_MS} ms`);
  }
  if (Number.isFinite(pooledMain.p95) && Number.isFinite(pooledMain.p50) && pooledMain.p95 - pooledMain.p50 > BASELINE_MAX_TAIL_GAP_MS) {
    baselineReasons.push(`pooled baseline tail gap ${(pooledMain.p95 - pooledMain.p50).toFixed(1)} ms exceeds ${BASELINE_MAX_TAIL_GAP_MS} ms`);
  }
  const baselineUsable = baselineReasons.length === 0 && pooledMain.n > 0 && Number.isFinite(pooledMain.p95);
  const baselineSummary = {
    seconds: baselineWindows.length ? baselineSeconds : null,
    main: pooledMain,
    interval: pooledInterval,
    workerMeanMs: mean(pooledWorker.map(r => r.workMs)),
    lostRecords: baselineLost,
    spread: { p50Ms: p50SpreadMs, p95Ms: p95SpreadMs },
    samples: baselineSamples,
    usable: baselineUsable,
    complete: baselineUsable,
    reasons: [...new Set(baselineReasons)],
  };
  const widgets = windows.map(w => {
    const winRecords = forSession(sorted, w.session);
    const seconds = w.from != null && w.to != null && w.to > w.from ? (w.to - w.from) / 1000 : (w.seconds ?? 0);
    const { main, interval, frames, lost } = frameStatsOf(w, winRecords);
    const clock = checkCostClock(w, winRecords);
    const mainDelta = baselineUsable ? {
      p50: diff(main.p50, pooledMain.p50),
      p95: diff(main.p95, pooledMain.p95),
      max: diff(main.max, pooledMain.max),
    } : { p50: null, p95: null, max: null };
    const workerFrames = w.from != null && w.to != null
      ? winRecords.filter(r => r.t === 'workerFrame' && r.playing && !r.cancelled && r.at >= w.from && r.at <= w.to) : [];
    const changed = workerFrames.filter(r => r.changed > 0);
    const workerQ = quantiles(changed.map(r => r.workMs));
    const workerMean = mean(changed.map(r => r.workMs));
    const workerTotalMs = changed.reduce((sum, r) => sum + (Number.isFinite(r.workMs) ? r.workMs : 0), 0);
    const chartWorkerFrames = w.from != null && w.to != null
      ? winRecords.filter(r => r.t === 'chartWorker' && r.at >= w.from && r.at <= w.to && (!w.widget || !r.widget || r.widget === w.widget)) : [];
    const chartQ = quantiles(chartWorkerFrames.map(r => r.ms));
    const chartTotalMs = chartWorkerFrames.reduce((sum, r) => sum + (Number.isFinite(r.ms) ? r.ms : 0), 0);
    const chartReplaced = chartWorkerFrames.reduce((sum, r) => sum + (Number.isFinite(r.replaced) ? r.replaced : 0), 0);
    const pictures = w.from != null && w.to != null && w.widget
      ? winRecords.filter(r => r.t === 'picture' && r.widget === w.widget && r.at >= (w.seekStart ?? w.from) && r.at <= w.to) : [];
    const playingPictures = pictures.filter(r => r.at >= w.from && r.at <= w.to && r.renderer === 'page');
    const drawMsValues = playingPictures.filter(r => !r.detached && Number.isFinite(r.drawMs)).map(r => r.drawMs);
    const worker = {
      frames: workerFrames.length,
      changedFrames: changed.length,
      p50: workerQ.p50,
      p95: workerQ.p95,
      max: workerQ.max,
      mean: workerMean,
      totalMsPerSec: seconds > 0 ? workerTotalMs / seconds : null,
      drawMsP95: p95(drawMsValues),
    };
    const chartWorker = chartWorkerFrames.length ? {
      paints: chartWorkerFrames.length,
      replaced: chartReplaced,
      p50: chartQ.p50,
      p95: chartQ.p95,
      max: chartQ.max,
      mean: mean(chartWorkerFrames.map(r => r.ms)),
      totalMsPerSec: seconds > 0 ? chartTotalMs / seconds : null,
    } : null;
    const commits = frames.reduce((sum, r) => sum + (Number.isFinite(r.commits) ? r.commits : 0), 0);
    const commitsPerSec = seconds > 0 ? commits / seconds : null;
    const queries = w.from != null && w.to != null
      ? winRecords.filter(r => r.t === 'q' && r.at >= w.from && r.at <= w.to) : [];
    const liveQueries = queries.filter(r => !r.cache && r.cmd !== undefined);
    const cacheHits = queries.filter(r => r.cache);
    const requestCountsByProduct = {};
    for (const q of liveQueries) {
      const name = q.product ?? q.cmd ?? 'unknown';
      requestCountsByProduct[name] = (requestCountsByProduct[name] ?? 0) + 1;
    }
    const requestsByProduct = Object.fromEntries(Object.entries(requestCountsByProduct)
      .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
      .map(([product, count]) => [product, seconds > 0 ? count / seconds : count]));
    const requestsPerSec = seconds > 0 ? liveQueries.length / seconds : null;
    const cacheHitsPerSec = seconds > 0 ? cacheHits.length / seconds : null;
    let lagP95Hours = null, futureFrames = 0;
    if (playingPictures.length) {
      const lags = [];
      for (const r of playingPictures) {
        const target = Math.floor(r.fixedHour ?? r.head);
        if (Number.isFinite(r.hour) && Number.isFinite(target)) {
          if (r.hour > target) futureFrames++;
          lags.push(target - r.hour);
        }
      }
      lagP95Hours = p95(lags);
    } else if (w.from != null && w.to != null && w.widget) {
      const fresh = freshness(winRecords, w.from, w.to, w.seekStart ?? w.from).widgets.find(entry => entry.widget === w.widget);
      if (fresh) {
        lagP95Hours = fresh.p95 >= Number.MAX_VALUE ? null : fresh.p95;
        futureFrames = fresh.ahead ?? 0;
      }
    }
    const audited = Array.isArray(w.audit) ? w.audit : [];
    const inWindowAudit = w.from != null && w.to != null ? audited.filter(s => Number.isFinite(s.at) && s.at >= w.from && s.at <= w.to) : audited;
    const postWindowAudit = w.from != null && w.to != null ? audited.filter(s => Number.isFinite(s.at) && s.at > w.to) : [];
    const observedPairs = new Set(playingPictures.filter(r => !r.detached).map(r => `${r.surface}\u0000${r.placement}`));
    const auditedPairs = new Set(inWindowAudit.filter(rendered).map(s => `${s.surface}\u0000${s.placement}`));
    const paintedInWindow = playingPictures.filter(r => r.mode === 'picture' && !r.detached && Number.isFinite(r.hour));
    const unauditedPictures = countUnauditedPictures(paintedInWindow, inWindowAudit);
    const inWindowAuditError = inWindowAudit.find(s => s.error !== null)?.error ?? null;
    const postWindowAuditErrors = postWindowAudit.filter(s => s.error !== null).map(s => ({ at: s.at, surface: s.surface, placement: s.placement, error: s.error }));
    const widgetErrors = w.from != null && w.to != null && w.widget
      ? winRecords.filter(r => r.t === 'widget_error' && r.widget === w.widget && r.at >= (w.seekStart ?? w.from) && r.at <= w.to) : [];
    let outcome = w.outcome ?? 'content';
    let detail = w.detail ?? null;
    if (widgetErrors.length && outcome !== 'error') {
      outcome = 'error';
      detail = widgetErrors[0].message ?? 'widget error';
    }
    let exact = outcome === 'content' ? 'not-audited' : 'n/a', exactError = null;
    if (outcome === 'content' && (inWindowAudit.length > 0 || observedPairs.size > 0)) {
      if (inWindowAuditError) { exact = 'fail'; exactError = inWindowAuditError; }
      else if (unauditedPictures > 0) { exact = 'fail'; exactError = `${unauditedPictures} painted picture${unauditedPictures === 1 ? '' : 's'} emitted without an audit sample`; }
      else if (inWindowAudit.length > 0 && [...observedPairs].every(pair => auditedPairs.has(pair))) { exact = 'pass'; }
      else { exact = 'fail'; exactError = 'missing exact-read audit for an observed surface'; }
    } else if (inWindowAuditError) {
      exact = 'fail'; exactError = inWindowAuditError;
    }
    const incompleteReasons = [];
    if (Array.isArray(w.reasons)) incompleteReasons.push(...w.reasons);
    if (lost > 0) incompleteReasons.push(`perf log lost ${lost} records`);
    if (main.n === 0) incompleteReasons.push('no timed playing frames');
    if (!clock.ok) incompleteReasons.push(clock.reason);
    if (!baselineUsable) incompleteReasons.push('empty-page baseline is unusable');
    const correctnessErrors = [];
    if (exact === 'fail') correctnessErrors.push(exactError ? `audit: ${exactError}` : 'exact-read audit failed');
    if (futureFrames > 0) correctnessErrors.push(`${futureFrames} future frame${futureFrames === 1 ? '' : 's'}`);
    const complete = Boolean(w.complete ?? true) && lost === 0 && main.n > 0 && clock.ok && baselineUsable;
    const pass = outcome !== 'error' && outcome !== 'placeholder' && complete && correctnessErrors.length === 0;
    const belowBaselineNoise = Number.isFinite(mainDelta.p95) && mainDelta.p95 < 0;
    const lowFrameCount = main.n > 0 && main.n < WIDGET_LOW_FRAMES;
    const costScoreMs = complete && Number.isFinite(mainDelta.p95) ? mainDelta.p95 + (worker.p95 ?? 0) + (chartWorker?.p95 ?? 0) : null;
    return {
      kind: w.kind, title: w.title ?? w.kind, outcome, detail,
      seconds, clockHps: clock.hps, main, mainDelta, interval, worker, chartWorker,
      commits, commitsPerSec,
      requests: liveQueries.length, requestsPerSec, requestsByProduct, requestCountsByProduct, cacheHitsPerSec,
      lagP95Hours, futureFrames,
      exact, exactError, auditSamples: inWindowAudit.length, postWindowAuditErrors,
      lostRecords: lost, complete, pass, correctnessErrors, incompleteReasons: [...new Set(incompleteReasons)],
      belowBaselineNoise, lowFrameCount,
      costScoreMs,
    };
  });
  return {
    formula: COST_FORMULA,
    profile,
    confinement,
    ungated,
    baseline: baselineSummary,
    widgets: rankWidgetCosts(widgets),
  };
}
