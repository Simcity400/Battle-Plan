import { COST_FORMULA, datedPlaybackVerdict, summarizeWidgetCosts } from './datedPlayback.mjs';
import { createReadStream, existsSync } from 'node:fs';
import { readFile, readdir, writeFile } from 'node:fs/promises';
import readline from 'node:readline';
import path from 'node:path';
import { compareCaptures } from '../../ui/e2e/compare.mjs';
import { mergeHistory, readHistory, slowestCards } from './coverage.mjs';
import { sought, MAP_FRAME_MS, MAP_P99_MS, UNPRESENTED, asked, catalogueHour, classifyPerfFile, costSessionConfinement, machineLine, profileLines, sessionConfinement, framesLate, handledAt, freshnessByKind, hoursInstalled, losses, mapFrames, phaseSeconds, popoutPresented, presentedAt, playbackPace, windowLosses } from './verdicts.mjs';
import { BUDGET_MS, FRAME_BUDGET_MS, MAX_PLAYBACK_AGE_HOURS, SUSTAINED_DROPS, consoleFailures, failedReplies, firstContent, frameTimes, groupBy, item, mapAcks, mapSeek, quantiles } from './verdicts.mjs';
import { FAIL, PARTIAL, dashboardRows } from './playbackVerdicts.mjs';
import { perFrame } from './trace.mjs';

const flag = name => { const at = process.argv.indexOf(name); return at < 0 ? null : process.argv[at + 1]; };
const files = Object.fromEntries(['run', 'sweeps', 'perf', 'out', 'baseline', 'history'].map(name => [name, flag(`--${name}`)]));
const legacy = process.argv.includes('--legacy') || process.argv.includes('--legacy-regrade');
const readJSON = async file => (file && existsSync(file) ? JSON.parse(await readFile(file, 'utf8')) : null);

const run = await readJSON(files.run) ?? {};
async function sweeps(folder) {
  const names = folder ? (await readdir(folder).catch(() => [])).filter(name => /^sweep-.+\.json$/.test(name)).sort() : [];
  if (!names.length) return null;
  const parts = await Promise.all(names.map(name => readJSON(path.join(folder, name))));
  const meta = { ...parts[0].meta, catalogue: parts.find(part => part.meta.catalogue)?.meta.catalogue,
    seconds: parts.reduce((sum, part) => sum + (part.meta.seconds ?? 0), 0), sessions: parts.map(part => part.meta) };
  return { meta, results: parts.flatMap(part => part.results), console: parts.flatMap(part => part.console) };
}
const sweep = await sweeps(files.sweeps);

async function perfRecords(folder, { keepQueries = false } = {}) {
  const names = folder ? await readdir(folder).catch(() => []) : [];
  const classified = names.map(name => ({ name, info: classifyPerfFile(name) })).filter(entry => entry.info !== null);
  const pages = classified.filter(entry => entry.info.kind === 'page');
  const viewers = classified.filter(entry => entry.info.kind === 'viewer');
  if (!pages.length) return { records: null, mapRecords: [] };
  const read = async (name, keep) => {
    for await (const line of readline.createInterface({ input: createReadStream(path.join(folder, name), 'utf8'), crlfDelay: Infinity })) keep(line);
  };
  const records = [], mapRecords = [];
  const wanted = /^\{"t":"(map_state|tick|workerFrame|picture|ink|commit|shown|widget|widget_error|q|frame|lost|page)"/;
  const lost = line => line.includes('"t":"lost"') && JSON.parse(line).t === 'lost';
  for (const { name, info: { session } } of pages) {
    await read(name, line => {
      if (!wanted.test(line) && !lost(line)) return;
      const record = JSON.parse(line);
      if (record.t === 'frame' && !record.playing) return;
      if (record.t === 'q') {
        if (record.outcome !== 'error' && !keepQueries) return;
        records.push(record.outcome === 'error' ? { ...record, session }
          : { t: 'q', at: record.at, cmd: record.cmd, product: record.product, cache: record.cache, outcome: record.outcome, hour: record.hour, ms: record.ms, session });
        return;
      }
      records.push({ ...record, session });
    });
  }
  for (const { name, info: { session } } of viewers) {
    await read(name, line => {
      if (/^\{"t":"map(_open|_upload)?"/.test(line)) mapRecords.push({ ...JSON.parse(line), session });
      else if (lost(line)) records.push({ ...JSON.parse(line), session });
    });
  }
  return { records, mapRecords };
}
const hasWidgetCosts = Boolean(sweep?.results?.some(result => result.area === 'widget-costs'));
const { records, mapRecords } = await perfRecords(files.perf, { keepQueries: hasWidgetCosts });

async function visualManifest(folder) {
  const names = (await readdir(folder).catch(() => [])).filter(name => /^manifest-.+\.json$/.test(name)).sort();
  if (!names.length) return false;
  const parts = await Promise.all(names.map(name => readJSON(path.join(folder, name))));
  await writeFile(path.join(folder, 'manifest.json'), JSON.stringify({ complete: parts.every(part => part.complete), items: parts.flatMap(part => part.items) }));
  return true;
}
await visualManifest(path.join(files.out, 'visual'));

const sections = [];
const fmt = value => (value === null || value === undefined || !Number.isFinite(value) ? '–' : String(Math.round(value)));
const verdict = pass => (pass ? 'pass' : '**FAIL**');
const why = reasons => (reasons ?? []).slice(0, 4).join('; ').replace(/\|/g, '\\|').replace(/\n/g, ' ') + ((reasons?.length ?? 0) > 4 ? `; and ${reasons.length - 4} more` : '');
let judging = null;
const under = (...phases) => { judging = phases.length ? phases : null; };
const section = (title, columns, rows, note = null) => { sections.push({ title, columns, rows, note, phases: judging }); return rows; };
const quantileCells = q => [q.n, fmt(q.p50), fmt(q.p95), fmt(q.max)];

const results = sweep?.results ?? [];
for (const result of results.filter(result => result.drawnAt != null)) {
  if (result.item.startsWith('hover: ') && result.delay) result.deadline = handledAt(mapRecords, result.posted) === null ? null : handledAt(mapRecords, result.posted) + result.delay;
  else if (result.item.startsWith('hover: ')) result.deadline = result.delay === null ? null : result.posted;
  const start = result.item.startsWith('mode: ') ? result.sent : result.item.startsWith('hover: ') ? result.deadline : result.input;
  if (result.item.startsWith('hover: ') && result.delay && result.deadline === null) {
    Object.assign(result, { ms: null, reasons: [...result.reasons, 'the viewer logged no frame handling the move, so its tooltip delay cannot be placed'], pass: false });
    continue;
  }
  if (start == null) continue;
  const at = presentedAt(mapRecords, result.drawnAt);
  const budget = result.budget ?? BUDGET_MS;
  const reasons = result.reasons.filter(reason => !reason.includes(' ms, over '));
  if (at === null) Object.assign(result, { ms: null, reasons: [...reasons, UNPRESENTED] });
  else {
    const exact = result.item.startsWith('hover: ') || result.label === 'show map';
    const ms = exact ? at - start : Math.max(result.ms ?? 0, at - start);
    if (result.check !== false && ms > budget) reasons.push(`${Math.round(ms)} ms, over ${budget} ms${result.delay ? ` after the ${Math.round(result.delay)} ms tooltip delay` : ''}`);
    Object.assign(result, { ms, reasons, presentedAt: at });
  }
  result.pass = result.reasons.length === 0;
}
for (const result of results.filter(result => result.item === 'pop-out' && result.label === 'open pop-out' && result.input != null)) {
  const at = popoutPresented(mapRecords, result.input, result.hour);
  const reasons = result.reasons.filter(reason => !reason.includes(' ms, over '));
  if (at === null) reasons.push(`the pop-out's viewer logged no presented frame of hour ${result.hour}, so its first frame is unobservable`);
  const ms = at === null ? null : at - result.input;
  if (ms !== null && ms > (result.budget ?? BUDGET_MS)) reasons.push(`${Math.round(ms)} ms, over ${result.budget ?? BUDGET_MS} ms`);
  Object.assign(result, { ms, reasons, presentedAt: at, pass: reasons.length === 0 });
}
if (!sweep) section('Sweep', ['Area', 'Result'], [{ cells: ['Sweep', `**FAIL**: not run: ${run.sweepNotRun ?? 'it did not run'}`], pass: false }]);
const area = name => results.filter(result => result.area === name);
const phaseFailures = results.filter(result => result.item === 'phase');
if (phaseFailures.length) section('Phases that stopped', ['Phase', 'Reason', 'Screenshot'],
  phaseFailures.map(result => ({ pass: false, cells: [result.area, why(result.reasons), result.screenshot ?? ''] })));

if (sweep) {
  const placements = firstContent(results);
  const kinds = new Set(placements.map(placement => placement.kind));
  const catalogue = sweep.meta.catalogue ?? [];
  const catalogueAdds = area('catalogue').filter(result => result.label === 'add');
  const showedContent = catalogueAdds.filter(result => result.widgets?.some(widget => (widget.end === 'content' || widget.end === 'empty') && !widget.errors?.length));
  const addedCards = new Set(area('catalogue').filter(result => result.item !== 'setup').map(result => result.item));
  const offered = run.tier === 'quick' ? catalogue.filter(card => addedCards.has(`${card.view ? 'view' : 'widget'}/${card.title}`)) : catalogue;
  const playedPages = new Set(area('playback').map(result => result.page)).size;
  section('Coverage', ['Area', 'Covered', 'Result'], [
    ...(asked(run, 'pages') ? [{ cells: ['Pages', `${new Set(area('pages').map(result => result.item.split('/')[1])).size} of ${sweep.meta.pages.length}, cold and warm`], pass: true },
      { cells: ['Widget kinds on default pages', `${kinds.size} kinds, ${placements.filter(p => p.visit === 'cold').length} placements`], pass: true }] : []),
    ...(asked(run, 'catalogue') ? [{ cells: ['Catalogue', `${offered.filter(card => card.view).length} views and ${offered.filter(card => !card.view).length} widget cards; ${showedContent.length} of ${offered.length} added showed content (their timings are judged below)`], pass: showedContent.length === offered.length }] : []),
    ...(asked(run, 'controls') ? [{ cells: ['Control cases', `${new Set(area('controls').map(result => result.item)).size} cases, ${area('controls').length} actions`], pass: true }] : []),
    ...(asked(run, 'seeks') ? [{ cells: ['Seeks', `${area('seeks').length} (${sweep.meta.seeks.map(([label]) => label).join(', ')} on every page)`], pass: true }] : []),
    ...(area('pages-at').length ? [{ cells: ['Pages at a chosen hour', `${new Set(area('pages-at').map(result => result.item.split('/')[1])).size} of ${sweep.meta.pages.length}`], pass: true }] : []),
    ...(asked(run, 'playback') ? [{ cells: ['Playback', `${area('playback').length} runs (${sweep.meta.speeds.join(' and ')} h/s on ${playedPages === sweep.meta.pages.length ? 'every page' : `${playedPages} pages`})`], pass: true },
      { cells: ['Browser trace', `${area('trace').filter(result => result.trace).length} of ${playedPages} pages traced playing`],
        pass: area('trace').filter(result => result.trace).length >= playedPages }] : []),
  ].map(row => ({ ...row, cells: [...row.cells, verdict(row.pass)] })));

  const sessionMetaBySlug = new Map((sweep.meta.sessions ?? []).map(meta => [meta.session, meta]));
  section('Cold sessions (each a fresh app on the profile; everything it times first is timed cold)', ['Session', 'Phases', 'Started with', 'Opened campaign (ms)', 'Took', 'App exit', 'Confinement', 'Result'],
    (run.sessions ?? []).map(entry => {
      const slug = entry.name.replace(/ /g, '-');
      const opened = results.find(result => result.area === 'startup' && result.session === slug);
      const conf = sessionConfinement(run, entry, sessionMetaBySlug.get(slug) ?? null);
      const failures = [];
      if (entry.error) failures.push(`it could not start: ${entry.error}`);
      else if (entry.sweepExit !== 0) failures.push(`the sweep exited ${entry.sweepExit}`);
      else if (entry.appExit !== 0) failures.push(`the app exited ${entry.appExit ?? 'only when stopped'}`);
      failures.push(...conf.reasons);
      const pass = failures.length === 0;
      return { pass, cells: [entry.name, entry.phases.join(', '), `emptied: ${entry.emptied.join(', ')}; kept only: ${entry.kept.join(', ')}`,
        fmt(opened?.ms), `${Math.round(entry.seconds)} s`, entry.appExit ?? '–', conf.summary, pass ? 'pass' : `**FAIL**: ${why(failures)}`] };
    }), 'Before each session the suite deletes the content cache (BattlePlan\\viewer under the profile\'s local app data), the shader cache, the WebView2 profile with its HTTP and code caches, and the app\'s settings, and checks they are gone. Kept: the recording. Reply and answer caches live in the app\'s memory and start empty with each session. The user\'s own BattlePlan folder is never used: the profile has its own local app data.');

  const runThrottle = run.cpuThrottle ?? sweep.meta.cpuThrottle ?? 1;
  section(`Opening the campaign (ms from the click on its library row to the dashboard and its visible widgets showing content, in each cold session${runThrottle > 1 ? `, CPU throttled ×${runThrottle}` : ''})`, ['Session', 'Step', 'ms', 'Result', 'Why'],
    area('startup').map(result => ({ pass: result.pass, cells: [result.session, result.label ?? result.item, fmt(result.ms), verdict(result.pass), why(result.reasons)] })));

  under('pages');
  const pageRows = [];
  for (const page of sweep.meta.pages) {
    const cells = [page], reasons = [];
    let pass = true;
    for (const passName of ['cold', 'warm']) {
      const steps = area('pages').filter(result => result.item === `${passName}/${page}`);
      const switchStep = steps.find(result => result.label === 'switch');
      const scrolls = steps.filter(result => result.label.startsWith('scroll'));
      const scroll = item('scroll', scrolls);
      cells.push(fmt(switchStep?.ms), fmt(scroll.max));
      for (const result of steps.filter(result => !result.pass)) { pass = false; reasons.push(`${passName} ${result.label}: ${why(result.reasons)}`); }
      if (!switchStep) { pass = false; reasons.push(`${passName}: never visited`); }
    }
    const widgets = placements.filter(placement => placement.page === page && placement.visit === 'cold');
    pageRows.push({ pass, cells: [...cells, widgets.length, verdict(pass), why(reasons)] });
  }
  section('Pages (ms from the click or wheel turn to the slowest visible widget\'s content; cold judged, warm beside it)', ['Page', 'Cold switch', 'Cold scroll max', 'Warm switch', 'Warm scroll max', 'Widgets', 'Result', 'Why'], pageRows);

  under();
  const pagesAt = (sweep.meta.sessions ?? []).find(meta => meta.pagesAt)?.pagesAt;
  if (pagesAt || area('pages-at').length) {
    const hours = [...new Set(pagesAt?.hours ?? [])];
    section(`Pages at ${pagesAt?.fraction ?? '?'} of the timeline (hour ${hours.join(', ') || '?'}; ms from the click or wheel turn to the slowest visible widget's content, after one seek there per page)`,
      ['Page', 'Switch', 'Scroll max', 'Widgets', 'Content', 'Empty', 'Unavailable', 'Other', 'Result', 'Why'],
      sweep.meta.pages.map(page => {
        const steps = area('pages-at').filter(result => result.item === `at/${page}`);
        const switchStep = steps.find(result => result.label === 'switch');
        const widgets = new Map();
        for (const step of steps) for (const widget of step.widgets ?? []) if (!widgets.has(widget.id)) widgets.set(widget.id, widget.end);
        const ends = [...widgets.values()];
        const count = end => ends.filter(value => value === end).length;
        const reasons = steps.filter(result => !result.pass).map(result => `${result.label}: ${why(result.reasons)}`);
        if (!switchStep) reasons.push('never visited');
        const pass = reasons.length === 0;
        return { pass, cells: [page, fmt(switchStep?.ms), fmt(item('scroll', steps.filter(result => result.label.startsWith('scroll'))).max), widgets.size,
          count('content'), count('empty'), count('unavailable'), ends.length - count('content') - count('empty') - count('unavailable'), verdict(pass), why(reasons)] };
      }));
  }

  under('pages');
  const byKind = groupBy(placements, placement => placement.kind);
  section('First content by widget kind (ms from input; every placement on the default pages; the 100 ms rule judges cold, warm is beside it)', ['Kind', 'Placements', 'Cold p50', 'Cold p95', 'Cold max', 'Warm p50', 'Warm p95', 'Warm max', 'Result', 'Why'],
    [...byKind].sort(([a], [b]) => a.localeCompare(b)).map(([kind, list]) => {
      const cold = item(kind, list.filter(p => p.visit === 'cold')), warm = item(kind, list.filter(p => p.visit === 'warm'), { budget: Infinity });
      const pass = cold.pass && warm.pass;
      return { pass, cells: [kind, cold.n, fmt(cold.p50), fmt(cold.p95), fmt(cold.max), fmt(warm.p50), fmt(warm.p95), fmt(warm.max), verdict(pass), why([...cold.reasons.map(r => `cold ${r}`), ...warm.reasons.map(r => `warm ${r}`)])] };
    }));

  under('seeks');
  const starts = (sweep.meta.sessions ?? []).map(meta => Date.parse(meta.started)).filter(Number.isFinite);
  const inputs = (sweep.meta.sessions ?? []).flatMap(meta => meta.seekInputs ?? []).sort((a, b) => a - b);
  const dated = new Set(records?.filter(r => r.t === 'picture').map(r => r.widget));
  const shownBy = records ? groupBy(records.filter(record => record.t === 'picture' || record.t === 'shown' && !dated.has(record.widget)).sort((a, b) => a.at - b.at), record => record.widget) : new Map();

  section('Seeks (ms from the click on the timeline to every visible widget showing content; each hour cold in a fresh session, then warm)',
    ['Page', ...sweep.meta.seeks.map(([label]) => `${label} cold`), ...sweep.meta.seeks.map(([label]) => `${label} warm`), 'Cold p50', 'Cold p95', 'Cold max', 'Result', 'Why'],
    sweep.meta.pages.map(page => {
      const steps = area('seeks').filter(result => result.item === page).map(result => sought(result, shownBy, starts, inputs.find(at => at > result.input) ?? Infinity));
      const cold = steps.filter(result => !result.label.endsWith(' warm'));
      const warm = steps.filter(result => result.label.endsWith(' warm'));
      const summary = item(page, cold);
      const warmFailures = warm.filter(result => !result.pass).flatMap(result => result.reasons.map(reason => `${result.label}: ${reason}`));
      const pass = summary.pass && !warmFailures.length;
      return { pass, cells: [page, ...sweep.meta.seeks.map(([label]) => fmt(cold.find(result => result.label === label)?.ms)),
        ...sweep.meta.seeks.map(([label]) => fmt(warm.find(result => result.label === `${label} warm`)?.ms)),
        fmt(summary.p50), fmt(summary.p95), fmt(summary.max), verdict(pass), why([...summary.reasons, ...warmFailures])] };
    }));

  under('controls');
  const controls = groupBy(area('controls'), result => result.item);
  const warmOf = list => list.filter(result => result.check === false), coldOf = list => list.filter(result => result.check !== false);
  section('Interactions (ms from trusted input to finished: painted, touched widgets showing content, the control\'s result shown and a setting\'s change shown on its widget or drawn by its map; a drag or a typed field from its last input. Cold: each control\'s first use, judged. Warm: the revisit that puts it back, beside it)',
    ['Case', 'Actions', 'Cold p50', 'Cold p95', 'Cold max', 'Warm p50', 'Warm p95', 'Warm max', 'Budget', 'Result', 'Why'],
    [...controls].map(([name, list]) => {
      const budget = Math.max(BUDGET_MS, ...list.map(result => result.budget ?? BUDGET_MS));
      const cold = item(name, coldOf(list), { budget }), warm = warmOf(list).length ? item(name, warmOf(list), { budget: Infinity }) : null;
      const reasons = [...cold.reasons, ...(warm?.reasons ?? []).map(reason => `warm ${reason}`)];
      return { pass: !reasons.length, cells: [name, list.length, fmt(cold.p50), fmt(cold.p95), fmt(cold.max), fmt(warm?.p50), fmt(warm?.p95), fmt(warm?.max), budget, verdict(!reasons.length), why(reasons)] };
    }));
  const longest = list => { const values = list.map(result => result.ms).filter(Number.isFinite); return values.length ? Math.max(...values) : null; };
  section('Interactions by control (every page, widget and control: its first use and the revisit that puts it back; a drag\'s gesture and its page frames beside)',
    ['Case', 'Control', 'Cold ms', 'Warm ms', 'Gesture ms', 'Frame gap max', 'Result', 'Why'],
    [...groupBy(area('controls'), result => `${result.item}\u0000${result.control ?? result.label}`)].map(([key, list]) => {
      const [name, control] = key.split('\u0000');
      const reasons = list.filter(result => !result.pass).flatMap(result => result.reasons.map(reason => (result.label === control ? reason : `${result.label}: ${reason}`)));
      const gesture = list.find(result => result.gestureMs != null || result.frameGapMax != null);
      return { pass: !reasons.length, cells: [name, control, fmt(longest(coldOf(list))), fmt(longest(warmOf(list))), fmt(gesture?.gestureMs), gesture?.frameGapMax?.toFixed(1) ?? '–',
        verdict(!reasons.length), why(reasons)] };
    }));

  under('catalogue');
  section('Catalogue (ms from Add to the added view showing content; the hour it reports showing against the clock)', ['Entry', 'ms', 'Hour shown', 'Result', 'Why'],
    area('catalogue').filter(result => result.label === 'add').map(result => {
      const from = result.shownFrom ?? result.input;
      const hour = records ? catalogueHour(records, { ...result, input: from }) : { pass: false, reason: 'no perf log' };
      const presented = hour.pass && result.shownFrom != null ? presentedAt(mapRecords, hour.drawnAt) : null;
      const ms = !hour.pass ? result.ms : result.shownFrom != null ? (presented === null ? null : presented - from) : Math.max(result.ms ?? 0, hour.at - from);
      const reasons = [...result.reasons.filter(reason => !reason.includes(' ms, over ')), ...(hour.pass ? [] : [hour.reason])];
      if (hour.pass && result.shownFrom != null && presented === null) reasons.push(UNPRESENTED);
      if (ms !== null && ms > BUDGET_MS) reasons.push(`${Math.round(ms)} ms, over ${BUDGET_MS} ms`);
      return { pass: !reasons.length, cells: [result.item, fmt(ms), hour.hour ?? '–', verdict(!reasons.length), why(reasons)] };
    }));
  const catalogueSetup = item('catalogue setup', area('catalogue').filter(result => result.item === 'setup'));
  section('Catalogue editing', ['Item', 'Actions', 'p50', 'p95', 'Max', 'Result', 'Why'],
    [{ pass: catalogueSetup.pass, cells: ['open, delete, cancel', ...quantileCells(catalogueSetup), verdict(catalogueSetup.pass), why(catalogueSetup.reasons)] }]);

  under('seeks');
  const mapRows = [];
  if (records) {
    const acks = mapAcks(records);
    for (const result of area('seeks').filter(result => result.maps?.length && result.input !== null)) {
      for (const [view, list] of [...acks].filter(([view]) => result.maps.includes(view))) {
        const { ms, reason } = mapSeek(list, result.input, mapRecords);
        const judged = result.check !== false;
        const pass = ms !== null && (!judged || ms <= BUDGET_MS);
        mapRows.push({ pass, judged, cells: [`seek ${result.label} (${result.item})`, view, fmt(ms), verdict(pass), reason ?? (pass ? '' : `${Math.round(ms)} ms, over ${BUDGET_MS} ms`)] });
      }
    }
    if (!mapRows.length) mapRows.push({ pass: false, cells: ['seek', '', '–', verdict(false), 'no map acknowledgements in the perf log'] });
  } else mapRows.push({ pass: false, cells: ['seek', '', '–', verdict(false), 'no perf log'] });
  const seekTimes = quantiles(mapRows.filter(row => row.judged).map(row => Number(row.cells[2])).filter(Number.isFinite));
  section('Map seeks (ms from the timeline click to the present of the frame drawing the sought hour; warm seeks beside the cold ones, not judged)', ['Item', 'View', 'ms', 'Result', 'Why'], mapRows,
    `Cold seeks: p50 ${fmt(seekTimes.p50)}, p95 ${fmt(seekTimes.p95)}, max ${fmt(seekTimes.max)} ms.`);
  under('map');
  const modes = area('map').filter(result => result.item.startsWith('mode: '));
  const modeSummary = item('map modes', modes.filter(result => !result.unavailable));
  section(`Map modes (ms from the view's options sent with the mode, as the host sends them, to the present of the first frame the viewer drew in it): p50 ${fmt(modeSummary.p50)}, p95 ${fmt(modeSummary.p95)}, max ${fmt(modeSummary.max)}`,
    ['Mode', 'ms', 'Result', 'Why'], modes.length ? modes.map(result => ({ pass: result.pass, cells: [`${result.item.slice(6)} (${result.label})`, result.unavailable ? 'unavailable' : fmt(result.ms),
      verdict(result.pass), result.unavailable ? `the recording cannot draw it: ${result.unavailable}` : why(result.reasons)] }))
      : [{ pass: false, cells: ['–', '–', verdict(false), 'no map mode was chosen'] }]);
  section(`Map interactions (the viewer's frames at 120 Hz: window-thread work and GPU time per frame within ${MAP_FRAME_MS.toFixed(2)} ms, p99 within ${MAP_P99_MS} ms; a frame over ${MAP_FRAME_MS.toFixed(2)} ms is a dropped frame)`,
    ['Interaction', 'While', 'Frames', 'Work p50', 'Work p99', 'Work max', 'GPU p99', 'GPU max', 'Dropped', 'Result', 'Why'],
    area('map').filter(result => result.item.startsWith('interaction: ')).map(result => {
      const frames = result.from ? mapFrames(mapRecords, result.from, result.to) : null;
      const reasons = [...result.reasons, ...(frames ? frames.reasons : ['no window measured'])];
      if (result.item.startsWith('interaction: playback') && result.from) {
        const hours = hoursInstalled(mapRecords, result.view ?? null, result.from, result.to);
        if (hours < 2) reasons.push(`the map installed ${hours} hours while playing: it did not play`);
      }
      const pass = reasons.length === 0;
      const ms = value => value?.toFixed(1) ?? '–';
      return { pass, cells: [result.item.slice(13), result.label, frames?.frames ?? '–', ms(frames?.work.p50), ms(frames?.work.p99), ms(frames?.work.max),
        ms(frames?.gpu.p99), ms(frames?.gpu.max), frames ? `${frames.dropped} of ${frames.frames}` : '–', verdict(pass), why(reasons)] };
    }), "Pointer and key input to the native map (zoom, pan, Fly and Walk, hover, clicks) enters through the viewer's suite input channel (BATTLEPLAN_E2E_INPUT, off for users) as egui's own events, where the windowing library's translated input arrives; everything after it runs as for a user. Only the windowing library's translation of real input is untested: the suite runs on a desktop of its own, without a cursor. Native viewer phases (map and mapfps) run under OS core confinement when profiled, without Chromium JS CPU throttling.");
  under('mapfps');
  const uncapped = area('mapfps');
  section('Map frame rate achieved, presented uncapped (the viewer without vsync, in a session of its own)', ['Interaction', 'While', 'Frames', 'FPS', 'Work p99', 'Note'],
    uncapped.length ? uncapped.map(result => {
      const frames = result.from ? mapFrames(mapRecords, result.from, result.to) : null;
      const ran = result.pass && !!frames?.frames;
      return { pass: ran, cells: [result.item.slice(13), result.label, frames?.frames ?? '–', frames?.frames ? Math.round(frames.fps) : '–', frames?.work.p99?.toFixed(1) ?? '–',
        ran ? '' : `**FAIL**: ${why([...result.reasons, ...(frames?.frames ? [] : ['no frames recorded'])])}`] };
    }) : [{ pass: false, cells: ['–', '–', '–', '–', '–', '**FAIL**: the uncapped session did not run'] }]);
  under('map');
  section('Map hover (ms from the pointer moving onto it, after the game\'s tooltip delay for a tooltip, to the present of the frame drawing it; it must be the next frame and within 100 ms)', ['Target', 'Highlight', 'ms', 'Frames late', 'Result', 'Why'],
    area('map').filter(result => result.item.startsWith('hover: ')).map(result => {
      const late = result.drawnAt != null && result.deadline != null ? framesLate(mapRecords, result.deadline, result.drawnAt) : null;
      const reasons = [...result.reasons, ...(late ? [`${late} frames drawn before the highlight`] : [])];
      const pass = result.pass && !late;
      return { pass, cells: [result.item.slice(7), result.label ?? '–', fmt(result.ms), late ?? '–', verdict(pass), why(reasons)] };
    }));
  section('Pop-out map', ['Step', 'ms', 'Result', 'Why'], area('map').filter(result => result.item === 'pop-out' || result.item === 'phase')
    .map(result => ({ pass: result.pass, cells: [result.label ?? result.item, fmt(result.ms), verdict(result.pass), why(result.reasons)] })));

  under('playback');

  const refresh = run.refreshHz ?? null;
  section(`Playback (frame time: each playing frame's main-thread time to render, against ${FRAME_BUDGET_MS.toFixed(1)} ms; dropped: an interval over 1.5 refreshes of the ${refresh ?? '?'} Hz display, failing on ${SUSTAINED_DROPS} or more in a row)`,
    ['Page', 'h/s', 'Frames', 'Frame p50', 'Frame p95', 'Frame max', 'Interval p95', 'Dropped', 'Longest drop run', 'Result', 'Why'],
    area('playback').map(result => {
      const sessionRecords = (records ?? []).filter(r => !result.session || !r.session || r.session === result.session);
      const { count, main, interval, drops } = frameTimes(sessionRecords, result.from, result.to, refresh);
      const reasons = [...result.reasons];
      const ticks = records ? sessionRecords.filter(record => record.t === 'tick' && record.playing && record.at >= result.from && record.at <= result.to) : null;
      if (ticks && !ticks.length) reasons.push('the clock recorded no ticks while playing');
      else if (ticks) {
        const other = ticks.find(tick => tick.speed !== result.speed);
        if (other) reasons.push(`the clock played at ${other.speed} h/s, not the ${result.speed} h/s chosen`);
        const sorted = [...ticks].sort((a, b) => a.at - b.at), first = sorted[0];
        const ended = result.lastHour == null ? -1 : sorted.findIndex(tick => tick.hour >= result.lastHour);
        const last = ended >= 0 ? sorted[ended] : sorted.at(-1);
        const seconds = (last.at - first.at) / 1000;
        const pace = playbackPace(first, last, result.speed);
        if (seconds < 1) reasons.push(`the clock ticked over only ${seconds.toFixed(2)} s, too little to tell its pace`);
        else if (!pace.pass) reasons.push(`measured clock speed ${pace.hps?.toFixed(1) ?? 'unknown'} h/s, expected ${result.speed} h/s`);
      }
      const hasWindowBounds = result.from != null && result.to != null;
      const lostWindow = hasWindowBounds ? windowLosses(sessionRecords, result.from, result.to) : { count: 0, sources: [] };
      if (lostWindow.count > 0 || (!hasWindowBounds && !result.complete)) {
        const detail = lostWindow.count > 0
          ? `${lostWindow.count} records (${lostWindow.sources.join(', ')})`
          : JSON.stringify(result.losses);
        reasons.push(`the perf log lost records: ${detail}`);
      }
      if (!count) reasons.push('no playing frames recorded');
      const untimed = count - main.n;
      if (untimed) reasons.push(`${untimed} playing frames have no main-thread time, so the frame times are incomplete`);
      else if (main.p95 > FRAME_BUDGET_MS) reasons.push(`frame p95 ${main.p95.toFixed(1)} ms, over ${FRAME_BUDGET_MS.toFixed(1)} ms`);
      if (interval.n < count) reasons.push(`${count - interval.n} playing frames have no interval`);
      if (!drops) reasons.push(refresh ? 'no perf log for dropped frames' : 'the display refresh rate was not recorded');
      else if (!drops.frames) reasons.push('no playing frame intervals in the perf log');
      else if (!drops.pass) reasons.push(`${drops.longest} dropped frames in a row (intervals over ${drops.limit.toFixed(1)} ms)`);
      const pass = reasons.length === 0;
      return { pass, cells: [result.page, result.speed, count, main.p50?.toFixed(1) ?? '–', main.p95?.toFixed(1) ?? '–', main.max?.toFixed(1) ?? '–',
        interval.p95?.toFixed(1) ?? '–', drops?.rate == null ? '–' : `${(drops.rate * 100).toFixed(1)}% (${drops.dropped})`, drops?.longest ?? '–',
        verdict(pass), why(reasons)] };
    }));

  const datedRuns = area('playback').filter(r => r.speed === 1440);
  let auditedPlacements = [];
  if (datedRuns.length || sweep?.meta.sessions?.some(s => s.phases?.includes('playback'))) {
    const result = datedPlaybackVerdict(datedRuns, records ?? [], { legacy });
    auditedPlacements = result.placements;
    await writeFile(path.join(files.out, 'dated-playback.json'), JSON.stringify(result, null, 2));
    const passedCount = result.placements.filter(p => p.pass === true).length;
    const failedCount = result.placements.filter(p => p.pass === false).length;
    const summaryNote = result.placements.length
      ? `Placements: ${passedCount} passed, ${failedCount} failed.`
      : null;
    section('Dashboard playback at 1,440 game hours per second', ['Result', 'Why'],
      [{ pass: result.pass, cells: [verdict(result.pass), why(result.reasons)] }], summaryNote);
  }

  const ms2 = value => (value === null || value === undefined || !Number.isFinite(value) ? '–' : value.toFixed(2));
  const traceThrottle = run.cpuThrottle ?? sweep.meta.cpuThrottle ?? 1;
  section(`Browser trace (one playing window a page under the DevTools trace${traceThrottle > 1 ? `, CPU throttled ×${traceThrottle}` : ''}, apart from the timed windows; ms a frame on the page's main thread unless named off it; the app's painting from the perf log over the same window)`,
    ['Page', 'h/s', 'Frames', 'Main busy', 'App painting', 'Script outside painting', 'Recalc style', 'Layout', 'Pre-paint, layers', 'Paint', 'Composite', 'GC', 'Other', 'Raster (off main)', 'GPU (off main)', 'Raw trace', 'What forces style and layout, and the longest scripts'],
    area('trace').map(result => {
      if (!result.trace) return { pass: false, cells: [result.page, result.speed, '–', '–', '–', '–', '–', '–', '–', '–', '–', '–', '–', '–', '–', '–', why(result.reasons)] };
      const each = perFrame(result.trace);
      const frames = records ? records.filter(record => record.t === 'frame' && (!result.session || !record.session || record.session === result.session) && record.at >= result.from && record.at <= result.to) : [];
      const painting = frames.length ? frames.reduce((sum, record) => sum + (record.paint_ms ?? 0), 0) / frames.length : null;
      const scripts = (result.trace.scripts ?? []).slice(0, 3).map(entry => `${entry.what} ${entry.ms} ms ×${entry.count}`);
      const throttleNote = result.cpuThrottle === 1 && traceThrottle > 1 ? ['unthrottled JS trace'] : [];
      return { pass: result.pass, cells: [result.page, result.speed, result.trace.frames ?? '–', ms2(each.main), ms2(painting),
        ms2(each.script === null || painting === null ? null : Math.max(0, each.script - painting)), ms2(each.style), ms2(each.layout), ms2(each.prePaint),
        ms2(each.paint), ms2(each.composite), ms2(each.gc), ms2(each.other), ms2(each.raster), ms2(each.gpu), result.file ? `[trace](${result.file.replaceAll('\\', '/')})` : '–',
        why([[...result.reasons, ...throttleNote, ...each.causes.slice(0, 6), ...scripts].join('; ')])] };
    }), area('trace').length ? null : 'No playing window was traced.');

  const sessionLogs = new Map();
  const sessionLog = async name => {
    if (!sessionLogs.has(name)) {
      const file = files.perf ? path.join(files.perf, `perf-${name}.jsonl`) : null;
      let log = null;
      if (file && existsSync(file)) {
        log = { records: [], cut: false };
        for (const line of (await readFile(file, 'utf8')).split(/\r?\n/)) {
          if (!line.trim()) continue;
          try { log.records.push(JSON.parse(line)); } catch { log.cut = true; }
        }
      }
      sessionLogs.set(name, log);
    }
    return sessionLogs.get(name);
  };
  const barRows = [];
  for (const result of area('playback').filter(result => result.from != null && result.to != null)) {
    const log = await sessionLog(result.session);
    const label = `${result.page} ${result.speed} h/s`;
    if (!log) { barRows.push({ pass: false, cells: [label, '–', '–', '–', verdict(false), 'no perf log for this session'] }); continue; }
    for (const entry of dashboardRows(log, { page: label, refreshHz: refresh, window: { from: result.from, to: result.to } }).rows) {
      const pass = entry.verdict !== FAIL && entry.verdict !== PARTIAL;
      barRows.push({ pass, cells: [entry.page, entry.bar, entry.measured, entry.limit, pass ? entry.verdict : `**${entry.verdict}**`, why([entry.note])] });
    }
  }
  section('Playback bars of the dashboard (section 10 of the playback design, tools/e2e/playbackVerdicts.mjs, over each playing window)',
    ['Page', 'Bar', 'Measured', 'The bar', 'Verdict', 'Note'], barRows.length ? barRows : [{ pass: false, cells: ['–', '–', '–', '–', verdict(false), 'no playback window was measured'] }]);

  under('playback', 'visual');
  const freshKinds = records ? freshnessByKind(records, area('playback'), area('hours'), starts, auditedPlacements) : null;
  section(`Freshness by widget kind (exact while paused, worker eligibility within ${MAX_PLAYBACK_AGE_HOURS} h at p95, zero future frames)`, ['Kind', 'Placements', 'Displayed lag p95 (h)', 'Worker eligible lag p95 (h)', 'Result', 'Why'],
    freshKinds === null ? [{ pass: false, cells: ['–', '–', '–', '–', verdict(false), 'no perf log'] }]
      : freshKinds.sort((a, b) => a.kind.localeCompare(b.kind)).map(kind => ({ pass: kind.pass, cells: [kind.kind, kind.placements, kind.worst === null ? '–' : kind.worst >= Number.MAX_VALUE ? 'none' : kind.worst, kind.eligible ?? '–', verdict(kind.pass), why(kind.reasons)] })));

  under('widget-costs');
  const costResults = area('widget-costs');
  if (costResults.length) {
    const emptyWindows = costResults.filter(r => r.isEmpty);
    const widgetWindows = costResults.filter(r => !r.isEmpty);
    const costSessionSlug = costResults[0]?.session ?? null;
    const costConf = costSessionConfinement(run, sweep, costSessionSlug);
    const costReport = summarizeWidgetCosts(widgetWindows, records ?? [], {
      baselines: emptyWindows,
      profile: { name: run.profile ?? null, cpuThrottle: run.cpuThrottle ?? sweep.meta.cpuThrottle ?? 1, cores: run.cores ?? null },
      confinement: costConf,
    });
    await writeFile(path.join(files.out, 'widget-costs.json'), JSON.stringify(costReport, null, 2));
    const ms1 = value => (value === null || value === undefined || !Number.isFinite(value) ? '–' : value.toFixed(1));
    const signed = value => (value === null || value === undefined || !Number.isFinite(value) ? '–' : `${value >= 0 ? '+' : ''}${value.toFixed(1)}`);
    const byOutcome = new Map();
    for (const w of costReport.widgets) byOutcome.set(w.outcome, (byOutcome.get(w.outcome) ?? 0) + 1);
    const outcomeLine = ['content', 'refusal', 'error'].map(k => `${byOutcome.get(k) ?? 0} ${k}`).join(', ');
    const baseMain = costReport.baseline.main;
    const sampleLine = (costReport.baseline.samples ?? []).map(sample =>
      `${sample.label}: p50 ${ms1(sample.main.p50)} / p95 ${ms1(sample.main.p95)} / max ${ms1(sample.main.max)} ms (${sample.main.n} frames)`).join('; ');
    const spreadLine = costReport.baseline.samples?.length >= 2
      ? ` Spread across samples: Δp50 ${ms1(costReport.baseline.spread?.p50Ms)} ms, Δp95 ${ms1(costReport.baseline.spread?.p95Ms)} ms.` : '';
    const costNote = `Each widget mounted alone at default size, playing at 1,440 h/s. Ranked by \`costScoreMs = ${COST_FORMULA}\` (ms/frame; \`Δp50\` shown beside \`Δp95\`; negative \`Δp95\` is never clamped to zero and is marked \`below baseline noise\`). Pooled empty-page baseline: p50 ${ms1(baseMain.p50)} / p95 ${ms1(baseMain.p95)} / max ${ms1(baseMain.max)} ms (${baseMain.n} frames${costReport.baseline.lostRecords ? `, ${costReport.baseline.lostRecords} lost records` : ''})${sampleLine ? ` [${sampleLine}]` : ''}.${spreadLine} Outcomes across ${costReport.widgets.length} kinds: ${outcomeLine}. Full data in \`widget-costs.json\`.`;
    const costRows = [];
    if (!costReport.baseline.usable) {
      costRows.push({
        pass: false,
        cells: [
          '–', '__empty__ (baseline)', 'empty', '**FAIL**', '–',
          ms1(baseMain.p50), '–', ms1(baseMain.p95), '–',
          `${baseMain.n} (${ms1(costReport.baseline.seconds)} s)`, '–', '–', '–', '–', 'n/a',
          why(costReport.baseline.reasons),
        ],
      });
    }
    for (const [index, w] of costReport.widgets.entries()) {
      const reqList = Object.entries(w.requestsByProduct ?? {}).slice(0, 4).map(([prod, r]) => `${prod} ${r.toFixed(1)}`).join(', ');
      const reqCell = w.requestsPerSec == null ? '–' : `${w.requestsPerSec.toFixed(1)}${reqList ? ` (${reqList})` : ''}`;
      const workerCell = w.worker.changedFrames > 0 ? `${ms1(w.worker.p95)} / ${ms1(w.worker.mean)}` : '–';
      const framesCell = `${w.main.n} (${ms1(w.seconds)} s)`;
      const notes = [];
      if (w.detail) notes.push(w.detail);
      if (w.belowBaselineNoise) notes.push('below baseline noise');
      if (w.lowFrameCount) notes.push(`low sample: ${w.main.n} frames in ${ms1(w.seconds)} s`);
      if (w.correctnessErrors?.length) notes.push(...w.correctnessErrors);
      if (w.postWindowAuditErrors?.length) {
        const firstPost = w.postWindowAuditErrors[0];
        notes.push(`post-window audit @ ${new Date(firstPost.at).toISOString()}: ${firstPost.error}`);
      }
      if (w.incompleteReasons?.length) notes.push(...w.incompleteReasons);
      costRows.push({
        pass: w.pass,
        cells: [
          index + 1, w.kind, w.outcome, verdict(w.pass), ms1(w.costScoreMs),
          ms1(w.main.p50), signed(w.mainDelta.p50), ms1(w.main.p95), signed(w.mainDelta.p95),
          framesCell, workerCell, w.commitsPerSec == null ? '–' : w.commitsPerSec.toFixed(1), reqCell,
          w.lagP95Hours == null ? '–' : Math.round(w.lagP95Hours), w.exact, why([...new Set(notes)]),
        ],
      });
    }
    section('Per-widget playback cost at 1,440 h/s (sorted most expensive first)',
      ['Rank', 'Kind', 'Outcome', 'Result', 'Score (ms)', 'Main p50', 'Δp50', 'Main p95', 'Δp95', 'Frames (s)', 'Worker p95 / mean', 'Commits/s', 'Req/s (by product)', 'Lag p95 (h)', 'Exact', 'Note'],
      costRows, costNote);
  }

  under(...(run.tier === 'quick' ? ['visual'] : ['visual', 'catalogue']));
  const visualRows = [];
  let visualNote = null;
  const head = path.join(files.out, 'visual');
  const accepted = files.baseline && existsSync(path.join(files.baseline, 'accepted.json')) ? JSON.parse(await readFile(path.join(files.baseline, 'accepted.json'), 'utf8')) : null;
  if (!existsSync(path.join(head, 'manifest.json'))) visualRows.push({ pass: false, cells: ['–', '–', verdict(false), 'no captures were taken', ''] });
  else if (accepted && JSON.stringify(accepted.fixture ?? null) !== JSON.stringify(run.fixture ?? null))
    visualRows.push({ pass: false, cells: ['–', '–', verdict(false), `the accepted baseline is of another fixture (${accepted.fixture?.name ?? 'unknown'}), not ${run.fixture?.name}: review visual/ and accept a new one`, ''] });
  else if (!accepted) visualRows.push({ pass: false, cells: ['–', '–', verdict(false), `no accepted baseline: review visual/ and accept it with \`python tools/e2e/suite.py accept --run "${files.out}"${files.baseline ? ` --state "${path.dirname(files.baseline)}"` : ''} --reason "…"\``, ''] });
  else {
    const comparison = await compareCaptures(files.baseline, head, { diffDirectory: path.join(head, 'diff') });
    visualNote = `Baseline accepted ${accepted.acceptedAt} from main ${String(accepted.commit ?? '?').slice(0, 10)} (${accepted.run}): ${accepted.reason}. ${comparison.counts.PASS} same, ${comparison.counts.DIFF} different, ${comparison.counts.ERROR} errors.`;
    for (const result of comparison.results.filter(result => result.status !== 'PASS'))
      visualRows.push({ pass: false, cells: [result.caseId ?? result.kind, result.kind, verdict(false), why([result.reason ?? result.status]), result.diff ? `[diff](visual/diff/${result.diff})` : ''] });
  }
  section('Visual check (pages, widgets, catalogue views, open states and the map at fixed hours and cameras, against the accepted baseline)', ['Capture', 'Kind', 'Result', 'Why', 'Diff'], visualRows, visualNote ?? (visualRows.length ? null : 'All the same.'));

  under();
  const timedWindows = [
    ...area('playback').filter(r => r.from != null && r.to != null).map(r => ({ label: `${r.page} ${r.speed} h/s${r.scroll ? ` (scroll ${r.scroll})` : ''}`, from: r.from, to: r.to })),
    ...area('widget-costs').filter(r => r.from != null && r.to != null).map(r => ({ label: `cost:${r.kind}`, from: r.from, to: r.to })),
  ];
  const lost = records ? losses(records, timedWindows) : [];
  section('Measurement losses (records the perf logs dropped: any can hide a failed reply or a slow frame)', ['Source', 'Records', 'Windows', 'From', 'To'],
    lost.map(loss => ({ pass: false, cells: [loss.source, loss.count, why(loss.windows), new Date(loss.from).toISOString(), new Date(loss.to).toISOString()] })), lost.length ? null : 'None.');

  const closed = new Set((run.sessions ?? []).filter(s => s.appExit === 0).map(s => s.name.replaceAll(' ', '-')));
  const finished = new Map(sweep.meta.sessions.filter(meta => meta.finished && closed.has(meta.session))
    .map(meta => [meta.session, Date.parse(meta.finished)]));
  const replies = records ? failedReplies(records, finished) : null;
  section('Failed or refused replies', ['Product', 'Count', 'First failure', 'Hours'], replies === null
    ? [{ pass: false, cells: ['–', '–', 'no perf log: replies could not be checked', ''] }]
    : replies.failures.map(reply => ({ pass: false, cells: [reply.product, reply.count, why([reply.first]), reply.hours.join(', ')] })),
    replies && `${replies.failures.length === 0 ? 'None. ' : ''}${replies.shutdown} requests ended during app shutdown after their sweep finished.`);
  const messages = consoleFailures(sweep.console ?? []);
  const byText = groupBy(messages, message => `${message.type}|${message.text}`);
  section('Console errors and warnings', ['Type', 'Count', 'First during', 'Message'],
    [...byText].map(([, list]) => ({ pass: false, cells: [list[0].type, list.length, list[0].during, why([list[0].text])] })), byText.size ? null : 'None.');

  const failing = results.filter(result => !result.pass);
  section(`Failing steps (${failing.length})`, ['Area', 'Item', 'Step', 'ms', 'Why', 'Screenshot'],
    failing.slice(0, 400).map(result => ({ pass: false, cells: [result.area, result.item, result.label ?? '', fmt(result.ms), why(result.reasons), result.screenshot ? `[png](${result.screenshot})` : ''] })),
    failing.length > 400 ? `The first 400 of ${failing.length}; sweep.json holds them all.` : null);
}

const judged = sections.filter(entry => !entry.phases || entry.phases.some(phase => asked(run, phase)));
const failed = judged.flatMap(entry => entry.rows).filter(row => !row.pass).length;
const passed = failed === 0;
const duration = run.seconds ? `${Math.round(run.seconds / 60)} min (${Object.entries(run.phases ?? {}).map(([name, seconds]) => `${name} ${Math.round(seconds)} s`).join(', ')})` : 'unknown';
const phaseLine = phaseSeconds(sweep?.meta.sessions).map(([name, seconds]) => `${name} ${Math.round(seconds)} s`).join(', ');
const extraHeader = profileLines(run, sweep, records);
const lines = [
  `# Acceptance suite${run.tier === 'quick' ? ' (quick tier)' : ''}: ${passed ? 'PASSED' : 'FAILED'}`, '',
  ...extraHeader.filter(line => line.startsWith('> ')),
  ...(extraHeader.some(line => line.startsWith('> ')) ? [''] : []),
  `${run.commit ? `main ${run.commit.slice(0, 10)}. ` : ''}Duration ${duration}.${sweep ? ` Sweep ${Math.round(sweep.meta.seconds ?? 0)} s, viewport ${sweep.meta.viewport?.width}×${sweep.meta.viewport?.height} at ${sweep.meta.viewport?.dpr}×, display ${run.refreshHz ?? '?'} Hz.` : ''}`,
  `Phases: ${phaseLine || 'none ran'}.`,
  machineLine(run),
  ...extraHeader.filter(line => !line.startsWith('> ')),
  ...(run.shell ? [`Measuring shell ${run.shell.folder} (${run.shell.revision?.slice(0, 10) ?? 'revision unknown'}) serving the production bundle ${run.shell.ui.folder}, built ${run.shell.ui.built}, ${run.shell.ui.files} files.`] : []),
  `${failed} failing rows. Budget: ${BUDGET_MS} ms for anything a user waits on.`, '',
];
for (const entry of judged) {
  lines.push(`## ${entry.title}`, '');
  if (entry.note) lines.push(entry.note, '');
  if (entry.rows.length) lines.push(`| ${entry.columns.join(' | ')} |`, `|${entry.columns.map(() => '---').join('|')}|`, ...entry.rows.map(row => `| ${row.cells.join(' | ')} |`), '');
}
await writeFile(path.join(files.out, 'e2e.md'), lines.join('\n'));
await writeFile(path.join(files.out, 'e2e.json'), JSON.stringify({ passed, failed, run, sections: judged }, null, 1));
if (files.history && sweep) {
  const before = readHistory(await readJSON(files.history), files.history);
  const wholeCatalogue = run.tier !== 'quick' && asked(run, 'catalogue') && area('catalogue').some(result => result.label === 'add');
  await writeFile(files.history, JSON.stringify(mergeHistory(before, results, { slowest: wholeCatalogue ? slowestCards(results) : null, at: new Date().toISOString() }), null, 1));
}
console.log(`${passed ? 'passed' : 'failed'}: ${failed} failing rows; report ${path.join(files.out, 'e2e.md')}`);
process.exitCode = passed ? 0 : 1;
