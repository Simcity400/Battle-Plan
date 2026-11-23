
export const PASS = 'pass', FAIL = 'fail', PARTIAL = 'partial', UNMEASURABLE = 'not measurable yet', REPORTED = 'reported';

export const MAP = { threadP99Ms: 6, threadWorstMs: 10, intervalP99: 1.1, intervalWorst: 1.5, lagP99Steps: 1, lagWorstSteps: 2,
  seekWorstMs: 150, seekMedianMs: 100, heldPeakBytes: 220e6 };
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
export const PAGING_FREE_MB = 2048;

function machineReasons(run) {
  const least = run.memory?.least_free_mb;
  return Number.isFinite(least) && least < PAGING_FREE_MB ? [`the machine was paging: ${least} MB of memory free at the least, under ${PAGING_FREE_MB}`] : [];
}
export const DASHBOARD = { intervalP99: 1.1, intervalWorst: 2, tileBins: 256 };
export const ANCHOR_SETTLE_MS = 300;
const TIMED = new Set(['tick', 'frame', 'q', 'loaf', 'longtask', 'shown', 'picture']);
export const FOLLOW_PRODUCTS = new Set(['follow', 'unfollow', 'follow-clock', 'follow-close']);

export const TODAY = {
  thread: '9.0 median, 14.6 worst when an hour installs',
  interval: 'not measured',
  lag: '770–840 / 1,200–1,224 / 1,294–1,440 h (p50 / p99 / worst)',
  pictures: '4.5–4.7 a second',
  step: '7.5 ms median, 128 worst (move and frame of rows)',
  seek: '102–313 ms before the map\'s 0.3 s',
  memory: 'cursor 1,228 MB; process 4.7 GB',
  webview: 'not measured',
  requests: '26 a second for seven widgets',
};

export function rank(values, p) {
  const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
  return sorted.length ? sorted[Math.min(sorted.length, Math.max(1, Math.ceil(p * sorted.length))) - 1] : null;
}

export function arg(args, flag) {
  const at = (args ?? []).indexOf(flag);
  return at < 0 ? null : args[at + 1];
}

const num = (args, flag) => { const value = arg(args, flag); return value === null ? null : Number(value); };
const ms = us => us / 1000;
const fixed = (value, digits = 1) => (Number.isFinite(value) ? value.toFixed(digits) : '–');

function worstOf(items, of) {
  let best = null;
  for (const item of items) if (Number.isFinite(of(item)) && (best === null || of(item) > of(best))) best = item;
  return best;
}

export function wholeHarness(run, report, frames, { acceptReconciliation = false } = {}) {
  const reasons = machineReasons(run);
  if (run.stopped) reasons.push(`stopped by the runner: ${run.stopped}`);
  const check = report?.cpu_reconciliation;
  const reconciliation = run.exit === 1 && check?.passed === false;
  const off = reconciliation ? `${(100 * check.process_minus_scopes_ns / check.process_cpu_ns).toFixed(1)}% on ${(check.process_cpu_ns / 1e9).toFixed(1)} s of CPU` : '';
  if (run.exit !== 0 && !(acceptReconciliation && reconciliation)) reasons.push(reconciliation ? `exit 1: the harness's CPU reconciliation failed (process CPU off its scopes by ${off})` : `exit ${run.exit ?? 'not recorded'}`);
  const from = num(run.args, '--from-hour'), hours = num(run.args, '--hours'), back = num(run.args, '--seek-back');
  const held = (run.args ?? []).includes('--paused');
  if (!report) reasons.push('no report.json');
  if (!frames) reasons.push('no frames.jsonl');
  else if (frames.cut) reasons.push('frames.jsonl ends in a cut line');
  if (!report && frames?.frames.length) {
    const last = frames.frames.at(-1);
    reasons.push(held ? `covered ${frames.frames.length} of ${hours} frames` : `covered ${frames.frames.length} frames, the head to ${Math.floor(last.head_hour)} of ${from + hours}`);
  }
  if (!report || !frames) return { whole: false, reasons };
  const list = frames.frames;
  if (report.from_hour !== from || report.game_hours !== hours) reasons.push(`the report covers ${report.from_hour} + ${report.game_hours} h, asked ${from} + ${hours}`);
  const gaps = list.filter((frame, index) => frame.frame !== index).length;
  if (gaps) reasons.push(`${gaps} frames out of sequence`);
  if (back !== null && report.seek_back !== back) reasons.push(`asked to seek back to ${back}, the report says ${report.seek_back ?? 'none'}`);
  if (arg(run.args, '--paced') !== null) {
    const paced = report.paced;
    if (!paced) reasons.push('no paced section in the report');
    else if (held) {
      if (list.length !== hours) reasons.push(`${list.length} of ${hours} frames at the held hour`);
    } else {
      if (paced.frames !== list.length) reasons.push(`${list.length} frames logged of ${paced.frames} reported`);
      const last = list.at(-1);
      const reached = last ? last.head_hour + paced.speed_hours_per_second * (paced.seconds - last.start_us / 1e6) : null;
      const spanned = back === null ? reached : Math.max(...list.map(frame => frame.head_hour));
      if (spanned === null || spanned < from + hours) reasons.push(`the head reached ${spanned === null ? 'nothing' : Math.floor(spanned)}, asked ${from + hours}`);
      if (back !== null) {
        if (!(report.layers?.seeks ?? []).some(seek => seek.hour === back && seek.frames_to_shown != null)) reasons.push(`the seek back to ${back} was not shown`);
        if (reached === null || reached <= back) reasons.push(`the head ended at ${reached === null ? 'nothing' : Math.floor(reached)}, not past ${back}`);
      }
    }
    const seeks = num(run.args, '--seeks');
    if (seeks) {
      const made = (report.layers?.seeks ?? []).filter(seek => seek.frames_to_shown != null).length;
      if (made < seeks) reasons.push(`${made} of ${seeks} seeks shown`);
    }
  } else if (list.length !== hours + (back === null ? 0 : 1)) reasons.push(`${list.length} of ${hours + (back === null ? 0 : 1)} frames`);
  return { whole: reasons.length === 0, reasons, accepted: reasons.length === 0 && acceptReconciliation && reconciliation ? `judged with exit 1 from the harness's CPU reconciliation only (${off}): not a result under the rule` : null };
}

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
  const seekTo = arg(run.args, '--seek-to');
  const seeks = seekTo === null ? num(run.args, '--seeks') ?? 20 : seekTo.split(',').length;
  if ((summary.resets ?? []).length < seeks) reasons.push(`${(summary.resets ?? []).length} of ${seeks} seeks`);
  if (!summary.frames?.count) reasons.push('no frames recorded');
  if (!summary.timing) {
    const samples = (summary.compared ?? []).map(entry => entry.samples);
    if (!samples.length || Math.min(...samples) === 0) reasons.push(`samples compared ${JSON.stringify(samples)}`);
    if (summary.skipped_samples) reasons.push(`${summary.skipped_samples} samples skipped`);
  }
  return { whole: reasons.length === 0, reasons };
}

const row = (fields) => ({ worst: null, today: '', note: '', values: {}, ...fields });

function partialRows(page, label, bars, reasons) {
  return bars.map(bar => row({ page, run: label, bar, measured: '–', limit: '', verdict: PARTIAL, note: reasons.join('; ') }));
}

const PACED_BARS = ['window-thread CPU a frame', 'presented frame interval', 'late frames against the floor', 'content lag', 'new pictures a second',
  'each layer: apply, settle and upload', 'playhead step time', 'playhead stalls', 'playhead held()', 'process peak memory'];

const presented = frame => frame.start_us + frame.wall_us;

function seekWindows(report) {
  const inside = new Set();
  for (const seek of report.layers?.seeks ?? []) {
    const span = seek.frames_to_lag_within_a_step ?? seek.frames_to_shown ?? Infinity;
    for (let index = seek.frame; index <= seek.frame + Math.min(span, 1e6); index++) inside.add(index);
  }
  return inside;
}

export function pacedRows({ page = 'map', label, run, report, frames, floor = null, acceptReconciliation = false }) {
  const whole = wholeHarness(run, report, frames, { acceptReconciliation });
  const bars = [...PACED_BARS, ...(num(run.args, '--seeks') ? ['seek: sought hour on screen'] : [])];
  if (!whole.whole) return partialRows(page, label, bars, whole.reasons);
  const list = frames.frames, paced = report.paced;
  const hz = paced.refresh_hz, interval = 1e6 / hz, step = paced.speed_hours_per_second / hz;
  const shownAt = frame => frame.layers?.drawn != null;
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
  lagRow('the layers', frame => frame.layers?.lag_hours ?? null);

  const seconds = paced.seconds - list[first].start_us / 1e6;
  const pictures = (lagSide, changed) => {
    let count = 0;
    for (let index = 0; index < measured.length; index++) if (changed(measured[index], list[first + index - 1])) count++;
    rows.push(row({ page, run: label, bar: `new pictures a second, ${lagSide}`, measured: `${fixed(count / seconds, 1)} (${count} in ${fixed(seconds, 2)} s)`, limit: `one a frame (${hz})`,
      verdict: REPORTED, today: TODAY.pictures, values: { perSecond: count / seconds } }));
  };
  pictures('the layers', (frame, before) => frame.layers?.drawn !== before?.layers?.drawn);
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
  } else {
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


export function anchors(ticks) {
  const found = [];
  for (let index = 0; index < ticks.length; index++) {
    const tick = ticks[index], before = ticks[index - 1];
    if (!before || tick.playing !== before.playing || tick.speed !== before.speed || tick.seek !== before.seek) found.push(tick.at);
  }
  return found;
}

function latest(sorted, time) {
  let low = 0, high = sorted.length - 1, found = null;
  while (low <= high) {
    const mid = (low + high) >> 1;
    if (sorted[mid].at <= time) { found = sorted[mid]; low = mid + 1; } else high = mid - 1;
  }
  return found;
}

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

export function dashboardRows(log, { page = 'page', refreshHz = null, expect = {}, window = null } = {}) {
  const overlapsWindow = (record, w) => (record.from ?? record.at) <= w.to && (record.to ?? record.at) >= w.from;
  const records = window ? log.records.filter(record => record.t === 'lost' ? overlapsWindow(record, window)
    : !TIMED.has(record.t) || (record.at >= window.from && record.at <= window.to)) : log.records;
  const whole = wholeLog(log, records, expect);
  const bars = ['webview frame interval', 'long animation frames', 'requests for values at the head',
    'list requests ahead of the head', 'chart tile requests'];
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

  const requests = records.filter(record => record.t === 'q' && !record.cache && record.cmd !== undefined && !FOLLOW_PRODUCTS.has(record.product)
    && !(record.product === 'sprite-artwork' && record.hour === null) && inStretch(record.at) && !settling(record.at));
  const lists = requests.filter(record => record.product !== 'series-tiles');
  const ahead = lists.filter(aheadOfHead);
  const values = lists.filter(record => !aheadOfHead(record));
  const byProduct = new Map();
  for (const record of values) byProduct.set(record.product, (byProduct.get(record.product) ?? 0) + 1);
  const reads = new Map();
  for (const frame of played.filter(frame => !settling(frame.at))) for (const [widget, work] of Object.entries(frame.work ?? {})) if (work.read) reads.set(widget, (reads.get(widget) ?? 0) + work.read);
  rows.push(row({ page, run: 'perf log', bar: 'requests for values at the head', measured: values.length ? [...byProduct].map(([product, count]) => `${product} ${count}`).join(', ') : 'none',
    limit: 'none while playing', verdict: values.length ? FAIL : PASS, worst: values.length ? { frame: null, hour: hourAt(values[0].at) } : null, today: TODAY.requests,
    note: reads.size && values.length ? `widgets with reads: ${[...reads].map(([widget, count]) => `${titles.get(widget) ?? widget} ${count}`).join(', ')}` : '', values: { count: values.length } }));
  rows.push(aheadRow(page, ahead, playing, ticks, hourAt));
  rows.push(tileRow(page, requests.filter(record => record.product === 'series-tiles'), playing, ticks, hourAt));
  return { whole, rows, follow };
}

export const aheadOfHead = record => typeof record.from === 'number' && typeof record.head === 'number' && record.from > record.head;

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


const cell = value => String(value ?? '').replace(/\|/g, '\\|').replace(/\n/g, ' ');

export function table(rows) {
  const lines = ['| Page | Run | Bar | Measured | The bar | Verdict | Worst frame (hour) | Today (P0) | Note |', '|---|---|---|---|---|---|---|---|---|'];
  for (const entry of rows) {
    const worst = entry.worst ? `${entry.worst.frame ?? '–'} (${entry.worst.hour == null ? '–' : Math.floor(entry.worst.hour)})` : '';
    const verdict = entry.verdict === FAIL || entry.verdict === PARTIAL ? `**${entry.verdict}**` : entry.verdict;
    lines.push(`| ${[entry.page, entry.run, entry.bar, entry.measured, entry.limit, verdict, worst, entry.today, entry.note].map(cell).join(' | ')} |`);
  }
  return lines.join('\n');
}

export const passed = rows => !rows.some(entry => entry.verdict === FAIL || entry.verdict === PARTIAL);
