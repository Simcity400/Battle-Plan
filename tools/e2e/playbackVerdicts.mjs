// The playback phase's verdicts: the bars of "completely smooth at 1,440 game hours a second", judged from what a run
// left behind (the viewer's paced harness and proof tool, the desktop app's perf log). Pure functions, as verdicts.mjs.

/** A row's verdict. A run that is not whole judges nothing: its rows are `partial`. */
export const PASS = 'pass', FAIL = 'fail', PARTIAL = 'partial', UNMEASURABLE = 'not measurable yet', REPORTED = 'reported';

/** The map's bars, in frames of the refresh interval and steps of the head. */
export const MAP = { threadP99Ms: 6, threadWorstMs: 10, intervalP99: 1.1, intervalWorst: 1.5, lagP99Steps: 1, lagWorstSteps: 2,
  seekWorstMs: 150, seekMedianMs: 100, heldPeakBytes: 220e6 };
/** Each layer's own bar, apply + settle + upload of the frame thread's own CPU, from its builder's brief. */
export const LAYER_BARS = {
  political: { p99: 0.3, worst: 1, from: 'P5d' },
  sounds: { p99: 0.5, worst: 1.5, from: 'P9' }, strikes: { p99: 0.5, worst: 1.5, from: 'P9' }, engagements: { p99: 0.5, worst: 1.5, from: 'P9' },
  vision: { p99: 0.5, worst: 1.5, from: 'P6' }, intel: { p99: 0.5, worst: 1.5, from: 'P6' }, 'intel icons': { p99: 0.5, worst: 1.5, from: 'P6' }, snow: { p99: 0.5, worst: 1.5, from: 'P6' },
  weather: { p99: 0.5, worst: 1.5, from: 'P6' }, radar: { p99: 0.5, worst: 1.5, from: 'P6' },
  rails: { p99: 0.3, worst: 1, from: 'P7' }, names: { p99: 0.3, worst: 1, from: 'P7' }, labels: { p99: 0.3, worst: 1, from: 'P7' },
  units: { p99: 1, worst: 3, from: 'P8a' }, convoys: { p99: 1, worst: 3, from: 'P8a' }, counters: { p99: 1, worst: 3, from: 'P8b' },
  naval_counters: { p99: 0.8, worst: 3, from: 'P8d' },
};
const SCOPES = ['apply', 'settle', 'upload'];
/** Under this much physical memory free during a run the machine was paging, and its times are not a result. */
export const PAGING_FREE_MB = 2048;

/** Why the runner's record says a run's machine was not fit: it ran short of memory. */
function machineReasons(run) {
  const least = run.memory?.least_free_mb;
  return Number.isFinite(least) && least < PAGING_FREE_MB ? [`the machine was paging: ${least} MB of memory free at the least, under ${PAGING_FREE_MB}`] : [];
}
/** The dashboard's bars. */
export const DASHBOARD = { intervalP99: 1.1, intervalWorst: 2, ageWorstSteps: 2, tileBins: 256 };
/** Commits, requests and ages within this long after play, pause, a speed change or a seek are the anchor's, not a value's. */
export const ANCHOR_SETTLE_MS = 300;
/** Perf-log records that happen at a time, as against those that state something for the session. */
const TIMED = new Set(['tick', 'frame', 'q', 'loaf', 'longtask', 'follow_message', 'shown', 'hour']);
/** Products of the follow protocol itself: sent on anchors, never for a value. */
export const FOLLOW_PRODUCTS = new Set(['follow', 'unfollow', 'follow-clock', 'follow-close']);

/** Today's figure beside each bar (P0, quiet machine, the game open and paused). */
export const TODAY = {
  thread: '9.0 median, 14.6 worst when an hour installs',
  interval: 'not measured',
  lag: '770–840 / 1,200–1,224 / 1,294–1,440 h (p50 / p99 / worst)',
  pictures: '4.5–4.7 a second',
  step: '7.5 ms median, 128 worst (move and frame of rows)',
  seek: '102–313 ms before the map\'s 0.3 s',
  memory: 'cursor 1,228 MB; process 4.7 GB',
  webview: 'not measured',
  commits: 'one transition per 250 ms tick',
  requests: '26 a second for seven widgets',
  age: 'up to 360 h (a 250 ms tick)',
};

/** The value at nearest rank p of `values` (finite numbers), as the harness and verdicts.mjs compute it. */
export function rank(values, p) {
  const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
  return sorted.length ? sorted[Math.min(sorted.length, Math.max(1, Math.ceil(p * sorted.length))) - 1] : null;
}

/** The value after `flag` in a command's arguments. */
export function arg(args, flag) {
  const at = (args ?? []).indexOf(flag);
  return at < 0 ? null : args[at + 1];
}

const num = (args, flag) => { const value = arg(args, flag); return value === null ? null : Number(value); };
const ms = us => us / 1000;
const fixed = (value, digits = 1) => (Number.isFinite(value) ? value.toFixed(digits) : '–');

/** Index of the largest of `values` (by `of`), or null. */
function worstOf(items, of) {
  let best = null;
  for (const item of items) if (Number.isFinite(of(item)) && (best === null || of(item) > of(best))) best = item;
  return best;
}

/** Whether a paced or step-mode harness run covered what it was asked: exit 0 and not stopped, a report and an uncut frame
 * log, consecutive frames, the span asked, the head at the end (paced), every frame asked (step), every seek made. */
export function wholeHarness(run, report, frames, { acceptReconciliation = false } = {}) {
  const reasons = machineReasons(run);
  if (run.stopped) reasons.push(`stopped by the runner: ${run.stopped}`);
  // The harness exits 1 when its exclusive CPU scopes and the process's CPU differ by over 5%, with the run complete.
  const check = report?.cpu_reconciliation;
  const reconciliation = run.exit === 1 && check?.passed === false;
  const off = reconciliation ? `${(100 * check.process_minus_scopes_ns / check.process_cpu_ns).toFixed(1)}% on ${(check.process_cpu_ns / 1e9).toFixed(1)} s of CPU` : '';
  if (run.exit !== 0 && !(acceptReconciliation && reconciliation)) reasons.push(reconciliation ? `exit 1: the harness's CPU reconciliation failed (process CPU off its scopes by ${off})` : `exit ${run.exit ?? 'not recorded'}`);
  const from = num(run.args, '--from-hour'), hours = num(run.args, '--hours');
  const held = (run.args ?? []).includes('--paused');
  if (!report) reasons.push('no report.json');
  if (!frames) reasons.push('no frames.jsonl');
  else if (frames.cut) reasons.push('frames.jsonl ends in a cut line');
  // A run cut short leaves the frames it covered: say how far it came.
  if (!report && frames?.frames.length) {
    const last = frames.frames.at(-1);
    reasons.push(held ? `covered ${frames.frames.length} of ${hours} frames` : `covered ${frames.frames.length} frames, the head to ${Math.floor(last.head_hour)} of ${from + hours}`);
  }
  if (!report || !frames) return { whole: false, reasons };
  const list = frames.frames;
  if (report.from_hour !== from || report.game_hours !== hours) reasons.push(`the report covers ${report.from_hour} + ${report.game_hours} h, asked ${from} + ${hours}`);
  const gaps = list.filter((frame, index) => frame.frame !== index).length;
  if (gaps) reasons.push(`${gaps} frames out of sequence`);
  if (todayOff(run) && report.layers_only?.on !== true) reasons.push('asked with today\'s pipeline off, the report says it ran');
  if (arg(run.args, '--paced') !== null) {
    const paced = report.paced;
    if (!paced) reasons.push('no paced section in the report');
    else if (held) {
      if (list.length !== hours) reasons.push(`${list.length} of ${hours} frames at the held hour`);
    } else {
      if (paced.frames !== list.length) reasons.push(`${list.length} frames logged of ${paced.frames} reported`);
      const last = list.at(-1);
      // The harness stops before the frame whose head reaches the end: the last head plus its travel to the stop.
      const reached = last ? last.head_hour + paced.speed_hours_per_second * (paced.seconds - last.start_us / 1e6) : null;
      if (reached === null || reached < from + hours) reasons.push(`the head reached ${reached === null ? 'nothing' : Math.floor(reached)}, asked ${from + hours}`);
    }
    const seeks = num(run.args, '--seeks');
    if (seeks) {
      const made = (report.layers?.seeks ?? []).filter(seek => seek.frames_to_shown != null).length;
      if (made < seeks) reasons.push(`${made} of ${seeks} seeks shown`);
    }
  } else if (list.length !== hours) reasons.push(`${list.length} of ${hours} frames`);
  return { whole: reasons.length === 0, reasons, accepted: reasons.length === 0 && acceptReconciliation && reconciliation ? `judged with exit 1 from the harness's CPU reconciliation only (${off}): not a result under the rule` : null };
}

/** Whether a proof-tool run covered what it was asked: exit 0, flat play to the last hour, every seek, frames, and
 * (unless timing) samples compared with none skipped. */
export function wholeProof(run, summary, frames) {
  const reasons = machineReasons(run);
  if (run.stopped) reasons.push(`stopped by the runner: ${run.stopped}`);
  if (run.exit !== 0) reasons.push(`exit ${run.exit ?? 'not recorded'}`);
  if (!summary) reasons.push('no summary.json');
  if (!frames) reasons.push('no frames.jsonl');
  else if (frames.cut) reasons.push('frames.jsonl ends in a cut line');
  if (!summary) return { whole: false, reasons };
  const reached = summary.flat?.last ?? null;
  if (reached === null || reached < summary.last) reasons.push(`flat play reached ${reached} of ${summary.last}`);
  const seeks = num(run.args, '--seeks') ?? 20;
  if ((summary.resets ?? []).length < seeks) reasons.push(`${(summary.resets ?? []).length} of ${seeks} seeks`);
  if (!summary.frames?.count) reasons.push('no frames recorded');
  if (!summary.timing) {
    const samples = (summary.compared ?? []).map(entry => entry.samples);
    if (!samples.length || Math.min(...samples) === 0) reasons.push(`samples compared ${JSON.stringify(samples)}`);
    if (summary.skipped_samples) reasons.push(`${summary.skipped_samples} samples skipped`);
  }
  return { whole: reasons.length === 0, reasons };
}

/** Whether a run was asked with today's pipeline off: by the runner's record or by the viewer's own flag. */
const todayOff = run => Boolean(run.todayOff) || ['--layers-only', '--today-off'].some(flag => (run.args ?? []).includes(flag));

const row = (fields) => ({ worst: null, today: '', note: '', values: {}, ...fields });

/** Every row of a run that is not whole: named, with no figure. */
function partialRows(page, label, bars, reasons) {
  return bars.map(bar => row({ page, run: label, bar, measured: '–', limit: '', verdict: PARTIAL, note: reasons.join('; ') }));
}

const PACED_BARS = ['window-thread CPU a frame', 'presented frame interval', 'late frames against the floor', 'content lag', 'new pictures a second',
  'each layer: apply, settle and upload', 'playhead step time', 'playhead stalls', 'playhead held()', 'process peak memory'];

/** The presented time of each frame: its start, its work and its wait for the GPU. */
const presented = frame => frame.start_us + frame.wall_us;

/** The frames of seek windows: from the seek's frame until it is back within a step (or shown, when it never is). */
function seekWindows(report) {
  const inside = new Set();
  for (const seek of report.layers?.seeks ?? []) {
    const span = seek.frames_to_lag_within_a_step ?? seek.frames_to_shown ?? Infinity;
    for (let index = seek.frame; index <= seek.frame + Math.min(span, 1e6); index++) inside.add(index);
  }
  return inside;
}

/** The rows of one paced harness run against the map's bars. `floor` is the floor run's late-frame rate, if one ran. */
export function pacedRows({ page = 'map', label, run, report, frames, floor = null, acceptReconciliation = false }) {
  const whole = wholeHarness(run, report, frames, { acceptReconciliation });
  // Runs before the format-12 swap named their unit files; since, the recording's playhead is on unless --no-playhead.
  const layered = Boolean(run.playhead ?? run.units);
  const todayOn = !todayOff(run);
  const bars = [...PACED_BARS, ...(num(run.args, '--seeks') ? ['seek: sought hour on screen'] : [])];
  if (!whole.whole) return partialRows(page, label, bars, whole.reasons);
  const list = frames.frames, paced = report.paced;
  const hz = paced.refresh_hz, interval = 1e6 / hz, step = paced.speed_hours_per_second / hz;
  const shownAt = frame => (layered ? frame.layers?.drawn != null : frame.drawn_hour != null);
  const first = list.findIndex(shownAt);
  if (first < 0) return partialRows(page, label, bars, []).map(entry => ({ ...entry, verdict: FAIL, measured: 'no picture was drawn', note: `none in ${list.length} frames of a whole run` }));
  const measured = list.slice(first);
  const at = frame => (frame ? { frame: frame.frame, hour: frame.head_hour } : null);
  const rows = [];
  const firstNote = `${whole.accepted ? `${whole.accepted}; ` : ''}from the first picture, frame ${first} at ${fixed(ms(list[first].start_us), 0)} ms; ${first} frames before it`;

  const thread = measured.map(frame => frame.thread_us);
  const threadP99 = ms(rank(thread, 0.99)), threadWorst = worstOf(measured, frame => frame.thread_us);
  rows.push(row({ page, run: label, bar: 'window-thread CPU a frame', measured: `p99 ${fixed(threadP99, 2)} / worst ${fixed(ms(threadWorst.thread_us), 2)} ms`,
    limit: `p99 ${MAP.threadP99Ms} / worst ${MAP.threadWorstMs} ms`, verdict: threadP99 <= MAP.threadP99Ms && ms(threadWorst.thread_us) <= MAP.threadWorstMs ? PASS : FAIL,
    worst: at(threadWorst), today: TODAY.thread, note: firstNote, values: { p99: threadP99, worst: ms(threadWorst.thread_us) } }));

  const intervals = measured.slice(1).map((frame, index) => ({ frame, us: presented(frame) - presented(measured[index]) }));
  const intervalP99 = rank(intervals.map(entry => entry.us), 0.99) / interval;
  const longest = worstOf(intervals, entry => entry.us);
  const late = intervals.filter(entry => entry.us > MAP.intervalWorst * interval).length;
  rows.push(row({ page, run: label, bar: 'presented frame interval', measured: `p99 ${fixed(intervalP99, 2)} / worst ${fixed(longest ? longest.us / interval : null, 2)} intervals; ${late} over ${MAP.intervalWorst}`,
    limit: `p99 ${MAP.intervalP99} / none over ${MAP.intervalWorst} intervals (${fixed(ms(interval), 2)} ms)`,
    verdict: intervalP99 <= MAP.intervalP99 && late === 0 ? PASS : FAIL, worst: at(longest?.frame), today: TODAY.interval,
    note: 'a frame is presented at its start plus its work and its wait for the GPU', values: { p99: intervalP99, worst: longest ? longest.us / interval : null, over: late } }));

  const rate = intervals.length ? late / intervals.length : null;
  rows.push(row({ page, run: label, bar: 'late frames against the floor', measured: `${late} of ${intervals.length} (${fixed(rate * 1000, 2)} per 1,000)`,
    limit: floor ? `at most the floor's ${fixed(floor.rate * 1000, 2)} per 1,000 (${floor.late} of ${floor.frames}, the same view paused, the camera panning)` : 'the floor run\'s rate',
    verdict: floor ? (rate <= floor.rate ? PASS : FAIL) : UNMEASURABLE, worst: at(longest?.frame),
    note: floor ? `late: presented over ${MAP.intervalWorst} intervals after the frame before` : 'no floor run in this phase', values: { per1000: rate * 1000 } }));

  const outside = seekWindows(report);
  const lagRow = (side, lagOf) => {
    const judged = measured.filter(frame => !outside.has(frame.frame));
    const missing = judged.filter(frame => lagOf(frame) == null);
    const lags = judged.map(lagOf);
    const p50 = rank(lags, 0.5), p99 = rank(lags, 0.99), most = worstOf(judged, lagOf);
    const worstLag = most ? lagOf(most) : null;
    const pass = !missing.length && p99 <= MAP.lagP99Steps * step && worstLag <= MAP.lagWorstSteps * step;
    rows.push(row({ page, run: label, bar: `content lag, ${side}`, measured: most ? `p50 ${fixed(p50, 0)} / p99 ${fixed(p99, 0)} / worst ${fixed(worstLag, 0)} h` : `none of ${side} was drawn`,
      limit: `p99 ${MAP.lagP99Steps * step} / worst ${MAP.lagWorstSteps * step} h (one and two steps)`, verdict: pass ? PASS : FAIL, worst: at(most), today: TODAY.lag,
      note: [firstNote, outside.size ? `outside the ${report.layers.seeks.length} seek windows` : '', missing.length ? `${missing.length} frames show nothing of ${side} (first ${missing[0].frame})` : '']
        .filter(Boolean).join('; '), values: { p50, p99, worst: worstLag } }));
  };
  if (layered) lagRow('the layers', frame => frame.layers?.lag_hours ?? null);
  if (todayOn) lagRow('today\'s parts', frame => (frame.drawn_hour == null ? null : frame.lag_hours));

  const seconds = paced.seconds - list[first].start_us / 1e6;
  const pictures = (lagSide, changed) => {
    let count = 0;
    for (let index = 0; index < measured.length; index++) if (changed(measured[index], list[first + index - 1])) count++;
    rows.push(row({ page, run: label, bar: `new pictures a second, ${lagSide}`, measured: `${fixed(count / seconds, 1)} (${count} in ${fixed(seconds, 2)} s)`, limit: `one a frame (${hz})`,
      verdict: REPORTED, today: TODAY.pictures, values: { perSecond: count / seconds } }));
  };
  if (layered) pictures('the layers', (frame, before) => frame.layers?.drawn !== before?.layers?.drawn);
  if (todayOn) pictures('today\'s parts', frame => frame.new_hour);
  const layers = report.layers?.layers ?? [];
  const cpuOf = (frame, index) => {
    const parts = frame.layers?.cpu_ms?.[index];
    return Array.isArray(parts) ? parts.reduce((sum, part) => (part == null || sum == null ? null : sum + part), 0) : null;
  };
  layers.forEach((layer, index) => {
    const name = layer.name ?? `layer ${index + 1}`;
    const bar = LAYER_BARS[name];
    const cpu = layer.cpu_ms ?? {}, wall = layer.wall_ms ?? {};
    const parts = SCOPES.filter(scope => layer[scope]).map(scope => `${scope} ${fixed(layer[scope].cpu_ms?.p99, 2)}`).join(', ');
    const most = worstOf(measured, frame => cpuOf(frame, index));
    const judged = bar && Number.isFinite(cpu.p99) && Number.isFinite(cpu.max);
    rows.push(row({ page, run: label, bar: `layer ${name}: apply, settle and upload`, measured: `own CPU p50 ${fixed(cpu.p50, 2)} / p99 ${fixed(cpu.p99, 2)} / worst ${fixed(cpu.max, 2)} ms (wall p99 ${fixed(wall.p99, 2)} / worst ${fixed(wall.max, 2)})${parts ? `; p99 ${parts}` : ''}`,
      limit: bar ? `p99 ${bar.p99}${bar.worst ? ` / worst ${bar.worst}` : ''} ms of its own CPU (${bar.from})` : 'no brief states its bar',
      verdict: !bar ? REPORTED : !judged ? UNMEASURABLE : cpu.p99 <= bar.p99 && (!bar.worst || cpu.max <= bar.worst) ? PASS : FAIL, worst: at(most),
      note: `${layer.frames_upload_carried ?? 0} frames carried upload; carried p99 ${fixed(layer.carried?.p99, 0)}`, values: { p50: cpu.p50, p99: cpu.p99, worst: cpu.max, wallP99: wall.p99 } }));
  });
  const vision = layers.findIndex(layer => layer.name === 'vision');
  if (vision >= 0) {
    const less = measured.map(frame => ({ frame, us: frame.thread_us - (cpuOf(frame, vision) ?? 0) * 1000 }));
    const p99 = ms(rank(less.map(entry => entry.us), 0.99)), most = worstOf(less, entry => entry.us);
    rows.push(row({ page, run: label, bar: 'window-thread CPU a frame, less the vision layer', measured: `p99 ${fixed(p99, 2)} / worst ${fixed(ms(most.us), 2)} ms`,
      limit: `p99 ${MAP.threadP99Ms} / worst ${MAP.threadWorstMs} ms`, verdict: REPORTED, worst: at(most.frame), today: TODAY.thread,
      note: 'the frame without vision\'s apply, settle and upload, whose expensive part is moving to a worker: what the CPU row becomes when it lands', values: { p99, worst: ms(most.us) } }));
  }

  const stepper = report.stepper;
  const stepMs = report.layers?.step_ms;
  if (stepper && stepMs) {
    // A frame's step_ms is the build time of every step it took; a step's time is its share.
    const perStep = measured.filter(frame => frame.layers?.steps > 0).map(frame => ({ frame, ms: frame.layers.step_ms / frame.layers.steps }));
    const p99 = rank(perStep.map(entry => entry.ms), 0.99), most = worstOf(perStep, entry => entry.ms);
    rows.push(row({ page, run: label, bar: 'playhead step time', measured: `p50 ${fixed(rank(perStep.map(entry => entry.ms), 0.5), 2)} / p99 ${fixed(p99, 2)} / worst ${fixed(most?.ms, 2)} ms a step over ${perStep.length} frames that stepped`,
      limit: 'p99 5 / worst 10 ms, at most three threads', verdict: most && p99 <= 5 && most.ms <= 10 ? PASS : FAIL, worst: at(most?.frame), today: TODAY.step,
      note: `${stepper.steps} steps, ${stepper.catch_ups ?? 0} catch-ups, ${stepper.split_steps ?? 0} split; the threads it ran on are not reported`, values: { p99, worst: most?.ms ?? null } }));
    rows.push(row({ page, run: label, bar: 'playhead stalls', measured: `${stepper.stalls} stalls, ${stepper.stalled_steps} steps holding one, ${stepper.piece_waits ?? 0} piece waits`,
      limit: 'none', verdict: stepper.stalls === 0 && stepper.stalled_steps === 0 ? PASS : FAIL, today: TODAY.step, values: { stalls: stepper.stalls } }));
    const bytes = stepper.playhead_bytes ?? {};
    rows.push(row({ page, run: label, bar: 'playhead held()', measured: `peak ${fixed(bytes.peak / 1e6, 1)} MB (ring ${fixed(bytes.ring_peak / 1e6, 1)}, states ${fixed(bytes.states_peak / 1e6, 1)}); the map's side ${fixed(stepper.host_bytes?.total / 1e6, 1)} MB at the end`,
      limit: `at most ${MAP.heldPeakBytes / 1e6} MB (section 6)`, verdict: bytes.peak <= MAP.heldPeakBytes ? PASS : FAIL, today: TODAY.memory, values: { peakMb: bytes.peak / 1e6 } }));
  } else if (layered) {
    rows.push(row({ page, run: label, bar: 'playhead step time', measured: '–', limit: 'p99 5 / worst 10 ms, at most three threads; no stall', verdict: UNMEASURABLE,
      today: TODAY.step, note: 'this viewer\'s report carries no stepper section' }));
  }

  const memory = report.memory ?? {};
  rows.push(row({ page, run: label, bar: 'process peak memory', measured: `working set ${fixed(memory.working_set_peak_bytes / 2 ** 20, 0)} MB, private ${fixed(memory.private_peak_bytes / 2 ** 20, 0)} MB`,
    limit: 'reported beside the playhead\'s held()', verdict: REPORTED, today: TODAY.memory, note: memory.scope ?? '',
    values: { workingSetMb: memory.working_set_peak_bytes / 2 ** 20 } }));

  if (num(run.args, '--seeks')) {
    const seeks = (report.layers?.seeks ?? []).map(seek => {
      const shown = list[seek.frame + seek.frames_to_shown];
      const between = list.slice(seek.frame, seek.frame + seek.frames_to_shown + 1);
      return { seek, ms: shown ? ms(presented(shown) - list[seek.frame].start_us) : null, over: between.filter(frame => frame.wall_us > MAP.intervalWorst * interval) };
    });
    const times = seeks.map(entry => entry.ms);
    const median = rank(times, 0.5), slowest = worstOf(seeks, entry => entry.ms);
    const over = seeks.flatMap(entry => entry.over);
    const pass = seeks.every(entry => entry.ms !== null) && median <= MAP.seekMedianMs && slowest.ms <= MAP.seekWorstMs && !over.length;
    rows.push(row({ page, run: label, bar: 'seek: sought hour on screen', measured: `median ${fixed(median, 0)} / worst ${fixed(slowest?.ms, 0)} ms over ${seeks.length}; ${over.length} frames over ${MAP.intervalWorst} intervals meanwhile`,
      limit: `median ${MAP.seekMedianMs} / worst ${MAP.seekWorstMs} ms; no frame over budget`, verdict: pass ? PASS : FAIL,
      worst: slowest ? { frame: slowest.seek.frame, hour: slowest.seek.hour } : null, today: TODAY.seek,
      note: `from the seek's frame to the present of the frame showing it${over.length ? `; over budget: frames ${over.slice(0, 5).map(frame => frame.frame).join(', ')}` : ''}`,
      values: { median, worst: slowest?.ms ?? null } }));
  }
  return rows;
}

/** The floor of a machine: paced frames of the same view at a held hour, the camera panning. A frame is late as in play:
 * presented over `intervalWorst` refresh intervals after the frame before. */
export function floorOf({ run, report, frames, acceptReconciliation = false }) {
  const whole = wholeHarness(run, report, frames, { acceptReconciliation });
  if (!whole.whole) return { whole: false, reasons: whole.reasons };
  const interval = 1e6 / (report.paced?.refresh_hz ?? num(run.args, '--refresh-hz'));
  const list = frames.frames;
  const intervals = list.slice(1).map((frame, index) => ({ frame, us: presented(frame) - presented(list[index]) }));
  const late = intervals.filter(entry => entry.us > MAP.intervalWorst * interval).length;
  return { whole: true, accepted: whole.accepted, frames: intervals.length, late, rate: intervals.length ? late / intervals.length : null, worst: worstOf(intervals, entry => entry.us)?.frame };
}

export function floorRows({ page = 'map', label, floor, run }) {
  if (!floor.whole) return partialRows(page, label, ['the floor: late frames paused, the camera panning'], floor.reasons);
  return [row({ page, run: label, bar: 'the floor: late frames paused, the camera panning', measured: `${floor.late} of ${floor.frames} (${fixed(floor.rate * 1000, 2)} per 1,000)`,
    limit: 'the machine\'s own floor for the late-frame bar', verdict: REPORTED, worst: floor.worst ? { frame: floor.worst.frame, hour: floor.worst.head_hour } : null,
    note: `${floor.accepted ? `${floor.accepted}; ` : ''}paced at the held hour, ${arg(run.args, '--pan')} map units a frame`, values: { per1000: floor.rate * 1000 } })];
}

/** The rows of one proof-tool run (`--timing`, with seeks): stalls, the playhead's held bytes, and seeks composed. */
export function proofRows({ page = 'map', label, run, summary, frames }) {
  const whole = wholeProof(run, summary, frames);
  const bars = ['playhead stalls', 'playhead held()', 'seek: sought hour composed (proof, flat play)'];
  if (!whole.whole) return partialRows(page, label, bars, whole.reasons);
  const rows = [];
  rows.push(row({ page, run: label, bar: 'playhead stalls', measured: `${summary.stalls} over ${summary.steps} steps`, limit: 'none', verdict: summary.stalls === 0 ? PASS : FAIL,
    today: TODAY.step, values: { stalls: summary.stalls } }));
  const series = summary.held_series ?? [];
  const peakAt = worstOf(series, entry => entry.playhead);
  const half = series.slice(0, Math.max(1, series.length >> 1)), later = series.slice(series.length >> 1);
  const grew = later.length && half.length ? Math.max(...later.map(entry => entry.playhead)) / Math.max(...half.map(entry => entry.playhead)) : null;
  rows.push(row({ page, run: label, bar: 'playhead held()', measured: `peak ${fixed(summary.playhead_held_peak / 1e6, 1)} MB; second half's peak ${fixed(grew, 2)}× the first's`,
    limit: `at most ${MAP.heldPeakBytes / 1e6} MB (section 6)`, verdict: summary.playhead_held_peak <= MAP.heldPeakBytes ? PASS : FAIL,
    worst: peakAt ? { frame: null, hour: peakAt.hour } : null, today: TODAY.memory, values: { peakMb: summary.playhead_held_peak / 1e6 } }));
  const shown = (summary.seeks ?? []).map(seek => seek.shown_ms);
  const slowest = worstOf(summary.seeks ?? [], seek => seek.shown_ms);
  rows.push(row({ page, run: label, bar: 'seek: sought hour composed (proof, flat play)', measured: `median ${fixed(rank(shown, 0.5), 0)} / worst ${fixed(slowest?.shown_ms, 0)} ms over ${shown.length}`,
    limit: `median ${MAP.seekMedianMs} / worst ${MAP.seekWorstMs} ms (judged on the paced run)`, verdict: REPORTED, worst: slowest ? { frame: null, hour: slowest.target } : null,
    today: TODAY.seek, note: 'the proof plays flat, without the window: beside the paced seek row', values: { median: rank(shown, 0.5), worst: slowest?.shown_ms ?? null } }));
  return rows;
}

/** The figure spread of repeated runs: per run (its role) and bar, each value's least and greatest over the whole repeats. */
export function spread(rows) {
  const groups = new Map();
  for (const entry of rows.filter(entry => entry.verdict !== PARTIAL && entry.repeat != null)) {
    const key = `${entry.page}\u0000${entry.role ?? entry.run}\u0000${entry.bar}`;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(entry);
  }
  return [...groups].filter(([, list]) => list.length > 1).map(([key, list]) => {
    const [page, run, bar] = key.split('\u0000');
    const values = {};
    for (const name of Object.keys(list[0].values)) {
      const seen = list.map(entry => entry.values[name]).filter(Number.isFinite);
      if (seen.length) values[name] = [Math.min(...seen), Math.max(...seen)];
    }
    return { page, run, bar, runs: list.length, values, verdicts: list.map(entry => entry.verdict) };
  });
}

// ---- The dashboard: a perf log of the desktop app.

/** The anchors of a log: ticks where the clock started or stopped playing, changed speed, or sought. */
export function anchors(ticks) {
  const found = [];
  for (let index = 0; index < ticks.length; index++) {
    const tick = ticks[index], before = ticks[index - 1];
    if (!before || tick.playing !== before.playing || tick.speed !== before.speed || tick.seek !== before.seek) found.push(tick.at);
  }
  return found;
}

/** The latest of `sorted` (by `at`) at or before `time`. */
function latest(sorted, time) {
  let low = 0, high = sorted.length - 1, found = null;
  while (low <= high) {
    const mid = (low + high) >> 1;
    if (sorted[mid].at <= time) { found = sorted[mid]; low = mid + 1; } else high = mid - 1;
  }
  return found;
}

/** Whether a perf log is whole: its last line parsed, nothing lost, playing frames, and what the run was asked to do. */
export function wholeLog(log, records, { playingSeconds = null, seeks = null } = {}) {
  const reasons = [];
  if (log.cut) reasons.push('the log ends in a cut line');
  const lost = records.filter(record => record.t === 'lost').reduce((sum, record) => sum + (record.count ?? 0), 0);
  if (lost) reasons.push(`${lost} records lost`);
  const ticks = records.filter(record => record.t === 'tick').sort((a, b) => a.at - b.at);
  const playing = stretches(ticks);
  const played = playing.reduce((sum, stretch) => sum + (stretch.to - stretch.from) / 1000, 0);
  if (!playing.length) reasons.push('the clock never played');
  if (!records.some(record => record.t === 'frame' && record.playing)) reasons.push('no playing frames');
  if (playingSeconds !== null && played < playingSeconds) reasons.push(`played ${played.toFixed(1)} s, asked ${playingSeconds}`);
  if (seeks !== null) {
    const made = new Set(ticks.map(tick => tick.seek)).size - 1;
    if (made < seeks) reasons.push(`${made} of ${seeks} seeks`);
  }
  return { whole: reasons.length === 0, reasons, played };
}

/** The clock's playing stretches, [from, to] in epoch ms, with the speed it played at. */
export function stretches(ticks) {
  const found = [];
  let open = null;
  for (const tick of ticks) {
    if (tick.playing && !open) open = { from: tick.at, to: tick.at, speed: tick.speed };
    else if (tick.playing) open.to = tick.at;
    else if (open) { open.to = tick.at; found.push(open); open = null; }
  }
  if (open) found.push(open);
  return found;
}

/** Per page and widget, the dashboard's bars over the playing stretches of a perf log. `refreshHz` sets the interval; when
 * it is not given the median playing frame interval stands in, and the note says so. */
export function dashboardRows(log, { page = 'page', refreshHz = null, expect = {}, window = null } = {}) {
  // A window (epoch ms) keeps the timed records inside it; what a record states for the session (widgets, follows, losses) stays.
  const records = window ? log.records.filter(record => !TIMED.has(record.t) || (record.at >= window.from && record.at <= window.to)) : log.records;
  const whole = wholeLog(log, records, expect);
  const bars = ['webview frame interval', 'long animation frames', 'React commits caused by a value', 'requests for values at the head', 'list requests ahead of the head',
    'chart tile requests', 'a followed value\'s age'];
  const ticks = records.filter(record => record.t === 'tick').sort((a, b) => a.at - b.at);
  const titles = new Map(records.filter(record => record.t === 'widget').map(record => [record.id, record.title]));
  const follow = followAccount(records, titles);
  if (!whole.whole) return { whole, rows: partialRows(page, 'perf log', bars, whole.reasons), follow };
  const playing = stretches(ticks);
  const marks = anchors(ticks);
  const inStretch = time => playing.some(stretch => time >= stretch.from && time <= stretch.to);
  const settling = time => marks.some(mark => time >= mark && time < mark + ANCHOR_SETTLE_MS);
  const frames = records.filter(record => record.t === 'frame').sort((a, b) => a.at - b.at).map((record, index) => ({ ...record, index }));
  const played = frames.filter(frame => frame.playing);
  const median = rank(played.map(frame => frame.dt), 0.5);
  const hz = refreshHz ?? 1000 / median;
  const interval = 1000 / hz;
  const hourAt = time => latest(ticks, time)?.hour ?? null;
  const at = frame => (frame ? { frame: frame.index, hour: hourAt(frame.at) } : null);
  const rows = [];
  const hzNote = refreshHz ? `${hz} Hz` : `${fixed(hz, 1)} Hz taken from the median playing frame interval (no refresh rate given)`;

  const p99 = rank(played.map(frame => frame.dt), 0.99) / interval;
  const longest = worstOf(played, frame => frame.dt);
  const over = played.filter(frame => frame.dt > DASHBOARD.intervalWorst * interval);
  rows.push(row({ page, run: 'perf log', bar: 'webview frame interval', measured: `p99 ${fixed(p99, 2)} / worst ${fixed(longest.dt / interval, 2)} intervals over ${played.length} frames; ${over.length} over ${DASHBOARD.intervalWorst}`,
    limit: `p99 ${DASHBOARD.intervalP99} / none over ${DASHBOARD.intervalWorst} intervals (${fixed(interval, 2)} ms)`, verdict: p99 <= DASHBOARD.intervalP99 && !over.length ? PASS : FAIL,
    worst: at(longest), today: TODAY.webview, note: `${hzNote}${over.length ? `; over: frames ${over.slice(0, 5).map(frame => frame.index).join(', ')}` : ''}`,
    values: { p99, worst: longest.dt / interval, over: over.length } }));

  const page0 = records.find(record => record.t === 'page');
  const loafs = records.filter(record => record.t === 'loaf' && inStretch(record.at));
  const longestLoaf = worstOf(loafs, record => record.dur);
  rows.push(row({ page, run: 'perf log', bar: 'long animation frames', measured: page0?.loaf === false ? '–' : `${loafs.length} while playing${longestLoaf ? `, longest ${fixed(longestLoaf.dur, 0)} ms (${longestLoaf.top ?? 'no script named'})` : ''}`,
    limit: 'none', verdict: page0?.loaf === false ? UNMEASURABLE : loafs.length ? FAIL : PASS,
    worst: longestLoaf ? { frame: null, hour: hourAt(longestLoaf.at) } : null, today: TODAY.webview,
    note: page0?.loaf === false ? 'this webview does not report long animation frames' : '', values: { count: loafs.length } }));

  // Commits by the part of the page that rendered, in playing frames past an anchor's settling.
  const judged = played.filter(frame => !settling(frame.at));
  const commits = new Map();
  for (const frame of judged) {
    const zones = Object.entries(frame.zones ?? {});
    const attributed = zones.reduce((sum, [, count]) => sum + count, 0);
    if (frame.commits > 0 && !attributed) zones.push(['unattributed', frame.commits]);
    for (const [zone, count] of zones) {
      const entry = commits.get(zone) ?? { count: 0, frames: [] };
      entry.count += count; entry.frames.push(frame);
      commits.set(zone, entry);
    }
  }
  const zoneName = zone => (zone.startsWith('widget:') ? titles.get(zone.slice(7)) ?? zone : zone);
  rows.push(row({ page, run: 'perf log', bar: 'React commits caused by a value', measured: commits.size ? [...commits].map(([zone, entry]) => `${zoneName(zone)} ${entry.count}`).join(', ') : `none in ${judged.length} frames`,
    limit: 'none while playing', verdict: commits.size ? FAIL : PASS, worst: at([...commits.values()][0]?.frames[0]), today: TODAY.commits,
    note: `play, pause, speed and seek own the ${ANCHOR_SETTLE_MS} ms after them`, values: { count: [...commits.values()].reduce((sum, entry) => sum + entry.count, 0) } }));
  for (const [zone, entry] of commits) rows.push(row({ page: `${page}: ${zoneName(zone)}`, run: 'perf log', bar: 'React commits caused by a value', measured: `${entry.count} in ${entry.frames.length} frames`,
    limit: 'none while playing', verdict: FAIL, worst: at(entry.frames[0]), today: TODAY.commits, values: { count: entry.count } }));

  const requests = records.filter(record => record.t === 'q' && !record.cache && record.cmd !== undefined && !FOLLOW_PRODUCTS.has(record.product)
    && inStretch(record.at) && !settling(record.at));
  const lists = requests.filter(record => record.product !== 'series-tiles');
  const ahead = lists.filter(aheadOfHead);
  const values = lists.filter(record => !aheadOfHead(record));
  const byProduct = new Map();
  for (const record of values) byProduct.set(record.product, (byProduct.get(record.product) ?? 0) + 1);
  const reads = new Map();
  for (const frame of judged) for (const [widget, work] of Object.entries(frame.work ?? {})) if (work.read) reads.set(widget, (reads.get(widget) ?? 0) + work.read);
  rows.push(row({ page, run: 'perf log', bar: 'requests for values at the head', measured: values.length ? [...byProduct].map(([product, count]) => `${product} ${count}`).join(', ') : 'none',
    limit: 'none while playing', verdict: values.length ? FAIL : PASS, worst: values.length ? { frame: null, hour: hourAt(values[0].at) } : null, today: TODAY.requests,
    note: reads.size ? `widgets that asked: ${[...reads].map(([widget, count]) => `${titles.get(widget) ?? widget} ${count}`).join(', ')}` : '', values: { count: values.length } }));
  for (const [widget, count] of reads) rows.push(row({ page: `${page}: ${titles.get(widget) ?? widget}`, run: 'perf log', bar: 'requests for values at the head', measured: `${count} reads`,
    limit: 'none while playing', verdict: FAIL, today: TODAY.requests, values: { count } }));

  rows.push(aheadRow(page, ahead, playing, ticks, hourAt));
  rows.push(tileRow(page, requests.filter(record => record.product === 'series-tiles'), playing, ticks, hourAt));
  rows.push(ageRow(page, records, ticks, playing, settling, follow, titles));
  return { whole, rows, follow };
}

/** A request for a block wholly ahead of the head: the first hour it asks from is past the head it was sent at. */
export const aheadOfHead = record => typeof record.from === 'number' && typeof record.head === 'number' && record.from > record.head;

/** A list's requests for blocks ahead of the head while playing: per list (`lane`), each block (the hour it asks from)
 * once, and no more than the blocks the head crossed and the two ahead of it, as chart tiles are judged. */
function aheadRow(page, ahead, playing, ticks, hourAt) {
  const base = { page, run: 'perf log', bar: 'list requests ahead of the head', limit: 'each block once, at most the blocks the head crossed and two ahead', today: TODAY.requests };
  const reasons = [];
  let worst = null;
  for (const stretch of playing) {
    const end = latest(ticks, stretch.to)?.hour ?? -Infinity;
    const lanes = new Map();
    for (const record of ahead.filter(record => record.at >= stretch.from && record.at <= stretch.to)) {
      const lane = record.lane ?? record.product;
      if (!lanes.has(lane)) lanes.set(lane, []);
      lanes.get(lane).push(record);
    }
    for (const list of lanes.values()) {
      const blocks = new Set(list.map(record => record.from));
      const crossed = [...blocks].filter(from => from <= end).length;
      const twice = list.length - blocks.size;
      if (list.length > crossed + 2 || twice) {
        reasons.push(`${list[0].product}: ${list.length} asked, ${crossed + 2} allowed (${crossed} crossed)${twice ? `, ${twice} asked twice` : ''}`);
        worst ??= list.at(-1);
      }
    }
  }
  return row({ ...base, measured: `${ahead.length} asked while playing`, verdict: reasons.length ? FAIL : PASS, worst: worst ? { frame: null, hour: hourAt(worst.at) } : null,
    note: reasons.slice(0, 4).join('; '), values: { count: ahead.length } });
}

/** Chart tiles asked while playing: per chart (its branch, family and tag group) and bin size, at most the tiles the head
 * crossed and the one ahead; a tile asked twice always fails. */
function tileRow(page, tiles, playing, ticks, hourAt) {
  const base = { page, run: 'perf log', bar: 'chart tile requests', limit: `one per chart per ${DASHBOARD.tileBins} bins of the head's travel`, today: TODAY.requests };
  if (tiles.some(record => !record.tile)) return row({ ...base, measured: `${tiles.length} asked`, verdict: UNMEASURABLE, note: 'the perf log names no tile (bins, index, chart) on its requests' });
  const reasons = [];
  let worst = null;
  for (const stretch of playing) {
    const within = tiles.filter(record => record.at >= stretch.from && record.at <= stretch.to);
    const travel = Math.abs((latest(ticks, stretch.to)?.hour ?? 0) - (latest(ticks, stretch.from)?.hour ?? 0));
    const charts = new Map();
    for (const record of within) {
      const key = `${record.tile.key}\u0000${record.tile.bins}`;
      if (!charts.has(key)) charts.set(key, []);
      charts.get(key).push(record);
    }
    for (const list of charts.values()) {
      const { bins } = list[0].tile;
      const allowed = Math.floor(travel / (bins * DASHBOARD.tileBins)) + 2;
      const twice = list.length - new Set(list.map(record => record.tile.index)).size;
      if (list.length > allowed || twice) { reasons.push(`${list.length} tiles of ${bins} h bins, ${allowed} allowed over ${travel} h${twice ? `, ${twice} asked twice` : ''}`); worst ??= list.at(-1); }
    }
  }
  return row({ ...base, measured: `${tiles.length} asked while playing`, verdict: reasons.length ? FAIL : PASS, worst: worst ? { frame: null, hour: hourAt(worst.at) } : null,
    note: reasons.slice(0, 4).join('; '), values: { count: tiles.length } });
}

/** Which followed queries each widget holds, which fell back and why, from the store's follow records. */
export function followAccount(records, titles) {
  const queries = new Map();
  for (const record of records.filter(record => record.t === 'follow')) {
    const entry = queries.get(record.id) ?? { id: record.id, product: record.product ?? null, status: null, reason: null, widgets: new Set() };
    if (record.product) entry.product = record.product;
    if (record.status !== 'released') { entry.status = record.status; entry.reason = record.reason ?? null; }
    queries.set(record.id, entry);
  }
  for (const record of records.filter(record => record.t === 'follow_widget')) {
    const entry = queries.get(record.id) ?? { id: record.id, product: null, status: 'pending', reason: null, widgets: new Set() };
    entry.widgets.add(record.widget);
    queries.set(record.id, entry);
  }
  const widgets = new Map();
  for (const entry of queries.values()) for (const widget of entry.widgets) {
    const held = widgets.get(widget) ?? { widget, title: titles.get(widget) ?? widget, followed: [], fellBack: [] };
    (entry.status === 'followed' ? held.followed : held.fellBack).push(entry.status === 'followed' ? entry.product : `${entry.product ?? 'a query'}: ${entry.reason ?? entry.status ?? 'never answered'}`);
    widgets.set(widget, held);
  }
  for (const [widget, title] of titles) if (!widgets.has(widget)) widgets.set(widget, { widget, title, followed: [], fellBack: [], none: true });
  return { queries: [...queries.values()], widgets: [...widgets.values()] };
}

/** Panels whose reply is structure alone (which rows there are, their order, names and art), every number they show a
 * live cell: their pushes are exempt from the age bar by name and judged instead at most four in any second of play,
 * and each of their cells is judged by its age. A panel not named here is judged as a value, so a panel whose numbers
 * are not live cells fails the bar. Name a product only once its reply carries no number that moves with the head. */
export const STRUCTURE_PANELS = new Set(['territory-breakdown', 'world-tension', 'occupied-states']);
/** The most pushes of a structure panel in any second of play. */
export const STRUCTURE_PUSHES_A_SECOND = 4;
/** The bit of every follow id a panel's host takes for a group of its live cells. */
const CELLS = 2 ** 31;

/** A followed value's age when it is painted: the steps the head took from the one its answer was evaluated at (the
 * last tick in that hour) to the first frame after its message landed (the frame the store paints it in). A step is a
 * tick of the clock, however far a late frame moved it. The service sends nothing for a step that changed no answer, so a
 * value not resent is current, not old. A panel's live cells are judged as its own values (`follow_cells` names whose
 * they are); a structure panel's own pushes are not values ([`STRUCTURE_PANELS`]). The longest wait between messages
 * while playing is beside it. */
function ageRow(page, records, ticks, playing, settling, follow, titles) {
  const base = { page, run: 'perf log', bar: "a followed value's age", limit: `at most ${DASHBOARD.ageWorstSteps} steps; a structure panel's pushes at most ${STRUCTURE_PUSHES_A_SECOND} a second`, today: TODAY.age };
  const followed = new Map(follow.queries.filter(entry => entry.status === 'followed').map(entry => [entry.id, entry]));
  if (!records.some(record => record.t === 'follow')) return row({ ...base, measured: '–', verdict: UNMEASURABLE, note: 'the perf log has no follow records' });
  if (!followed.size) return row({ ...base, measured: '–', verdict: UNMEASURABLE, note: 'nothing was followed' });
  const panelOf = new Map();
  for (const record of records.filter(record => record.t === 'follow_cells')) for (const id of record.cells ?? []) panelOf.set(id, record.id);
  const queryOf = id => followed.get(id >= CELLS ? panelOf.get(id) : id);
  const structure = id => id < CELLS && STRUCTURE_PANELS.has(followed.get(id)?.product);
  const messages = records.filter(record => record.t === 'follow_message').sort((a, b) => a.at - b.at);
  const ages = [];
  const pushes = new Map();
  let gap = null;
  for (const stretch of playing) {
    const steps = ticks.filter(tick => tick.playing && tick.at >= stretch.from && tick.at <= stretch.to);
    const within = messages.filter(message => message.at >= stretch.from && message.at <= stretch.to);
    for (let index = 1; index < within.length; index++) gap = Math.max(gap ?? 0, within[index].at - within[index - 1].at);
    for (const message of within) {
      if (settling(message.at)) continue;
      const painted = steps.findIndex(tick => tick.at > message.at);
      if (painted < 0) continue;
      for (const [id, hour] of message.answers ?? []) {
        if (!queryOf(id)) continue;
        if (structure(id)) {
          if (!pushes.has(id)) pushes.set(id, []);
          pushes.get(id).push(message.at);
          continue;
        }
        let evaluated = painted;
        while (evaluated > 0 && Math.floor(steps[evaluated].hour) > hour) evaluated--;
        ages.push({ steps: painted - evaluated, hours: steps[painted].hour - hour, id, tick: steps[painted] });
      }
    }
  }
  const holders = query => [...query.widgets].map(widget => titles.get(widget) ?? widget).join(', ') || 'no widget named';
  const crowded = [];
  for (const [id, times] of pushes) {
    const at = times.findIndex((time, index) => index + STRUCTURE_PUSHES_A_SECOND < times.length && times[index + STRUCTURE_PUSHES_A_SECOND] - time < 1000);
    if (at >= 0) crowded.push({ id, note: `${followed.get(id).product} (${holders(followed.get(id))}) pushed over ${STRUCTURE_PUSHES_A_SECOND} times in a second` });
  }
  const exempt = [...pushes].map(([id, times]) => `${followed.get(id).product}'s structure (${times.length} pushes)`);
  const cells = ages.filter(age => age.id >= CELLS).length;
  const notes = [...crowded.map(entry => entry.note), ...(exempt.length ? [`exempt by name: ${exempt.join(', ')}`] : [])];
  if (!ages.length) return row({ ...base, measured: '–', verdict: FAIL, note: ['no followed answer landed while playing', ...notes].join('; ') });
  const worst = worstOf(ages, age => age.steps);
  const p99 = rank(ages.map(age => age.steps), 0.99);
  const query = queryOf(worst.id);
  const named = `${query.product ?? 'a query'}${worst.id >= CELLS ? "'s live cells" : ''}`;
  return row({ ...base, measured: `p99 ${fixed(p99, 0)} / worst ${worst.steps} steps (${fixed(worst.hours, 0)} h) over ${ages.length} answers, ${cells} of live cells`,
    verdict: worst.steps <= DASHBOARD.ageWorstSteps && !crowded.length ? PASS : FAIL, worst: { frame: null, hour: worst.tick.hour },
    note: [`worst: ${named} (${holders(query)}); longest wait between messages while playing ${fixed(gap, 0)} ms`, ...notes].join('; '),
    values: { p99, worst: worst.steps, cells, crowded: crowded.length } });
}

// ---- The table.

const cell = value => String(value ?? '').replace(/\|/g, '\\|').replace(/\n/g, ' ');

/** One verdict table: page, run, bar, measured, the bar, the verdict, the worst frame and hour, today. */
export function table(rows) {
  const lines = ['| Page | Run | Bar | Measured | The bar | Verdict | Worst frame (hour) | Today (P0) | Note |', '|---|---|---|---|---|---|---|---|---|'];
  for (const entry of rows) {
    const worst = entry.worst ? `${entry.worst.frame ?? '–'} (${entry.worst.hour == null ? '–' : Math.floor(entry.worst.hour)})` : '';
    const verdict = entry.verdict === FAIL || entry.verdict === PARTIAL ? `**${entry.verdict}**` : entry.verdict;
    lines.push(`| ${[entry.page, entry.run, entry.bar, entry.measured, entry.limit, verdict, worst, entry.today, entry.note].map(cell).join(' | ')} |`);
  }
  return lines.join('\n');
}

/** Whether a set of rows passes: no fail and no partial. Not measurable and reported rows do not decide it. */
export const passed = rows => !rows.some(entry => entry.verdict === FAIL || entry.verdict === PARTIAL);
