import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { LogListProps } from '../../charts/log/LogList';
import type { EspionageLogReply, EspionageOperationRun, EspionageOperationEnd, OperationDescriptionRef } from '../../generated/dashboard';
import { operationAt, operationName, operationChoices, operationJumpHours, operationRows, operationTooltip as flaggedOperationTooltip, preparationRatio, executionRatio, operationProgress, operationOutcome, operationWindowIndex, operationWindowIdentity, operationWindows, windowedOperationRows, validateEspionageLogReply } from './espionageData.ts';
import { entryAt } from '../../dashboard/reads.ts';
import { contentKey } from '../../dashboard/options.ts';
import { keepWanted } from '../../dashboard/productWindows.ts';
import { countryMatch } from '../../dashboard/groups.ts';

const icon = 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png';
const run = (firstHour: number, lastHour: number, ownerTag = 'SYNTHETIC_OWNER'): EspionageOperationRun => ({ firstHour, lastHour, readHour: firstHour,
  ownerTag, targetTag: 'SYNTHETIC_TARGET', typeKey: 'mod operation/été', phaseCount: 4, autoRepeat: false, durationDays: 8, assignedSlots: 3,
  preparationRatio: 35001, elapsedDays: 2, currentPhase: 0, terminal: false, autoCommence: true, startHour: 12, operatives: ['123', '456'], operativeNames: null });
const ending = (byHour: number, lastSeen: number, reason: 'completed' | 'disappeared' = 'disappeared'): EspionageOperationEnd => ({ byHour, lastSeen, reason,
  completionHour: null, completionSequence: null, typeKey: 'mod operation/été', outcome: null, outcomeUnrecorded: 'Outcome not recorded', ownerTag: 'SYNTHETIC_OWNER', targetTag: 'SYNTHETIC_TARGET' });
const fixture = (): EspionageLogReply => ({ branchId: '1', window: 0, windows: [], firstHour: 10, lastHour: 30, available: [{ firstHour: 10, lastHour: 20 }, { firstHour: 25, lastHour: 30 }], readHours: [10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 25, 26, 27, 28, 29, 30], warnings: [], unavailable: null,
  completionsAvailable: [], completionsUnrecorded: 'Operation completion events not recorded',
  types: [{ key: 'mod operation/été', name: 'Opération des étoiles', icon, iconMissing: null }], operations: [{ definitionToken: 42, id: 'handle:10', firstRead: 10, startHour: 12, startKnownAt: 12,
    runs: [{ ...run(10, 11), durationDays: 0, startHour: null, elapsedDays: null }, run(12, 20), run(25, 30)], end: null }] });
const filter = { types: [], country: () => true, role: 'owner', from: null, live: true, runningOrder: 'start' } as const;

test('operation rows retain a fallback picture and progress while stating status once', async () => {
  const data = fixture();
  data.types[0].icon = null;
  data.types[0].iconMissing = 'Raw missing asset reason';
  data.windows = [{ firstHour: 10, lastHour: 30 }];
  const host = globalThis as typeof globalThis & { __operationLog?: LogListProps<ReturnType<typeof operationAt> & {}> };
  const replacements: Record<string, string> = {
    '/widgets/espionage-log/useEspionageLog.ts': `export const useEspionageLog = (_widget, runtime) => ({ track: runtime.tracks[0], data: ${JSON.stringify(data)}, retry() {} });
      export const useEspionageWindows = () => ({ data: new Map([[0, ${JSON.stringify(data)}]]), errors: new Map(), retry() {} });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({});',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/charts/log/LogList.tsx': `export const PreparedLogCoverage = () => null; import { createElement } from 'react'; export default props => {
      globalThis.__operationLog = props; return createElement('div', null, props.rows.map(row => createElement('div', { key: row.key }, props.renderRow(row)))); };`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'operation-row-fixture', enforce: 'pre',
      transform: (_source, id) => { const key = Object.keys(replacements).find(key => id.endsWith(key)); return key ? replacements[key] : undefined; },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/espionage-log/EspionageLogWidget.tsx');
    const props = { widget: { id: 'operations', kind: 'espionage-log', trackIds: ['main'], countries: { kind: 'all' },
      time: { mode: 'follow' }, options: { chips: false } }, runtime: { client: {}, clock: { getHour: () => 18 }, tracks: [{
        id: 'main', axisOffset: 0, branch: { id: '1', firstHour: 10, lastHour: 30 }, source: { id: 'synthetic', revision: '1' },
      }] }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const html = renderToStaticMarkup(createElement(Widget, props));
    assert.match(html, /class="art-fallback"/);
    assert.match(html, /class="progress-meter/);
    assert.equal((html.match(/running/gi) ?? []).length, 1);
    assert.doesNotMatch(html, /is-missing|>\?</);
    assert.doesNotMatch(html, /espionage-type|Raw missing asset reason/);
    const log = host.__operationLog!;
    const columns = typeof log.columns === 'function' ? log.columns(log.rows![0]) : log.columns;
    assert.equal(columns, 'var(--space-6) minmax(0, 2fr) minmax(0, 1fr)');
  } finally {
    delete host.__operationLog;
    await vite.close();
  }
});

test('a rejected operation description shows a short error and retains its detail in diagnostics', async () => {
  const data = fixture();
  data.windows = [{ firstHour: 10, lastHour: 30 }];
  data.operations[0].end = { ...ending(25, 20, 'completed'), outcomeUnrecorded: null,
    outcome: { bonus: false, detected: false, recordedLanguage: 'synthetic_language',
      descriptionRef: { hour: 25, id: '1', owner: 1, sequence: null }, descriptionUnrecorded: null, descriptionRequiredBytes: '26' } };
  const row = operationRows(data, 25, filter)[0];
  const host = globalThis as typeof globalThis & {
    __operationTooltipRow?: typeof row;
    __operationDescription?: { reference: OperationDescriptionRef; text: string; detail?: string };
    __operationDescriptionReady?: () => void;
    __operationEffects?: (() => void | (() => void))[];
    __operationLog?: LogListProps<typeof row.item>;
  };
  host.__operationTooltipRow = row;
  const replacements: Record<string, string> = {
    '/widgets/espionage-log/useEspionageLog.ts': `export const useEspionageLog = (_widget, runtime) => ({ track: runtime.tracks[0], data: ${JSON.stringify(data)}, retry() {} });
      export const useEspionageWindows = () => ({ data: new Map([[0, ${JSON.stringify(data)}]]), errors: new Map(), retry() {} });`,
    '/dashboard/operationDescription.ts': 'export const loadOperationDescription = async () => { throw new Error("Private backend failure"); };',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({});',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/charts/log/LogList.tsx': 'export const PreparedLogCoverage = () => null; export default props => { globalThis.__operationLog = props; return null; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'operation-description-fixture', enforce: 'pre',
      transform(source, id) {
        const key = Object.keys(replacements).find(key => id.endsWith(key));
        if (key) return replacements[key];
        if (id.endsWith('/espionage-log/EspionageLogWidget.tsx')) return 'const recordEffect = effect => { globalThis.__operationEffects.push(effect); };\n' + source
          .replace('useState<LogRow<OperationAt> | null>(null)', 'useState(globalThis.__operationTooltipRow)')
          .replace(/  const \[description, setDescription\] = useState[^\n]+;/,
            `  const description = globalThis.__operationDescription;
              const setDescription = value => { globalThis.__operationDescription = value; globalThis.__operationDescriptionReady?.(); };`)
          .replaceAll('useEffect(() =>', 'recordEffect(() =>');
      },
    }] });
  let dispose: void | (() => void) = undefined;
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/espionage-log/EspionageLogWidget.tsx');
    let diagnostics: readonly string[] = [];
    const props = { widget: { id: 'descriptions', kind: 'espionage-log', trackIds: ['main'], countries: { kind: 'all' },
      time: { mode: 'follow' }, options: { chips: false } }, runtime: { client: {}, clock: { getHour: () => 25 }, tracks: [{
        id: 'main', axisOffset: 0, branch: { id: '1', firstHour: 10, lastHour: 30 }, source: { id: 'synthetic', revision: '1' },
      }] }, active: true, onDiagnostics: (warnings: readonly string[]) => { diagnostics = warnings; } } as unknown as WidgetProps;
    const render = () => { host.__operationEffects = []; return renderToStaticMarkup(createElement(Widget, props)); };
    render();
    const loaded = new Promise<void>(resolve => { host.__operationDescriptionReady = resolve; });
    dispose = host.__operationEffects![0]();
    await loaded;
    render();
    const tooltip = host.__operationLog!.tooltip!(row)[0];
    assert.equal(tooltip.rows.find(item => item.label === 'Outcome')?.value, 'Couldn’t load description.');
    assert.doesNotMatch(JSON.stringify(tooltip), /Private backend failure/);
    host.__operationEffects!.at(-1)!();
    assert.ok(diagnostics.includes('Private backend failure'));
  } finally {
    dispose?.();
    for (const key of ['__operationTooltipRow', '__operationDescription', '__operationDescriptionReady', '__operationEffects', '__operationLog'] as const) delete host[key];
    await vite.close();
  }
});

const operationTooltip = (reply: EspionageLogReply, row: Parameters<typeof flaggedOperationTooltip>[1], name: (tag: string | null) => string, description?: string) =>
  flaggedOperationTooltip(reply, row, tag => ({ name: name(tag) }), description);

test('operation country rows retain dated pending and unavailable flags while native actor absence stays flagless', () => {
  const reply = fixture();
  const [row] = operationRows(reply, 18, filter);
  const lookups: (string | null)[] = [];
  const tip = flaggedOperationTooltip(reply, row, tag => {
    lookups.push(tag);
    return { name: `Name at ${row.hour}: ${tag}`, flagPending: tag === row.item.ownerTag,
      flagReason: tag === row.item.targetTag ? 'Recorded target flag unavailable' : undefined };
  })[0];
  assert.deepEqual(tip.rows.find(row => row.label === 'Owner')?.countryFlag, { flag: undefined, flagPending: true, flagReason: undefined });
  assert.deepEqual(tip.rows.find(row => row.label === 'Target')?.countryFlag, { flag: undefined, flagPending: false, flagReason: 'Recorded target flag unavailable' });
  assert.deepEqual(lookups, [row.item.ownerTag, row.item.targetTag]);
  assert.equal(tip.icon, icon);
  assert.ok(tip.rows.filter(row => !['Owner', 'Target'].includes(row.label)).every(row => row.countryFlag === undefined));
  const absent = { ...row, item: { ...row.item, ownerTag: null, targetTag: null } };
  const missing = flaggedOperationTooltip(reply, absent, () => ({ name: 'Unknown country', flag: 'must-not-use', flagReason: 'not an actor' }))[0];
  assert.ok(missing.rows.slice(0, 2).every(row => row.countryFlag === undefined));
});

test('mod operation names and art survive content choices and tooltips without a vanilla catalogue', () => {
  const reply = validateEspionageLogReply(fixture());
  const choices = operationChoices(reply);
  assert.equal(choices[0].label, 'Opération des étoiles');
  assert.equal(choices[0].icon, icon);
  assert.equal(contentKey(choices[0].value), 'mod operation/été');
  const row = operationRows(reply, 18, filter)[0];
  const tip = operationTooltip(reply, row, tag => tag ?? 'Unknown')[0];
  assert.equal(tip.title, 'Opération des étoiles');
  assert.equal(tip.icon, icon);
  assert.equal(tip.rows.find(row => row.label === 'Preparation')?.value, '35%');
  assert.ok(tip.rows.some(row => row.value === 'Name unavailable'));
  assert.equal(tip.rows.find(row => row.label === 'Outcome'), undefined);
  assert.equal(tip.rows.find(row => row.label === 'Execution progress')?.value, '2 / 8 days');
  assert.equal(executionRatio(row.item), .25);
  assert.equal(operationProgress(row.item), .25);
  assert.equal(preparationRatio(row.item), .35001);
  const malformedArt = fixture(); malformedArt.types[0].icon = 'file://unsupported-art';
  const refused = validateEspionageLogReply(malformedArt);
  assert.equal(refused.types[0].icon, null);
  assert.equal(refused.types[0].name, 'Opération des étoiles');
  assert.match(refused.warnings.at(-1)!, /mod operation\/été.*\(1\)/);
  const unnamed = fixture(); unnamed.types[0].name = null;
  assert.equal(operationChoices(unnamed)[0].label, 'Operation name unavailable');
});

test('preparation and launch remain one operation without leaking its future start', () => {
  const reply = validateEspionageLogReply(fixture());
  const early = operationAt(reply, reply.operations[0], 11)!;
  assert.equal(early.startHour, null);
  assert.equal(early.state, 'preparing');
  assert.deepEqual(operationJumpHours(early, 11), [10]);
  assert.deepEqual(operationJumpHours(early, 11, 11), [11]);
  assert.equal(operationProgress(early), .35001);
  const launched = operationRows(reply, 18, filter);
  assert.equal(launched.length, 1);
  assert.equal(launched[0].id, operationRows(reply, 11, filter)[0].id);
  assert.equal(launched[0].item.state, 'running');
  assert.equal(launched[0].item.startHour, 12);
  assert.equal(launched[0].item.operation.end, null);
});

test('missing operation reads do not carry progress or invent endings', () => {
  const reply = validateEspionageLogReply(fixture());
  const missing = operationAt(reply, reply.operations[0], 24)!;
  assert.equal(missing.state, 'unread');
  assert.equal(missing.run, null);
  assert.equal(missing.readHour, null);
  assert.equal(preparationRatio(missing), null);
  assert.equal(operationAt(reply, reply.operations[0], 25)!.state, 'running');
  assert.equal(operationAt(reply, reply.operations[0], 9), null);
  const crossed = fixture(); crossed.operations[0].runs[1].lastHour = 24;
  assert.throws(() => validateEspionageLogReply(crossed), /unobserved/);
});

test('repeat, phases, duration and slot changes are dated values in one operation', () => {
  const reply = fixture();
  reply.readHours = [10, 12, 16, 25];
  reply.operations[0].runs = [{ ...run(10, 11), durationDays: 0, startHour: null }, run(12, 15),
    { ...run(16, 20), autoRepeat: true, phaseCount: 6, currentPhase: 1, durationDays: 9, assignedSlots: 5 }, run(25, 30)];
  validateEspionageLogReply(reply);
  const before = operationRows(reply, 15, filter);
  const after = operationRows(reply, 20, filter);
  assert.equal(before.length, 1); assert.equal(after.length, 1);
  assert.equal(before[0].id, after[0].id);
  assert.equal(before[0].hour, 12); assert.equal(after[0].hour, 16);
  const values = (row: typeof before[0]) => operationTooltip(reply, row, tag => tag ?? 'Unknown')[0].rows;
  const labels = ['Auto-repeat', 'Phases', 'Execution progress', 'Operatives'];
  assert.deepEqual(labels.map(label => values(before[0]).find(row => row.label === label)?.value), ['Disabled', 'Phase 1 of 4', '2 / 8 days', '2 / 3']);
  assert.deepEqual(labels.map(label => values(after[0]).find(row => row.label === label)?.value), ['Enabled', 'Phase 2 of 6', '2 / 9 days', '2 / 5']);
  assert.equal(after[0].item.operation.end, null);
});

test('a failed Countries or CountryLinks read leaves one running row with unknown countries', () => {
  const reply = fixture();
  reply.operations[0].runs = [run(10, 15), { ...run(16, 18), ownerTag: null, targetTag: null }, run(19, 20), run(25, 30)];
  validateEspionageLogReply(reply);
  const all = { ...filter, includeUnknownCountries: true };
  const before = operationRows(reply, 15, all);
  const missing = operationRows(reply, 18, all);
  const resumed = operationRows(reply, 19, all);
  assert.equal(missing.length, 1);
  assert.equal(missing[0].id, before[0].id); assert.equal(resumed[0].id, before[0].id);
  assert.equal(missing[0].item.state, 'running');
  assert.equal(missing[0].item.ownerTag, null); assert.equal(missing[0].item.targetTag, null);
  assert.equal(missing[0].item.operation.end, null);
  assert.deepEqual(operationTooltip(reply, missing[0], tag => tag ?? 'Unknown country')[0].rows.slice(0, 2).map(row => row.value), ['Unknown country', 'Unknown country']);
  assert.equal(resumed[0].item.ownerTag, 'SYNTHETIC_OWNER');
  assert.equal(operationRows(reply, 18, filter).length, 0);
});

test('a later read can establish the same operation start without backfilling earlier rows', () => {
  const reply = fixture();
  reply.operations[0].startKnownAt = 16;
  reply.operations[0].runs = [{ ...run(10, 11), durationDays: 0, startHour: null }, { ...run(12, 15), startHour: null }, run(16, 20), run(25, 30)];
  validateEspionageLogReply(reply);
  const before = operationRows(reply, 15, filter)[0];
  const known = operationRows(reply, 16, filter)[0];
  assert.equal(before.item.startHour, null);
  assert.equal(operationTooltip(reply, before, tag => tag ?? 'Unknown')[0].rows.find(row => row.label === 'Execution started')?.value, 'Not recorded');
  assert.equal(known.id, before.id);
  assert.equal(known.item.startHour, 12);
  assert.deepEqual(operationJumpHours(known.item, 16), [12]);
  assert.equal(operationTooltip(reply, known, tag => tag ?? 'Unknown')[0].rows.find(row => row.label === 'Execution started')?.value, 'Hour 12');
  assert.equal(known.item.operation.end, null);
});

test('a missing definition name is a recording gap and later lexical data is never backfilled', () => {
  const reply = fixture();
  reply.operations[0].runs[0].typeKey = null;
  validateEspionageLogReply(reply);
  const early = operationRows(reply, 11, filter)[0];
  const known = operationRows(reply, 12, filter)[0];
  assert.equal(early.item.typeKey, null);
  assert.equal(operationTooltip(reply, early, tag => tag ?? 'Unknown')[0].title, 'Operation name unavailable');
  assert.equal(operationTooltip(reply, known, tag => tag ?? 'Unknown')[0].title, 'Opération des étoiles');
  assert.equal(known.id, early.id);
  assert.ok(reply.types.every(type => !type.key.startsWith('?')));
  assert.deepEqual(operationRows(reply, 11, { ...filter, types: ['mod operation/été'] }), []);
});

test('operative slots include unassigned slots while missing assigned IDs never become zero', () => {
  const reply = fixture();
  const values = () => operationTooltip(reply, operationRows(reply, 18, filter)[0], tag => tag ?? 'Unknown')[0].rows;
  assert.equal(values().find(row => row.label === 'Operatives')?.value, '2 / 3');
  assert.equal(values().find(row => row.label === 'Assigned operatives'), undefined);
  assert.deepEqual(values().filter(row => row.label === 'Operative').map(row => row.value), ['Name unavailable', 'Name unavailable']);
  assert.ok(!values().some(row => /^Operative [0-9]/.test(row.label)));
  reply.operations[0].runs[1].operatives = [];
  assert.equal(values().find(row => row.label === 'Operatives')?.value, '0 / 3');
  reply.operations[0].runs[1].operatives = null;
  assert.equal(values().find(row => row.label === 'Operatives')?.value, 'Unknown / 3');
  assert.equal(values().filter(row => /^Operative \d/.test(row.label ?? '')).length, 0);
});

test('daily carried operation values and gap rows retain the actual supplying read date', () => {
  const reply = fixture();
  reply.lastHour = 50;
  reply.available = [{ firstHour: 10, lastHour: 20 }, { firstHour: 35, lastHour: 50 }];
  reply.readHours = [10, 35];
  reply.operations[0].runs = [run(10, 20), run(35, 50)];
  const carried = operationRows(reply, 18, filter)[0];
  assert.equal(carried.hour, 10);
  assert.equal(carried.item.readHour, 10);
  assert.equal(carried.item.run?.currentPhase, 0);
  assert.equal(carried.item.run?.preparationRatio, 35001);
  const gap = operationRows(reply, 25, filter)[0];
  assert.equal(gap.hour, 10);
  assert.equal(gap.item.readHour, null);
  assert.equal(gap.item.run, null);
  assert.equal(preparationRatio(gap.item), null);
  const resumed = operationRows(reply, 40, filter)[0];
  assert.equal(resumed.hour, 35);
  assert.equal(resumed.item.readHour, 35);
});

test('Live prioritises current operations; disappearance is dated by its confirming read with unknown outcome', () => {
  const reply = fixture();
  reply.operations.push({ definitionToken: 42, id: 'ended:10', firstRead: 10, startHour: 10, startKnownAt: 10, runs: [run(10, 15)], end: ending(16, 15) });
  const live = operationRows(reply, 18, filter);
  assert.deepEqual(live.map(row => row.id), ['handle:10', 'ended:10']);
  const done = live[1];
  assert.equal(done.hour, 16); assert.equal(done.by, true);
  assert.equal(done.item.state, 'disappeared');
  assert.deepEqual(operationJumpHours(done.item, 18), [10, 16]);
  assert.equal(operationTooltip(reply, done, tag => tag ?? 'Unknown')[0].rows.find(row => row.label === 'Outcome')?.value, 'Outcome not recorded.');
});

test('row dates resolve changing country tags and names within the same operation', () => {
  const reply = fixture();
  reply.operations[0].runs = [run(10, 15, 'OLD_TAG'), run(16, 20, 'NEW_TAG')];
  const names = [{ firstHour: 10, value: 'Old country name' }, { firstHour: 16, value: 'Renamed country' }];
  const old = operationRows(reply, 14, filter)[0];
  const renamed = operationRows(reply, 18, filter)[0];
  assert.equal(old.hour, 14); assert.equal(old.item.ownerTag, 'OLD_TAG');
  assert.equal(renamed.hour, 18); assert.equal(renamed.item.ownerTag, 'NEW_TAG');
  assert.equal(old.id, renamed.id);
  const ownerAt = (row: typeof old) => operationTooltip(reply, row, tag => tag === row.item.ownerTag ? entryAt(names, row.hour)!.value : 'Target')[0].rows[0].value;
  assert.equal(ownerAt(old), 'Old country name');
  assert.equal(ownerAt(renamed), 'Renamed country');
  reply.operations[0].end = { ...ending(25, 20), ownerTag: 'END_TAG' };
  assert.equal(operationRows(reply, 25, filter)[0].item.ownerTag, 'END_TAG');
});

test('operation log retains all 5000 countries and never applies a player-status filter', () => {
  const reply = fixture();
  reply.operations = Array.from({ length: 5000 }, (_, i) => ({ definitionToken: 42, id: `synthetic-${i}`, firstRead: 10, startHour: 12, startKnownAt: 12,
    runs: [run(10, 20, `Country_${i}`)], end: null }));
  const rows = operationRows(validateEspionageLogReply(reply), 18, filter);
  assert.equal(rows.length, 5000);
  assert.equal(new Set(rows.map(row => row.item.ownerTag)).size, 5000);
  assert.equal(operationRows(reply, 18, { ...filter, country: tag => tag === 'Country_4999' }).length, 1);
  reply.operations[0].runs[0].ownerTag = null;
  assert.equal(operationRows(reply, 18, { ...filter, includeUnknownCountries: true }).length, 5000);
});

test('execution uses the supplying read and native sentinels stay unknown', () => {
  const reply = fixture();
  reply.readHours = [10, 25];
  reply.operations[0].runs = [run(10, 20), { ...run(25, 30), elapsedDays: null }];
  assert.equal(operationProgress(operationAt(reply, reply.operations[0], 15)!), .25);
  assert.equal(operationProgress(operationAt(reply, reply.operations[0], 20)!), .25);
  assert.equal(operationProgress(operationAt(reply, reply.operations[0], 24)!), null);
  assert.equal(operationProgress(operationAt(reply, reply.operations[0], 25)!), null);
  assert.equal(operationTooltip(reply, operationRows(reply, 25, filter)[0], tag => tag ?? 'Unknown')[0].rows.find(row => row.label === 'Execution progress')?.value, 'Not recorded');
});

test('one end-outcome adapter shows stored text or an availability status only after the end', () => {
  const reply = fixture();
  reply.operations[0].end = ending(25, 20, 'completed');
  assert.equal(operationOutcome(operationAt(reply, reply.operations[0], 20)!), null);
  assert.equal(operationOutcome(operationAt(reply, reply.operations[0], 25)!), 'Outcome not recorded.');
  reply.operations[0].id = '1:10';
  reply.operations[0].end.outcome = { bonus: false, detected: true, recordedLanguage: 'synthetic_language', descriptionRef: { hour: 25, id: '1', owner: 1, sequence: null }, descriptionUnrecorded: null, descriptionRequiredBytes: '26' };
  reply.operations[0].end.outcomeUnrecorded = null;
  assert.equal(operationOutcome(operationAt(validateEspionageLogReply(reply), reply.operations[0], 25)!), 'Outcome');
  const row = operationRows(reply, 25, filter)[0];
  assert.equal(operationTooltip(reply, row, tag => tag ?? 'Unknown', 'Synthetic captured outcome')[0].rows.find(row => row.label === 'Outcome')?.value, 'Synthetic captured outcome');
  assert.equal(operationTooltip(reply, row, tag => tag ?? 'Unknown')[0].rows.find(row => row.label === 'Recorded description language')?.value, undefined);
  const malformed = fixture(); malformed.operations[0].runs[0].preparationRatio = 100001;
  assert.throws(() => validateEspionageLogReply(malformed), /malformed/);
});

test('completion-only operations retain the exact date, sequence and stored flags when text is refused', () => {
  const reply = fixture();
  reply.completionsAvailable = [{ firstHour: 25, lastHour: 30 }];
  reply.completionsUnrecorded = null;
  reply.operations = [{ definitionToken: 42, id: 'completion:18446744073709551615', firstRead: 25, startHour: null, startKnownAt: null, runs: [],
    end: { ...ending(25, 25, 'completed'), completionHour: 22, completionSequence: '18446744073709551615', outcomeUnrecorded: null,
      outcome: { bonus: true, detected: false, recordedLanguage: 'synthetic_language', descriptionRef: null, descriptionUnrecorded: 'Stored effect description refused: acquisition budget exceeded', descriptionRequiredBytes: '18446744073709551615' } } }];
  const recorded = validateEspionageLogReply(reply);
  assert.equal(operationRows(recorded, 24, filter).length, 0);
  const rows = operationRows(recorded, 25, filter);
  assert.equal(rows.length, 1);
  const row = rows[0];
  assert.equal(row.hour, 22); assert.equal(row.by, false);
  assert.equal(row.item.run, null); assert.equal(row.item.state, 'completed');
  assert.equal(row.item.operation.end?.completionSequence, '18446744073709551615');
  assert.equal(row.item.operation.end?.outcome?.descriptionRequiredBytes, '18446744073709551615');
  assert.deepEqual(operationJumpHours(row.item, 25), [22]);
  const tooltip = operationTooltip(recorded, row, tag => tag ?? 'Unknown')[0];
  assert.equal(tooltip.rows.find(row => row.label === 'Completed')?.value, 'Hour 22');
  assert.equal(tooltip.rows.find(row => row.label === 'Outcome')?.value, 'Description unavailable.');
  assert.equal(tooltip.rows.find(row => row.label === 'Recorded description language')?.value, undefined);
  assert.equal(tooltip.rows.find(row => row.label === 'Bonus result')?.value, 'Yes');
  assert.equal(tooltip.rows.find(row => row.label === 'Detected')?.value, 'No');
  assert.equal(tooltip.note, undefined);
  assert.equal(operationRows(recorded, 25, { ...filter, from: 23 }).length, 0);
  reply.operations[0].end!.typeKey = null;
  assert.equal(operationTooltip(validateEspionageLogReply(reply), operationRows(reply, 25, filter)[0], tag => tag ?? 'Unknown')[0].title, 'Operation name unavailable');
  reply.completionsAvailable = [];
  assert.throws(() => validateEspionageLogReply(reply), /unrecorded completion read/);
  reply.completionsAvailable = [{ firstHour: 25, lastHour: 30 }];
  reply.operations[0].end!.completionSequence = '18446744073709551616';
  assert.throws(() => validateEspionageLogReply(reply), /malformed/);
});

test('completion holder matching uses endpoint supply while event range and Jump retain the actual date', () => {
  const reply = fixture();
  reply.operations = [{ definitionToken: 42, id: 'opaque completion', firstRead: 25, startHour: null, startKnownAt: null, runs: [],
    end: { ...ending(25, 25, 'completed'), completionHour: 22, completionSequence: '1', ownerTag: 'Y', targetTag: 'TARGET' } }];
  const groups = { countryIdentities: [
    { tag: 'X', identities: [{ firstHour: 10, lastHour: 23, slot: 1, countryId: 7 }] },
    { tag: 'Y', identities: [{ firstHour: 10, lastHour: 23, slot: 3, countryId: 11 }, { firstHour: 24, lastHour: 30, slot: 1, countryId: 7 }] },
    { tag: 'TARGET', identities: [{ firstHour: 10, lastHour: 30, slot: 2, countryId: 9 }] },
  ] } as import('../../generated/dashboard').CountryGroupReply;
  const owner = countryMatch({ kind: 'tags', tags: ['X'] }, [], groups, 30);
  assert.equal(owner.matches('Y', 22), false);
  assert.equal(operationRows(reply, 24, { ...filter, country: owner.matches }).length, 0);
  const [row] = operationRows(reply, 25, { ...filter, country: owner.matches });
  assert.equal(row.hour, 22);
  assert.equal(row.item.countryHour, 25);
  assert.deepEqual(operationJumpHours(row.item, 25), [22]);
  assert.equal(operationRows(reply, 25, { ...filter, country: owner.matches, from: 23 }).length, 0);
  assert.equal(operationRows(reply, 25, { ...filter, country: owner.matches, role: 'target' }).length, 0);
  assert.equal(operationRows(reply, 25, { ...filter, country: owner.matches, role: 'either' }).length, 1);
});

test('held operation endpoint tags remain attached to their supplying read across a rename and reuse', () => {
  const reply = fixture();
  reply.readHours = [10];
  reply.operations[0].runs = [{ ...run(10, 20, 'X'), readHour: 10 }];
  const groups = { countryIdentities: [
    { tag: 'X', identities: [{ firstHour: 10, lastHour: 12, slot: 1, countryId: 7 }, { firstHour: 13, lastHour: 20, slot: 2, countryId: 9 }] },
    { tag: 'LATER', identities: [{ firstHour: 13, lastHour: 20, slot: 1, countryId: 7 }] },
  ] } as import('../../generated/dashboard').CountryGroupReply;
  const match = countryMatch({ kind: 'tags', tags: ['LATER'] }, [], groups, 20);
  assert.equal(match.matches('X', 18), false);
  const [row] = operationRows(reply, 18, { ...filter, country: match.matches });
  assert.equal(row.item.countryHour, 10);
  assert.equal(row.hour, 10);
});


test('paged recorded descriptions stay on their ending read and cannot leak into earlier playback', () => {
  const reply = fixture(); const operation = reply.operations[0]; operation.id = '1:10';
  operation.end = { ...ending(25, 20, 'completed'), outcomeUnrecorded: null,
    outcome: { bonus: true, detected: false, recordedLanguage: 'synthetic_language',
      descriptionRef: { hour: 25, id: '1', owner: 1, sequence: null }, descriptionUnrecorded: null,
      descriptionRequiredBytes: '40000' } };
  const checked = validateEspionageLogReply(reply);
  const before = operationRows(checked, 18, filter)[0];
  assert.equal(operationTooltip(checked, before, () => 'Synthetic', 'Future text')[0].rows.find(row => row.label === 'Outcome'), undefined);
  const ended = operationRows(checked, 30, filter)[0];
  assert.equal(operationOutcome(ended.item), 'Outcome');
  assert.equal(operationTooltip(checked, ended, () => 'Synthetic', 'Exact client text')[0].rows.find(row => row.label === 'Outcome')?.value, 'Exact client text');
  operation.end.outcome!.descriptionRef!.hour = 26;
  assert.throws(() => validateEspionageLogReply(reply), /ending read/);
});

test('assigned names and empty native names retain their exact slot order', () => {
  const reply = fixture(); reply.operations[0].runs[1].operativeNames = [
    { id: '123', name: 'Authored operative', unavailable: null }, { id: '456', name: '', unavailable: null },
  ];
  const checked = validateEspionageLogReply(reply); const row = operationRows(checked, 18, filter)[0];
  const tooltip = operationTooltip(checked, row, () => 'Synthetic')[0];
  assert.equal(tooltip.rows.find(row => row.label === 'Operatives')?.value, '2 / 3');
  assert.equal(tooltip.rows.find(row => row.label === 'Assigned operatives'), undefined);
  const names = tooltip.rows.filter(row => row.label === 'Operative');
  assert.deepEqual(names.map(row => row.label), ['Operative', 'Operative']);
  assert.deepEqual(names.map(row => row.value), ['Authored operative', 'Unnamed operative']);
  reply.operations[0].runs[1].operativeNames![0].id = '456';
  assert.throws(() => validateEspionageLogReply(reply), /malformed/);
});


test('metadata windows retain dated anchors and merge episodes independently of reply order', () => {
  const early = fixture(); early.firstHour = 10; early.lastHour = 20; early.available = [{ firstHour: 10, lastHour: 20 }];
  early.readHours = [10, 16]; early.operations[0].runs = [run(10, 15), run(16, 20)];
  early.operations[0].end = null;
  const late = fixture(); late.window = 1; late.firstHour = 25; late.lastHour = 30;
  late.available = [{ firstHour: 16, lastHour: 16 }, { firstHour: 25, lastHour: 30 }];
  late.readHours = [16, 25];
  late.operations[0].runs = [{ ...run(16, 16), readHour: 16 }, run(25, 30)];
  late.operations[0].end = ending(25, 16);
  validateEspionageLogReply(late);
  const windows = new Map([[1, late], [0, early]]);
  const ended = windowedOperationRows(windows, 30, filter);
  assert.equal(ended.length, 1); assert.equal(ended[0].item.state, 'disappeared');
  assert.equal(ended[0].item.readHour, 16); assert.equal(ended[0].item.windowIndex, 1);
  assert.equal(ended[0].item.operation.firstRead, 10);
  const before = windowedOperationRows(windows, 18, filter);
  assert.equal(before.length, 1); assert.equal(before[0].item.state, 'running');
  assert.equal(before[0].item.windowIndex, 0);
  assert.equal(windowedOperationRows(new Map([[0, early]]), 30, filter).length, 0);
});

test('manifest and viewport demand stay small through a long campaign', () => {
  const manifest = { ...fixture(), window: null, operations: [], readHours: [],
    windows: Array.from({ length: 5000 }, (_, index) => ({ firstHour: 10 + 24 * index, lastHour: 33 + 24 * index })) };
  manifest.lastHour = manifest.windows.at(-1)!.lastHour;
  validateEspionageLogReply(manifest);
  const at = manifest.windows[4998].lastHour;
  assert.equal(operationWindowIndex(manifest.windows, at), 4998);
  assert.deepEqual(operationWindows(manifest, at, true), [4997, 4998, 4999]);
  assert.deepEqual(operationWindows(manifest, at, false, [3]), [2, 3, 4998]);
  assert.deepEqual(operationWindows(manifest, at, false, [], manifest.windows[4998].firstHour), [4998]);
  assert.throws(() => validateEspionageLogReply({ ...manifest, operations: fixture().operations }), /manifest contains detail/);
  assert.throws(() => validateEspionageLogReply({ ...fixture(), windows: manifest.windows }), /detail contains a manifest/);
});


test('known empty operation names remain empty in rows, choices and tooltips', () => {
  const reply = fixture(); reply.types[0].name = '';
  const checked = validateEspionageLogReply(reply);
  assert.equal(operationName(checked.types[0]), '');
  assert.equal(operationChoices(checked)[0].label, '');
  assert.equal(operationTooltip(checked, operationRows(checked, 18, filter)[0], () => 'Synthetic')[0].title, '');
  reply.types[0].name = null;
  assert.equal(operationName(reply.types[0]), 'Operation name unavailable');
  assert.equal(operationName(undefined), 'Operation name unavailable');
});

test('ending values use the actual supplying read before a clipped daily window', () => {
  const reply = fixture(); reply.window = 1; reply.firstHour = 124; reply.lastHour = 147;
  reply.available = [{ firstHour: 112, lastHour: 112 }, { firstHour: 124, lastHour: 136 }];
  reply.readHours = [112, 136];
  const measured = { ...run(112, 112), preparationRatio: 25000, durationDays: 0, startHour: null,
    elapsedDays: null, phaseCount: 3, currentPhase: null, assignedSlots: 1, operatives: ['77'],
    operativeNames: [{ id: '77', name: 'Recorded operative', unavailable: null }] };
  reply.operations = [{ ...reply.operations[0], firstRead: 112, startHour: null, startKnownAt: null,
    runs: [measured, { ...measured, firstHour: 124, lastHour: 135 }], end: ending(136, 112) }];
  const checked = validateEspionageLogReply(reply);
  const rows = windowedOperationRows(new Map([[1, checked]]), 147, filter);
  assert.equal(rows.length, 1); assert.equal(rows[0].item.readHour, 112);
  assert.equal(rows[0].item.run?.preparationRatio, 25000);
  const tooltip = operationTooltip(checked, rows[0], () => 'Synthetic')[0];
  assert.equal(tooltip.rows.find(row => row.label === 'Preparation')?.value, '25%');
  assert.equal(tooltip.rows.find(row => row.label === 'Phases')?.value, '3 phases; current phase unknown');
  assert.equal(tooltip.rows.find(row => row.label === 'Operatives')?.value, '1 / 1');
  assert.equal(tooltip.rows.find(row => row.label === 'Assigned operatives'), undefined);
  assert.deepEqual(tooltip.rows.filter(row => row.label === 'Operative').map(row => [row.label, row.value]), [['Operative', 'Recorded operative']]);
  assert.equal(tooltip.note, undefined);
  // Missing original evidence remains missing; nearby clipped values are not a substitute.
  reply.operations[0].runs.shift();
  const unknown = operationRows(validateEspionageLogReply(reply), 147, filter)[0];
  assert.equal(unknown.item.run, null); assert.equal(unknown.item.readHour, null);
  assert.equal(operationTooltip(reply, unknown, () => 'Synthetic')[0].rows.find(row => row.label === 'Preparation'), undefined);
});

test('a content revision releases held names for the same requested metadata window', () => {
  const identity = operationWindowIdentity('source', 'revision', 'branch', 1);
  const held = { identity, data: new Map([[3, 'Old localized name']]), errors: new Map<number, string>() };
  assert.equal(keepWanted(held, identity, new Set([3])), held);
  const refreshed = keepWanted(held, operationWindowIdentity('source', 'revision', 'branch', 2), new Set([3]));
  assert.equal(refreshed.data.size, 0); assert.equal(refreshed.errors.size, 0);
  assert.notEqual(refreshed.identity, held.identity);
});
