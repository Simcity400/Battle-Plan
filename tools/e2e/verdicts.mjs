// The suite's verdicts from what a run measured: pure functions, so they can be tested without the app.

export const BUDGET_MS = 100;
/** One frame at 95 Hz, the host's frame budget (crates/desktop/src/perf.rs). */
export const FRAME_BUDGET_MS = 1000 / 95;
/** Preparation regressions beyond these shares of the baseline fail; wall time is the noisiest. */
export const PREP_MARGINS = { wallSeconds: 0.25, cpuSeconds: 0.15, peakCommitGb: 0.10 };
/** The baseline is the median of this many latest passing preparations. */
export const PREP_HISTORY = 5;
/** How long a seek may take the map to present its hour. */
export const MAP_WAIT_MS = 10_000;

/** Nearest-rank percentiles, as the host's perf summary computes them. */
export function quantiles(values) {
  const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
  if (!sorted.length) return { n: 0, p50: null, p95: null, max: null };
  const rank = p => sorted[Math.min(sorted.length, Math.max(1, Math.ceil(p * sorted.length))) - 1];
  return { n: sorted.length, p50: rank(0.5), p95: rank(0.95), max: sorted.at(-1) };
}

/** An item: its samples' quantiles, failing when any sample failed or the maximum is over budget. */
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

/** Each widget placement's first content in page visits, by kind and pass: from the input that showed it. */
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
      // The rule judges a first visit; a revisit is reported beside it.
      if (visit === 'cold' && ms !== null && ms > BUDGET_MS) reasons.push(`${Math.round(ms)} ms, over ${BUDGET_MS} ms`);
      placements.push({ visit, page, kind: widget.kind, title: widget.title, step: step.label, id: `${page}#${widget.id}`, ms, end: widget.end,
        reasons, label: `${page}: ${widget.title} (${step.label})` });
    }
  }
  // A placement seen in several steps of one visit counts once: where it first showed content.
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

/** The map views' acknowledgements, from the perf log, by view, in time order. */
export function mapAcks(records) {
  const views = groupBy(records.filter(record => record.t === 'map_state'), record => record.view);
  for (const acks of views.values()) acks.sort((a, b) => a.at - b.at);
  return views;
}

/** When the viewer presented the frame it drew at `drawnAt` (its map state's `drawn_at`, stamped as the frame's interface
 * is built): that frame's record ends its interface pass, and the record after it carries when the paint that presented
 * it ended. Null when the frame log does not hold that frame. */
export function presentedAt(mapRecords, drawnAt) {
  if (!Number.isFinite(drawnAt)) return null;
  const main = new Set(mapRecords.filter(record => record.t === 'map_open' && record.role === 'main').map(record => record.pid));
  for (const pid of main) {
    const own = mapRecords.filter(record => record.t === 'map' && record.pid === pid).sort((a, b) => a.at - b.at);
    const index = own.findIndex(record => record.at - record.work_ms <= drawnAt && drawnAt <= record.at);
    const next = index < 0 ? null : own[index + 1];
    if (next && Number.isFinite(next.painted_at)) return next.painted_at;
  }
  return null;
}

/** When the pop-out's viewer presented its first frame of `hour` after the input at `after`: the end of the first paint
 * its frame log records once it installed that hour on screen (`map_upload`). Null when it records none. */
export function popoutPresented(mapRecords, after, hour) {
  const popouts = new Set(mapRecords.filter(record => record.t === 'map_open' && record.role === 'popout').map(record => record.pid));
  const installed = mapRecords.filter(record => record.t === 'map_upload' && popouts.has(record.pid) && record.ok && record.hour === hour && record.at > after);
  if (!installed.length) return null;
  const first = installed.reduce((a, b) => (b.at < a.at ? b : a));
  const paints = mapRecords.filter(record => record.t === 'map' && record.pid === first.pid && Number.isFinite(record.painted_at) && record.painted_at > first.at);
  return paints.length ? Math.min(...paints.map(record => record.painted_at)) : null;
}

/** The hours the main viewer installed on screen for map view `view` (null: the full campaign map) over [from, to]. */
export function hoursInstalled(mapRecords, view, from, to) {
  const main = new Set(mapRecords.filter(record => record.t === 'map_open' && record.role === 'main').map(record => record.pid));
  return new Set(mapRecords.filter(record => record.t === 'map_upload' && main.has(record.pid) && record.ok && (record.view ?? null) === view
    && record.at >= from && record.at <= to).map(record => record.hour)).size;
}

/** The reason a map frame's presentation cannot be timed. */
export const UNPRESENTED = "its frame is not in the viewer's frame log, so when it was presented is unknown";

/** How long each seek took the map views to present the sought position: the first acknowledgement after the
 * input, not loading, showing what the view should show, once what it should show moved; timed to the present
 * of the frame that drew it. */
export function mapSeek(acks, input, mapRecords) {
  const before = acks.filter(ack => ack.at <= input).at(-1);
  const from = before?.expected ?? null;
  const done = acks.find(ack => ack.at > input && ack.at <= input + MAP_WAIT_MS && !ack.loading && !ack.blank && ack.hour !== null && ack.expected !== null
    && (from === null || Math.abs(ack.expected - from) >= 1) && Math.abs(ack.hour + (ack.fraction ?? 0) - ack.expected) < 1);
  if (!done) return { ms: null, reason: 'never presented the sought hour within 10 s' };
  if (!Number.isFinite(done.drawn_at)) return { ms: null, reason: 'its acknowledgement carries no drawn time' };
  const presented = presentedAt(mapRecords, done.drawn_at);
  return presented === null ? { ms: null, reason: UNPRESENTED } : { ms: presented - input, reason: null };
}

/** The map's own target: every frame of an interaction within one 120 Hz frame, and nearly all well within it. */
export const MAP_FRAME_MS = 1000 / 120;
export const MAP_P99_MS = 7;

/** The main viewer's frames over [from, to] judged against 120 Hz: window-thread work (the frame's logic and UI, and
 * the CPU part of its paint) and GPU time per frame. A
 * frame whose work exceeds one 120 Hz frame is a dropped frame at 120 Hz, whatever the display shows. `fps` is the
 * frame rate achieved, meaningful when the viewer presents uncapped.
 * GPU time arrives later than its frame: each `map` record names its own `frame` (counted per viewer process), and a
 * later record carries `gpu_frame` (the frame measured) with its `gpu_ms` once that frame's timestamps resolve, or
 * null `gpu_ms` with why it has none (`gpu_error`). The process's `map_open` says whether it can time the GPU at all
 * (`gpu_timer`). */
export function mapFrames(mapRecords, from, to) {
  const opens = mapRecords.filter(record => record.t === 'map_open' && record.role === 'main');
  const main = new Set(opens.map(record => record.pid));
  // Each record carries when the paint that ended the frame before it finished: the frame's work runs from its interface
  // pass through egui's output, tessellation and paint to then, less the paint's wait for a swapchain image.
  const next = new Map();
  for (const pid of main) {
    const own = mapRecords.filter(record => record.t === 'map' && record.pid === pid).sort((a, b) => a.at - b.at);
    own.forEach((record, index) => next.set(record, own[index + 1]));
  }
  // Every frame whose run, from its start through its paint, overlaps [from, to].
  const frames = mapRecords.filter(record => record.t === 'map' && main.has(record.pid) && record.at - record.work_ms <= to
    && Math.max(record.at, next.get(record)?.painted_at ?? record.at) >= from);
  const timed = frames.map(frame => {
    const after = next.get(frame);
    return Number.isFinite(after?.painted_at) && Number.isFinite(after.acquire_ms) ? frame.work_ms + (after.painted_at - frame.at) - after.acquire_ms : null;
  });
  const work = timed.filter(Number.isFinite);
  const untimed = opens.find(record => frames.some(frame => frame.pid === record.pid) && record.gpu_timer === 'unsupported');
  const measured = new Map(mapRecords.filter(record => record.t === 'map' && Number.isFinite(record.gpu_frame) && Number.isFinite(record.gpu_ms))
    .map(record => [`${record.pid}:${record.gpu_frame}`, record.gpu_ms]));
  const failed = new Map(mapRecords.filter(record => record.t === 'map' && Number.isFinite(record.gpu_frame) && record.gpu_ms == null && record.gpu_error)
    .map(record => [`${record.pid}:${record.gpu_frame}`, record.gpu_error]));
  const gpu = frames.map(frame => measured.get(`${frame.pid}:${frame.frame}`)).filter(Number.isFinite);
  const gpuErrors = [...groupBy(frames.map(frame => failed.get(`${frame.pid}:${frame.frame}`)).filter(Boolean), error => error)];
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

/** Viewer frames that began after an input and ended before the frame that showed its result: zero is the next frame. */
export function framesLate(mapRecords, posted, drawnAt) {
  const main = new Set(mapRecords.filter(record => record.t === 'map_open' && record.role === 'main').map(record => record.pid));
  return mapRecords.filter(record => record.t === 'map' && main.has(record.pid) && record.at - record.work_ms >= posted && record.at < drawnAt).length;
}

/** Every report of the hour a widget shows: its `shown` records, and a map view's acknowledged frames, where a blank or
 * failed map shows no hour. Only reports since `since` (its app session's start) count. */
export function shownHours(records, since = -Infinity) {
  return [...records.filter(record => record.t === 'shown'),
    ...records.filter(record => record.t === 'map_state' && !record.loading).map(record => ({ t: 'shown', widget: record.view, at: record.at,
      drawnAt: record.drawn_at ?? null, hour: record.blank || record.error ? null : record.hour }))].filter(record => record.at >= since);
}

/** The start of the app session `at` falls in, from the sessions' start times. */
export const sessionStart = (starts, at) => Math.max(-Infinity, ...starts.filter(start => start <= at));

/** Per widget kind on the default pages. Paused, each placement scrolled into view must report showing the clock's
 * hour (`hours`: one check per placement, at its epoch `at`, with the clock's `hour`). Playing, each placement in view
 * must report, and lag the clock by under one tick; out of view it reports none, as it shows none. */
export function freshnessByKind(records, playbackResults, hourChecks = [], starts = []) {
  const kinds = new Map();
  const entryOf = kind => { const entry = kinds.get(kind) ?? { kind, placements: new Set(), unreported: new Set(), wrong: [], stale: [], worst: null }; kinds.set(kind, entry); return entry; };
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
    const fresh = freshness(records, result.from, result.to, sessionStart(starts, result.from));
    const reported = new Map(fresh.widgets.map(widget => [widget.widget, widget]));
    for (const widget of (result.widgets ?? []).filter(widget => widget.visible)) {
      const entry = entryOf(widget.kind);
      entry.placements.add(`${result.page}: ${widget.title}`);
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
    return { kind: entry.kind, placements: entry.placements.size, worst: entry.worst, pass: reasons.length === 0, reasons };
  });
}

/** Whether an added catalogue view reported the hour it shows, and that it is the clock's: within the 10 s after Add,
 * when it first showed the clock's hour (`at`, epoch ms), and that it showed no other after it. */
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

/** When the main viewer handled pointer input sent at `posted`: the end of its first frame begun after it, the latest the
 * game's hover delay can have started. Null when its frame log holds no such frame. */
export function handledAt(mapRecords, posted) {
  const main = new Set(mapRecords.filter(record => record.t === 'map_open' && record.role === 'main').map(record => record.pid));
  const frames = mapRecords.filter(record => record.t === 'map' && main.has(record.pid) && record.at - record.work_ms >= posted);
  return frames.length ? Math.min(...frames.map(record => record.at)) : null;
}

/** The playback frame times: each playing frame's main-thread time to render it (the host's `main`). */
export function frameTimes(summary) {
  const main = summary?.playing_main_ms;
  return main?.count ? { n: main.count, p50: main.p50, p95: main.p95, max: main.max } : { n: 0, p50: null, p95: null, max: null };
}

/** A playing frame interval longer than this many display refreshes is a dropped frame. */
export const DROP_REFRESHES = 1.5;
/** This many dropped frames in a row is stutter a user sees, whatever the main thread's time. */
export const SUSTAINED_DROPS = 3;

/** Dropped frames over the playing frames of [from, to]: their share and the longest run of them in a row. */
export function droppedFrames(records, from, to, refreshHz) {
  const limit = DROP_REFRESHES * 1000 / refreshHz;
  const frames = records.filter(record => record.t === 'frame' && record.playing && record.at >= from && record.at <= to && Number.isFinite(record.dt))
    .sort((a, b) => a.at - b.at);
  let dropped = 0, run = 0, longest = 0;
  for (const frame of frames) {
    if (frame.dt > limit) { dropped++; run++; longest = Math.max(longest, run); } else run = 0;
  }
  return { frames: frames.length, dropped, rate: frames.length ? dropped / frames.length : null, longest, limit,
    pass: frames.length > 0 && longest < SUSTAINED_DROPS };
}

/** Per widget that reports the hour it shows, over the playing frames of [from, to]: how many hours its values
 * lagged the clock's hour (no values counts as never current). `step` is the hours one data tick moves. */
export function freshness(records, from, to, since = -Infinity) {
  const ticks = records.filter(record => record.t === 'tick').sort((a, b) => a.at - b.at);
  const frames = records.filter(record => record.t === 'frame' && record.playing && record.at >= from && record.at <= to);
  const shown = groupBy(shownHours(records, since).sort((a, b) => a.at - b.at), record => record.widget);
  const titles = new Map(records.filter(record => record.t === 'widget').map(record => [record.id, record.title]));
  const at = (list, time) => { let low = 0, high = list.length - 1, found = null; while (low <= high) { const mid = (low + high) >> 1; if (list[mid].at <= time) { found = list[mid]; low = mid + 1; } else high = mid - 1; } return found; };
  const speed = at(ticks, frames[0]?.at ?? from)?.speed ?? 1;
  // The clock tells data hours on a lattice of this many hours per quarter second of play.
  const step = Math.max(1, Math.ceil(speed * 0.25));
  const widgets = [];
  for (const [widget, list] of shown) {
    const lags = [];
    let none = 0, ahead = 0;
    for (const frame of frames) {
      const clock = at(ticks, frame.at), value = at(list, frame.at);
      if (!clock) continue;
      if (value?.hour == null) { none++; lags.push(Infinity); continue; }
      // Ahead of the clock by more than the tick it may have just told is a value the clock never reached.
      const off = Math.floor(clock.hour) - value.hour;
      if (off < -step) ahead++;
      lags.push(off < -step ? -off : Math.max(0, off));
    }
    if (!lags.length) continue;
    const q = quantiles(lags.map(lag => (Number.isFinite(lag) ? lag : Number.MAX_VALUE)));
    widgets.push({ widget, title: titles.get(widget) ?? widget, frames: lags.length, none, ahead, step, p50: q.p50, p95: q.p95, max: q.max, pass: q.p95 <= step && ahead === 0 });
  }
  return { step, frames: frames.length, widgets };
}

/** Records the perf logs dropped, by source: any loss can hide a failure or a slow frame, so each fails the run. */
export function losses(records) {
  return [...groupBy(records.filter(record => record.t === 'lost'), record => record.source ?? 'unknown')]
    .map(([source, list]) => ({ source, count: list.reduce((sum, record) => sum + (record.count ?? 0), 0), from: Math.min(...list.map(record => record.from ?? record.at)), to: Math.max(...list.map(record => record.to ?? record.at)) }));
}

/** Failed or refused replies in the perf log, by product. A reply that failed on its closed connection after its session's
 * sweep finished was cut off by the suite closing the app, not failed by it: `sessions` are those sweeps' [from, to]
 * in epoch ms, and a record's `at` is when it was asked. */
export function failedReplies(records, sessions = []) {
  const cutOff = record => /connection closed/i.test(record.failure ?? '')
    && sessions.some(({ from, to }) => record.at >= from && record.at <= to && record.at + record.ms > to);
  const failed = records.filter(record => record.t === 'q' && record.outcome === 'error' && !cutOff(record));
  return [...groupBy(failed, record => record.product)].map(([product, list]) => ({ product, count: list.length, first: list[0].failure ?? 'no failure text recorded', hours: [...new Set(list.map(record => record.hour))].slice(0, 5) }));
}

/** Console output from our code: everything but the browser's own advice, which `cdp.mjs` already left out. */
export const consoleFailures = messages => messages.map(message => ({ ...message, pass: false }));

/** A preparation against the median of the latest passing ones. The first run has no baseline and says so. */
export function preparation(prep, history) {
  const reasons = [];
  if (prep.state !== 'ready') reasons.push(`ended ${prep.state}${prep.error ? `: ${prep.error}` : ''}`);
  for (const failed of prep.failedSections ?? []) reasons.push(failed);
  if (prep.exitCode !== 0 && prep.exitCode !== undefined) reasons.push(`the app exited with ${prep.exitCode} after preparing`);
  const passing = history.filter(entry => entry.pass).slice(-PREP_HISTORY);
  const metrics = Object.entries(PREP_MARGINS).map(([metric, margin]) => {
    const values = passing.map(entry => entry[metric]).filter(Number.isFinite).sort((a, b) => a - b);
    const baseline = values.length ? values[Math.floor((values.length - 1) / 2)] : null;
    const value = prep[metric];
    const limit = baseline === null ? null : baseline * (1 + margin);
    const over = limit !== null && Number.isFinite(value) && value > limit;
    if (!Number.isFinite(value)) reasons.push(`${metric} was not measured`);
    if (over) reasons.push(`${metric} ${value.toFixed(2)} over ${limit.toFixed(2)} (baseline ${baseline.toFixed(2)} + ${Math.round(margin * 100)}%)`);
    return { metric, value, baseline, margin, limit, runs: values.length, pass: !over && Number.isFinite(value) };
  });
  return { pass: reasons.length === 0, reasons, metrics, baselineRuns: passing.length };
}
