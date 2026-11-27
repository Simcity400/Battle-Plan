
export const BUDGET_MS = 100;
export const MAX_PLAYBACK_AGE_HOURS = 48;
export function playbackPace(first, last, speed) {
  const elapsed = (last?.composedAt ?? last?.at) - (first?.composedAt ?? first?.at);
  const hps = elapsed > 0 ? 1000 * (last.hour - first.hour) / elapsed : null;
  return { hps, pass: hps !== null && Math.abs(hps / speed - 1) <= .02 };
}
export const FRAME_BUDGET_MS = 1000 / 95;
export const MAP_WAIT_MS = 10_000;

export const pageShown = page => `(document.querySelector('.sidebar-page-list button[aria-current=page]')?.textContent.trim() === ${JSON.stringify(page)}
  && (document.querySelector('.dash-grid-scroll')?.getClientRects().length ?? 0) > 0)`;

export function settingDrawn({ settings, drawn, state }, before) {
  return drawn !== null && drawn !== undefined && drawn === settings && settings !== before && state !== 'loading';
}

export const DRAWN_GRACE_MS = 500;

export function stepOver({ settled, busy, idle, heldMs, wait }) {
  if (!settled) return false;
  return busy === null || busy === undefined ? heldMs >= wait : busy === 0 && idle >= 2;
}

export const stillInFlight = (kinds, timeout) => `still in flight after ${Math.round(timeout / 1000)} s: ${Object.entries(kinds).filter(([, n]) => n > 0).map(([kind, n]) => `${kind} ${n}`).join(', ')}`;

export const noVisibleChange = (settingsBefore, settingsNow) => settingsNow !== null && settingsNow === settingsBefore
  ? 'no visible change after the setting: the widget was never given it'
  : 'no visible change after the setting: the widget took it, and nothing it shows changed';

export const phaseSeconds = sessions => (sessions ?? []).flatMap(meta => (meta.phases ?? [])
  .filter(phase => Number.isFinite(meta[`${phase}Seconds`])).map(phase => [phase, meta[`${phase}Seconds`]]));

export function machineLine(run) {
  const machine = run?.machine;
  const count = value => Number.isFinite(value) ? value : 'not recorded';
  const sample = (when, at) => !at ? `${when} not recorded`
    : `${when} CPU ${Number.isFinite(at.cpuPercent) ? `${at.cpuPercent}%` : 'not recorded'}, memory ${count(at.memoryPercent)}${Number.isFinite(at.memoryPercent) ? '%' : ''}, `
      + `apps ${count(at.apps?.thisRun)} of this run, ${count(at.apps?.otherRuns)} of other suite runs, ${count(at.apps?.machine)} on the machine`;
  if (!machine) return 'Machine: not recorded.';
  return `Machine (${count(machine.logicalCpus)} logical CPUs): ${sample('before the first app', machine.start)}; ${sample('before the last app closed', machine.end)}.`;
}

export function classifyWidgetSnapshot({
  domState = null, domDetail = null,
  unavailableText = null, errorDetail = null,
  outsideText = null, notRecordedText = null, emptyText = null,
  probeState = null, probeDrawing = false,
} = {}) {
  const clean = value => (typeof value === 'string' && value.trim() ? value.trim() : null);
  const state = clean(domState);
  const detail = clean(domDetail);
  if (state === 'unavailable') return { outcome: 'refusal', detail: clean(unavailableText) ?? detail ?? 'unavailable' };
  if (state === 'error') return { outcome: 'error', detail: clean(errorDetail) ?? detail ?? 'error' };
  if (state === 'outside') return { outcome: 'outside', detail: clean(outsideText) ?? detail ?? 'outside' };
  if (state === 'not-recorded') return { outcome: 'not-recorded', detail: clean(notRecordedText) ?? detail ?? 'not-recorded' };
  if (state === 'empty') return { outcome: 'content', detail: clean(emptyText) ?? detail };
  if (state === 'loading') return probeDrawing ? { outcome: 'content', detail: null } : { outcome: 'error', detail: detail ?? 'still loading' };
  const unavail = clean(unavailableText);
  if (unavail) return { outcome: 'refusal', detail: unavail };
  const err = clean(errorDetail);
  if (err) return { outcome: 'error', detail: err };
  const out = clean(outsideText);
  if (out) return { outcome: 'outside', detail: out };
  const unrec = clean(notRecordedText);
  if (unrec) return { outcome: 'not-recorded', detail: unrec };
  if (probeState === 'loading' && !probeDrawing) {
    return { outcome: 'error', detail: 'still loading' };
  }
  if (probeState === 'blank') {
    return { outcome: 'error', detail: 'blank widget body' };
  }
  const empty = clean(emptyText);
  if (empty) return { outcome: 'content', detail: empty };
  return { outcome: 'content', detail: null };
}

export function classifyPerfFile(name) {
  const viewer = /^perf-(.+)\.jsonl\.map\.\d+\.jsonl$/.exec(name);
  if (viewer) return { kind: 'viewer', session: viewer[1] };
  const page = /^perf-(.+)\.jsonl$/.exec(name);
  if (page) return { kind: 'page', session: page[1] };
  return null;
}

export function costSessionConfinement(run, sweep, costSessionSlug) {
  if (!costSessionSlug) return { job: null, page: null };
  const runSessions = Array.isArray(run?.sessions) ? run.sessions : null;
  const job = runSessions
    ? (runSessions.find(entry => entry.name.replace(/ /g, '-') === costSessionSlug)?.confinement ?? null)
    : null;
  const sweepSessions = Array.isArray(sweep?.meta?.sessions) ? sweep.meta.sessions : null;
  const page = sweepSessions
    ? (sweepSessions.find(meta => meta.session === costSessionSlug)?.confinement ?? null)
    : (sweep?.meta?.session === costSessionSlug ? (sweep.meta.confinement ?? null) : null);
  return { job, page };
}

export function sessionConfinement(run, sessionEntry, sweepMeta = null) {
  const cores = run?.cores ?? null;
  const job = sessionEntry?.confinement ?? null;
  const page = sweepMeta?.confinement ?? null;
  const reasons = [];
  if (cores) {
    const expectedLanes = Math.max(1, Math.min(4, cores.count - 2));
    if (!job) reasons.push('Job object confinement was not recorded');
    else if (!job.allConfined || job.confinedProcesses < job.jobProcesses) {
      reasons.push(`Job object confined ${job.confinedProcesses}/${job.jobProcesses} processes (expected all at ${cores.mask})`);
    }
    if (!page) {
      reasons.push('page confinement observation was not recorded');
    } else {
      if (page.hardwareConcurrency !== cores.count) {
        reasons.push(`page saw navigator.hardwareConcurrency = ${page.hardwareConcurrency}, expected ${cores.count}`);
      }
      const phases = sessionEntry?.phases ?? sweepMeta?.phases ?? [];
      const expectsLanes = phases.some(phase => phase === 'playback' || phase === 'widget-costs');
      if (page.lanes != null && page.lanes !== expectedLanes) {
        reasons.push(`created ${page.lanes} drawing lanes, expected ${expectedLanes} for ${cores.count} cores`);
      } else if (page.lanes == null && expectsLanes) {
        reasons.push('drawing lane count was not recorded');
      }
    }
  }
  const parts = [];
  if (job) parts.push(`Job ${job.confinedProcesses}/${job.jobProcesses}${job.allConfined ? '' : ' (NOT ALL CONFINED)'}`);
  if (page?.hardwareConcurrency != null) parts.push(`hardwareConcurrency=${page.hardwareConcurrency}`);
  if (page?.lanes != null) parts.push(`${page.lanes} lane${page.lanes === 1 ? '' : 's'}`);
  return { pass: reasons.length === 0, reasons, job, page, summary: parts.join(', ') || '–' };
}

export function profileLines(run, sweep, records = null) {
  const throttle = run?.cpuThrottle ?? sweep?.meta?.cpuThrottle ?? 1;
  const cores = run?.cores ?? null;
  const profile = run?.profile ?? null;
  const sweepSessions = sweep?.meta?.sessions ?? (sweep?.meta ? [sweep.meta] : []);
  const bySlug = new Map(sweepSessions.map(meta => [meta.session, meta]));
  const paired = (run?.sessions ?? []).map(entry => {
    const slug = entry.name.replace(/ /g, '-');
    return { name: entry.name, slug, ...sessionConfinement(run, entry, bySlug.get(slug) ?? null) };
  });
  const fallbackPage = sweepSessions.map(meta => meta.confinement).find(Boolean) ?? sweep?.meta?.confinement ?? null;
  const lines = [];
  if (profile || cores || throttle > 1 || paired.some(p => p.job || p.page) || fallbackPage) {
    const parts = [];
    if (profile) parts.push(`profile ${profile}`);
    parts.push(`CPU throttle ×${throttle}`);
    if (cores) parts.push(`core affinity ${cores.count} cores (${cores.mask}: CPUs ${cores.cpus.join(', ')})`);
    else parts.push('core affinity unconstrained');
    const measured = [];
    if (paired.length === 1) {
      const only = paired[0];
      if (only.job) measured.push(`Job object confined ${only.job.confinedProcesses}/${only.job.jobProcesses} processes${only.job.allConfined ? '' : ' (NOT ALL CONFINED)'}`);
      if (only.page?.hardwareConcurrency != null) measured.push(`page saw navigator.hardwareConcurrency = ${only.page.hardwareConcurrency}`);
      if (only.page?.lanes != null) measured.push(`${only.page.lanes} drawing lane${only.page.lanes === 1 ? '' : 's'} created`);
    } else if (paired.length > 1) {
      for (const item of paired) if (item.job || item.page) measured.push(`${item.name}: ${item.summary}`);
    } else {
      const jobConf = run?.confinement ?? null;
      if (jobConf) measured.push(`Job object confined ${jobConf.confinedProcesses}/${jobConf.jobProcesses} processes${jobConf.allConfined ? '' : ' (NOT ALL CONFINED)'}`);
      if (fallbackPage?.hardwareConcurrency != null) measured.push(`page saw navigator.hardwareConcurrency = ${fallbackPage.hardwareConcurrency}`);
      if (fallbackPage?.lanes != null) measured.push(`${fallbackPage.lanes} drawing lane${fallbackPage.lanes === 1 ? '' : 's'} created`);
    }
    lines.push(`Execution profile: ${parts.join(', ')}.${measured.length ? ` Measured confinement: ${measured.join('; ')}.` : ''}`);
    if (profile || cores || throttle > 1) {
      lines.push('Profile caveat: this profile limits OS core affinity, slows JS main-thread execution via Chromium CPU throttling, and stretches drawing-lane worker work via stretch() in ui/src/draw/weakCpu.ts. It does not imitate a slower graphics card (GPU raster, compositing, or native viewer shaders), slower memory bandwidth or caches, or slower native Rust and IPC decoding.');
    }
  }
  return lines;
}

export const asked = (run, phase) => !Array.isArray(run?.requested) || run.requested.includes(phase);

const canonical = value => Array.isArray(value) ? value.map(canonical)
  : value && typeof value === 'object' ? Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])])) : value;

export function mapDrew(state, expected) {
  if (!state?.applied) return 'its map reports no settings it drew with';
  for (const [setting, json] of expected) {
    const [group, key] = setting.split('.');
    const reported = key === undefined ? state.applied[group] : state.applied[group]?.[key];
    let got = reported ?? null, want = JSON.parse(json);
    if (group === 'visualMods') { got = [...got ?? []].sort(); want = [...want].sort(); }
    if (JSON.stringify(canonical(got)) !== JSON.stringify(canonical(want))) return `it drew ${setting} as ${JSON.stringify(reported ?? null)}, the menu saved ${json}`;
  }
  return null;
}

export function quantiles(values) {
  const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
  if (!sorted.length) return { n: 0, p50: null, p95: null, max: null };
  const rank = p => sorted[Math.min(sorted.length, Math.max(1, Math.ceil(p * sorted.length))) - 1];
  return { n: sorted.length, p50: rank(0.5), p95: rank(0.95), max: sorted.at(-1) };
}

export function item(name, samples, { budget = BUDGET_MS, reasons = [] } = {}) {
  const q = quantiles(samples.map(sample => sample.ms));
  const failed = samples.filter(sample => !sample.pass);
  const why = [...reasons, ...failed.flatMap(sample => sample.reasons.map(reason => sample.label ? `${sample.label}: ${reason}` : reason))];
  if (!samples.length && !reasons.length) why.push('no samples: the case never ran');
  if (q.max !== null && q.max > budget && !why.some(reason => reason.includes('over'))) why.push(`max ${Math.round(q.max)} ms, over ${budget} ms`);
  return { item: name, ...q, budget, pass: why.length === 0, reasons: [...new Set(why)] };
}

export function groupBy(values, key) {
  const groups = new Map();
  for (const value of values) { const k = key(value); if (!groups.has(k)) groups.set(k, []); groups.get(k).push(value); }
  return groups;
}

export function firstContent(results) {
  const placements = [];
  for (const step of results.filter(result => result.area === 'pages' && result.widgets)) {
    const [visit, page] = step.item.split('/');
    for (const widget of step.widgets) {
      const ms = widget.ready ?? widget.first;
      const reasons = [...new Set(widget.errors ?? [])];
      if (widget.end === 'loading') reasons.push('still loading');
      if (widget.end === 'unavailable') reasons.push(widget.notice?.text ?? 'unavailable');
      if (widget.end === 'blank') reasons.push('shows nothing');
      if (step.reasons?.includes('no trusted input reached the page')) reasons.push('no trusted input reached the page');
      if (visit === 'cold' && ms !== null && ms > BUDGET_MS) reasons.push(`${Math.round(ms)} ms, over ${BUDGET_MS} ms`);
      placements.push({ visit, page, kind: widget.kind, title: widget.title, step: step.label, id: `${page}#${widget.id}`, ms, end: widget.end,
        reasons, label: `${page}: ${widget.title} (${step.label})` });
    }
  }
  const seen = new Map();
  for (const placement of placements) {
    const key = `${placement.visit}|${placement.id}`;
    const before = seen.get(key);
    if (!before) { seen.set(key, placement); continue; }
    before.reasons = [...new Set([...before.reasons, ...placement.reasons.filter(reason => !reason.includes(' ms, over'))])];
    if (before.ms === null && placement.ms !== null) Object.assign(before, { ms: placement.ms, step: placement.step });
  }
  return [...seen.values()].map(placement => ({ ...placement, pass: placement.reasons.length === 0 }));
}

export function mapAcks(records) {
  const views = groupBy(records.filter(record => record.t === 'map_state'), record => record.view);
  for (const acks of views.values()) acks.sort((a, b) => a.at - b.at);
  return views;
}

export function presentedAt(mapRecords, drawnAt, session) {
  mapRecords = mapRecords.filter(record => record.session === session);
  if (!Number.isFinite(drawnAt)) return null;
  const main = new Set(mapRecords.filter(record => record.t === 'map_open' && record.role === 'main').map(record => record.map));
  for (const map of main) {
    const own = mapRecords.filter(record => record.t === 'map' && record.map === map).sort((a, b) => a.at - b.at);
    const index = own.findIndex(record => record.at - record.work_ms <= drawnAt && drawnAt <= record.at);
    const next = index < 0 ? null : own[index + 1];
    if (next && Number.isFinite(next.painted_at)) return next.painted_at;
  }
  return null;
}

export function presentedMapFrames(mapRecords, { session, from, to }, role) {
  const own = mapRecords.filter(record => record.session === session);
  const maps = new Set(own.filter(record => record.t === 'map_open' && record.role === role).map(record => record.map));
  const drawn = new Map(own.filter(record => record.t === 'map' && maps.has(record.map) && Number.isInteger(record.frame)
    && typeof record.branch === 'string' && Number.isInteger(record.hour) && Number.isFinite(record.drawn_at))
    .map(record => [`${record.map}:${record.frame}`, record]));
  return own.filter(record => record.t === 'map_present' && record.at >= from && record.at <= to)
    .flatMap(record => {
      const frame = drawn.get(`${record.map}:${record.frame}`);
      return frame ? [{ at: record.at, branch: frame.branch, hour: frame.hour }] : [];
    }).sort((a, b) => a.at - b.at);
}

export function firstPopoutFrame(mapRecords, { session, input, at, popoutTarget }) {
  const first = presentedMapFrames(mapRecords, { session, from: input, to: at }, 'popout')[0];
  let reason = null;
  if (!first) reason = 'the pop-out viewer reports no presented draw, so its first frame is unobservable';
  else if (!popoutTarget?.branch || !Number.isInteger(popoutTarget.hour)) reason = 'the pop-out seek target was not captured';
  else if (first.branch !== popoutTarget.branch || first.hour !== popoutTarget.hour)
    reason = `the first pop-out frame drew branch ${first.branch} hour ${first.hour}, expected branch ${popoutTarget.branch} hour ${popoutTarget.hour}`;
  return { ms: first ? first.at - input : null, presentedAt: first?.at ?? null, reason };
}

export function presentedHours(records, mapRecords, { view, from, to, session, branch }) {
  if (view === null) {
    const presented = presentedMapFrames(mapRecords, { session, from, to }, 'main').filter(frame => frame.branch === branch);
    return presented.length ? new Set(presented.map(frame => frame.hour)).size : null;
  }
  const drawn = records.filter(record => record.t === 'map_state' && record.session === session && record.view === view
    && !record.blank && Number.isFinite(record.hour) && Number.isFinite(record.drawn_at)
    && record.at >= from && record.at <= to);
  const presented = drawn.map(record => ({ hour: record.hour, at: presentedAt(mapRecords, record.drawn_at, session) }))
    .filter(record => record.at !== null && record.at <= to);
  if (!presented.length || !mapFrames(mapRecords, from, to, session).work.n) return null;
  return new Set(presented.filter(record => record.at >= from).map(record => record.hour)).size;
}

export const UNPRESENTED = "its frame is not in the viewer's frame log, so when it was presented is unknown";

export function mapSeek(acks, input, mapRecords) {
  const before = acks.filter(ack => ack.at <= input).at(-1);
  const from = before?.expected ?? null;
  const done = acks.find(ack => ack.at > input && ack.at <= input + MAP_WAIT_MS && !ack.loading && !ack.blank && ack.hour !== null && ack.expected !== null
    && (from === null || Math.abs(ack.expected - from) >= 1) && Math.abs(ack.hour + (ack.fraction ?? 0) - ack.expected) < 1);
  if (!done) return { ms: null, reason: 'never presented the sought hour within 10 s' };
  if (!Number.isFinite(done.drawn_at)) return { ms: null, reason: 'its acknowledgement carries no drawn time' };
  const presented = presentedAt(mapRecords, done.drawn_at, done.session);
  return presented === null ? { ms: null, reason: UNPRESENTED } : { ms: presented - input, reason: null };
}

export const MAP_FRAME_MS = 1000 / 120;
export const MAP_P99_MS = 7;

export function mapFrames(mapRecords, from, to, session) {
  mapRecords = mapRecords.filter(record => record.session === session);
  const opens = mapRecords.filter(record => record.t === 'map_open' && record.role === 'main');
  const main = new Set(opens.map(record => record.map));
  const next = new Map();
  for (const map of main) {
    const own = mapRecords.filter(record => record.t === 'map' && record.map === map).sort((a, b) => a.at - b.at);
    own.forEach((record, index) => next.set(record, own[index + 1]));
  }
  const frames = mapRecords.filter(record => record.t === 'map' && main.has(record.map) && record.at - record.work_ms <= to
    && Math.max(record.at, next.get(record)?.painted_at ?? record.at) >= from);
  const timed = frames.map(frame => {
    const after = next.get(frame);
    return Number.isFinite(after?.painted_at) && Number.isFinite(after.acquire_ms) ? frame.work_ms + (after.painted_at - frame.at) - after.acquire_ms : null;
  });
  const work = timed.filter(Number.isFinite);
  const untimed = opens.find(record => frames.some(frame => frame.map === record.map) && record.gpu_timer === 'unsupported');
  const measured = new Map(mapRecords.filter(record => record.t === 'map' && Number.isFinite(record.gpu_frame) && Number.isFinite(record.gpu_ms))
    .map(record => [`${record.map}:${record.gpu_frame}`, record.gpu_ms]));
  const failed = new Map(mapRecords.filter(record => record.t === 'map' && Number.isFinite(record.gpu_frame) && record.gpu_ms == null && record.gpu_error)
    .map(record => [`${record.map}:${record.gpu_frame}`, record.gpu_error]));
  const gpu = frames.map(frame => measured.get(`${frame.map}:${frame.frame}`)).filter(Number.isFinite);
  const gpuErrors = [...groupBy(frames.map(frame => failed.get(`${frame.map}:${frame.frame}`)).filter(Boolean), error => error)];
  const q = values => { const sorted = [...values].sort((a, b) => a - b); const rank = p => sorted[Math.min(sorted.length, Math.max(1, Math.ceil(p * sorted.length))) - 1];
    return sorted.length ? { n: sorted.length, p50: rank(0.5), p99: rank(0.99), max: sorted.at(-1) } : { n: 0, p50: null, p99: null, max: null }; };
  const reasons = [];
  const judge = (name, stats) => {
    if (stats.max > MAP_FRAME_MS) reasons.push(`${name} max ${stats.max.toFixed(1)} ms, over ${MAP_FRAME_MS.toFixed(2)} ms`);
    if (stats.p99 > MAP_P99_MS) reasons.push(`${name} p99 ${stats.p99.toFixed(1)} ms, over ${MAP_P99_MS} ms`);
  };
  const workStats = q(work), gpuStats = q(gpu);
  if (!workStats.n) reasons.push(frames.length ? 'the viewer recorded no paint for any frame' : 'the viewer recorded no frames');
  else {
    judge('work', workStats);
    if (workStats.n < frames.length) reasons.push(`${frames.length - workStats.n} of ${frames.length} frames have no recorded paint`);
  }
  if (untimed) reasons.push(`the viewer cannot time the GPU on this adapter: ${untimed.gpu_timer_reason ?? 'no reason given'}`);
  else {
    if (gpuStats.n) judge('GPU', gpuStats);
    for (const [error, list] of gpuErrors) reasons.push(`${list.length} of ${frames.length} frames have no GPU time: ${error}`);
    const unreported = frames.length - gpuStats.n - gpuErrors.reduce((sum, [, list]) => sum + list.length, 0);
    if (unreported) reasons.push(`${unreported} of ${frames.length} frames have no GPU time and no reason (gpu_ms on its map records)`);
  }
  const dropped = work.filter(ms => ms > MAP_FRAME_MS).length;
  return { work: workStats, gpu: gpuStats, dropped, frames: frames.length, fps: frames.length / Math.max(0.001, (to - from) / 1000), pass: reasons.length === 0, reasons };
}

export function framesLate(mapRecords, posted, drawnAt, session) {
  mapRecords = mapRecords.filter(record => record.session === session);
  const main = new Set(mapRecords.filter(record => record.t === 'map_open' && record.role === 'main').map(record => record.map));
  return mapRecords.filter(record => record.t === 'map' && main.has(record.map) && record.at - record.work_ms >= posted && record.at < drawnAt).length;
}

export function shownHours(records, since = -Infinity) {
  return [...records.filter(record => record.t === 'shown'),
    ...records.filter(record => record.t === 'map_state' && !record.loading).map(record => ({ t: 'shown', widget: record.view, at: record.at,
      drawnAt: record.drawn_at ?? null, hour: record.blank || record.error ? null : record.hour }))].filter(record => record.at >= since);
}

export const sessionStart = (starts, at) => Math.max(-Infinity, ...starts.filter(start => start <= at));

export function freshnessByKind(records, playbackResults, hourChecks = [], starts = [], dated = []) {
  const kinds = new Map();
  const entryOf = kind => { const entry = kinds.get(kind) ?? { kind, placements: new Set(), unreported: new Set(), wrong: [], stale: [], worst: null, eligible: null }; kinds.set(kind, entry); return entry; };
  const shown = groupBy(shownHours(records).sort((a, b) => a.at - b.at), record => record.widget);
  for (const check of hourChecks) {
    const entry = entryOf(check.kind);
    const where = `${check.page}: ${check.title}`;
    entry.placements.add(where);
    const since = sessionStart(starts, check.at);
    const last = (shown.get(check.widget) ?? []).filter(record => record.at >= since && record.at <= check.at).at(-1);
    if (!last) entry.unreported.add(where);
    else if (last.hour !== check.hour) entry.wrong.push(`${where} shows hour ${last.hour ?? 'none'}, the clock is at ${check.hour}`);
  }
  for (const result of playbackResults) {
    const pictures = new Map(dated.filter(p => p.from === result.from).map(p => [p.widget, p]));
    let reported;
    for (const widget of (result.widgets ?? []).filter(widget => widget.visible)) {
      const entry = entryOf(widget.kind);
      entry.placements.add(`${result.page}: ${widget.title}`);
      const picture = pictures.get(widget.id);
      if (picture) {
        if (picture.status === 'refused' || picture.status === 'error') { entry.stale.push(`${result.page}: ${picture.status}: ${picture.reason ?? picture.status}`); continue; }
        if (picture.p95 !== null) entry.worst = Math.max(entry.worst ?? -Infinity, picture.p95);
        if (picture.eligibleP95 !== null) entry.eligible = Math.max(entry.eligible ?? -Infinity, picture.eligibleP95);
        if (!picture.pass) entry.stale.push(`${result.page}: failed the worker presentation gate (${picture.future} future frames, exact reads ${picture.exact ? 'passed' : 'failed'})`);
        continue;
      }
      reported ??= new Map(freshness(records, result.from, result.to, sessionStart(starts, result.from)).widgets.map(w => [w.widget, w]));
      const seen = reported.get(widget.id);
      if (!seen) { entry.unreported.add(`${result.page}: ${widget.title}`); continue; }
      if (entry.worst === null || seen.p95 > entry.worst) entry.worst = seen.p95;
      if (!seen.pass) entry.stale.push(`${result.page} ${result.speed} h/s: ${seen.p95 >= Number.MAX_VALUE ? 'no values' : `${seen.p95} h off the clock`} at p95${seen.ahead ? ` (${seen.ahead} frames ahead of it)` : ''}`);
    }
  }
  return [...kinds.values()].map(entry => {
    const reasons = [];
    if (entry.unreported.size) reasons.push(`does not report the hour it shows (${[...entry.unreported].join(', ')})`);
    reasons.push(...entry.wrong, ...entry.stale);
    const pass = reasons.length === 0;
    return { kind: entry.kind, placements: entry.placements.size, worst: entry.worst, eligible: entry.eligible,
      pass, reasons };
  });
}

export function catalogueHour(records, step) {
  if (!step.widget) return { pass: false, reason: 'its widget id was not read' };
  const shown = shownHours(records, step.input).filter(record => record.widget === step.widget && record.at >= step.input && record.at <= step.input + 10_000)
    .sort((a, b) => a.at - b.at);
  if (!shown.length) return { pass: false, reason: 'does not report the hour it shows' };
  const first = shown.findIndex(record => record.hour === step.hour);
  if (first < 0) return { pass: false, reason: `shows hour ${shown.at(-1).hour}, the clock is at ${step.hour}` };
  const strayed = shown.slice(first + 1).find(record => record.hour !== step.hour);
  if (strayed) return { pass: false, hour: strayed.hour, reason: `showed the clock's hour ${step.hour}, then hour ${strayed.hour}` };
  return { pass: true, hour: step.hour, at: shown[first].at, drawnAt: shown[first].drawnAt ?? null };
}

export function handledAt(mapRecords, posted, session) {
  mapRecords = mapRecords.filter(record => record.session === session);
  const main = new Set(mapRecords.filter(record => record.t === 'map_open' && record.role === 'main').map(record => record.map));
  const frames = mapRecords.filter(record => record.t === 'map' && main.has(record.map) && record.at - record.work_ms >= posted);
  return frames.length ? Math.min(...frames.map(record => record.at)) : null;
}

export function frameTimes(records, from, to, refreshHz) {
  const frames = records.filter(r => r.t === 'frame' && r.playing && r.at >= from && r.at <= to).sort((a, b) => a.at - b.at);
  return { count: frames.length, main: quantiles(frames.map(r => r.main)), interval: quantiles(frames.map(r => r.dt)),
    drops: refreshHz ? droppedFrames(frames, refreshHz) : null };
}

export const DROP_REFRESHES = 1.5;
export const SUSTAINED_DROPS = 3;

export function droppedFrames(frames, refreshHz) {
  const limit = DROP_REFRESHES * 1000 / refreshHz;
  let dropped = 0, run = 0, longest = 0;
  for (const frame of frames) {
    if (frame.dt > limit) { dropped++; run++; longest = Math.max(longest, run); } else run = 0;
  }
  return { frames: frames.length, dropped, rate: frames.length ? dropped / frames.length : null, longest, limit,
    pass: frames.length > 0 && longest < SUSTAINED_DROPS };
}

export function freshness(records, from, to, since = -Infinity) {
  const ticks = records.filter(record => record.t === 'tick').sort((a, b) => a.at - b.at);
  const frames = records.filter(record => record.t === 'frame' && record.playing && record.at >= from && record.at <= to);
  const shown = groupBy(shownHours(records, since).sort((a, b) => a.at - b.at), record => record.widget);
  const titles = new Map(records.filter(record => record.t === 'widget').map(record => [record.id, record.title]));
  const at = (list, time) => { let low = 0, high = list.length - 1, found = null; while (low <= high) { const mid = (low + high) >> 1; if (list[mid].at <= time) { found = list[mid]; low = mid + 1; } else high = mid - 1; } return found; };
  const widgets = [];
  for (const [widget, list] of shown) {
    const lags = [];
    let none = 0, ahead = 0;
    for (const frame of frames) {
      const clock = at(ticks, frame.at), value = at(list, frame.at);
      if (!clock) continue;
      if (value?.hour == null) { none++; lags.push(Infinity); continue; }
      const head = clock.hour + (clock.playing ? Math.max(0, frame.at - (clock.composedAt ?? clock.at)) * clock.speed / 1000 : 0);
      const age = Math.floor(head) - value.hour;
      if (age < 0) ahead++;
      lags.push(age);
    }
    if (!lags.length) continue;
    const q = quantiles(lags.map(lag => (Number.isFinite(lag) ? lag : Number.MAX_VALUE)));
    widgets.push({ widget, title: titles.get(widget) ?? widget, frames: lags.length, none, ahead, p50: q.p50, p95: q.p95, max: q.max, pass: q.p95 <= MAX_PLAYBACK_AGE_HOURS && ahead === 0 });
  }
  return { frames: frames.length, widgets };
}

export function windowLosses(records, from, to) {
  const overlapping = (records ?? []).filter(record => record.t === 'lost'
    && (record.from ?? record.at) <= to && (record.to ?? record.at) >= from);
  return {
    count: overlapping.reduce((sum, record) => sum + (record.count ?? 0), 0),
    sources: [...new Set(overlapping.map(record => record.source ?? 'unknown'))],
    records: overlapping,
  };
}

export function losses(records, windows = null) {
  return [...groupBy(records.filter(record => record.t === 'lost'), record => record.source ?? 'unknown')]
    .map(([source, list]) => {
      const from = Math.min(...list.map(record => record.from ?? record.at));
      const to = Math.max(...list.map(record => record.to ?? record.at));
      const base = { source, count: list.reduce((sum, record) => sum + (record.count ?? 0), 0), from, to };
      if (!Array.isArray(windows)) return base;
      const hit = [...new Set(windows.filter(w => w.from != null && w.to != null && list.some(r => (r.from ?? r.at) <= w.to && (r.to ?? r.at) >= w.from)).map(w => w.label))];
      return { ...base, windows: hit.length ? hit : ['outside timed windows'] };
    });
}

export function failedReplies(records, finished = new Map()) {
  let shutdown = 0;
  const failed = [];
  for (const record of records) {
    if (record.t !== 'q' || record.outcome !== 'error') continue;
    const end = finished.get(record.session);
    const closed = record.failure === 'failed: The dashboard is closing' || record.failure === 'failed: The dashboard data connection closed';
    if (closed && record.at + record.ms > end) shutdown++;
    else failed.push(record);
  }
  const failures = [...groupBy(failed, record => record.product)].map(([product, list]) => ({ product, count: list.length, first: list[0].failure ?? 'no failure text recorded', hours: [...new Set(list.map(record => record.hour))].slice(0, 5) }));
  return { failures, shutdown };
}

export const consoleFailures = messages => messages.map(message => ({ ...message, pass: false }));


export function sought(result, shownBy, starts = [], nextInput = Infinity) {
  if (!result.widgetIds || result.input == null) return result;
  const reasons = [...result.reasons];
  let ms = result.ms;
  const start = sessionStart(starts, result.input);
  const end = Math.min(nextInput, ...starts.filter(at => at > result.input));
  for (const [id, title] of Object.entries(result.widgetIds)) {
    const list = (shownBy.get(id) ?? []).filter(r => r.at >= start && r.at < end);
    const before = list.findLast(r => r.at < result.input);
    const after = list.filter(r => r.at >= result.input);
    const pin = result.pins ? result.pins[id] : before?.fixedHour ?? after[0]?.fixedHour;
    const target = pin ?? result.hour;
    const matches = r => (r.axisHour ?? r.hour) === target && (r.fixedHour == null || r.fixedHour === pin);
    let reached = pin != null && before && matches(before) ? result.input : null;
    for (const r of after) {
      if (matches(r)) reached ??= r.at;
      else if (reached !== null || (pin != null && r.fixedHour != null && r.fixedHour !== pin)) {
        reasons.push(`${title} left hour ${target} before the next input`);
        break;
      }
    }
    if (reached === null || reached - result.input > MAP_WAIT_MS) reasons.push(`${title} never showed hour ${target} within 10 s`);
    else ms = Math.max(ms ?? 0, reached - result.input);
  }
  if (result.check !== false && ms > BUDGET_MS && !reasons.some(r => r.includes(' ms, over'))) reasons.push(`${Math.round(ms)} ms, over ${BUDGET_MS} ms`);
  return { ...result, ms, reasons, pass: reasons.length === 0 };
}
