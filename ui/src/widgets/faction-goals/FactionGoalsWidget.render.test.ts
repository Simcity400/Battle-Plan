import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import type { FactionGoalDefinition, FactionGoalsReply } from '../../generated/dashboard';
import { mountModule } from '../../dashboard/pendingGroups.fixture.ts';
import { RefusedError } from '../../dashboard/queryError.ts';

const definition = (token: string): FactionGoalDefinition => ({ progress: [], sections: [], bands: [], conditions: [], rewards: [], flags: [], token,
  name: `Mod ${token}`, description: null, icon: null, category: 'mod_term', categoryName: 'A mod term', reason: null });
/** Lists from 10; unreadable lists from 100; absent from 150; still being prepared after 180. */
const prepared = (pendingFrom: number): FactionGoalsReply => ({
  runs: [{ firstHour: 10, lastHour: 99, list: 0, startsSegment: false }, { firstHour: 100, lastHour: 149, list: 1, startsSegment: false },
    { firstHour: 150, lastHour: Math.min(pendingFrom - 1, 180), list: 2, startsSegment: false }],
  lists: [{ absent: false, reason: null, goals: [{ ordinal: 0, definition: 0, status: 0 }], completed: [], cancelled: [] },
    { absent: false, reason: 'Faction goal status is absent', goals: [], completed: [], cancelled: [] },
    { absent: true, reason: null, goals: [], completed: [], cancelled: [] }],
  definitions: [definition('mod_goal')],
  progress: [{ ordinal: 0, runs: [{ firstHour: 10, lastHour: 49, progressRaw: '25000', progressPercent: 25, startsSegment: false },
    { firstHour: 50, lastHour: 99, progressRaw: '75000', progressPercent: 75, startsSegment: false }] }],
  unobserved: [], pending: pendingFrom <= 200 ? [{ firstHour: pendingFrom, lastHour: 200 }] : [],
  coverage: { schema: 34, firstHour: 0, lastHour: Math.min(pendingFrom - 1, 200), complete: pendingFrom > 200 }, channels: [], warnings: [],
});
type Part = { firstHour: number | null; lastHour: number | null; data?: FactionGoalsReply; error?: Error };
type State = { hour: number; faction: string; parts: Record<string, Part[]>; error?: Error; details: [string, number][]; rows: unknown[];
  retries: number; held: unknown[]; diagnostics: unknown[]; states: { kind: string; failure?: Error; onRetry?: () => void }[];
  detailError?: Error };

async function mount() {
  const host = globalThis as typeof globalThis & { __goals?: State };
  const state: State = { hour: 20, faction: '9', parts: { 9: [{ firstHour: null, lastHour: null, data: prepared(181) }] }, details: [], rows: [],
    retries: 0, held: [], diagnostics: [], states: [] };
  host.__goals = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/faction-goals/FactionGoalsWidget.tsx', {
    '/dashboard/factionChoice.ts': `export const useChosenFaction = () => ({ groups: { data: {} }, pick: { faction: { id: globalThis.__goals.faction, name: 'Mod pact',
        names: [{ firstHour: 0, lastHour: 200, name: 'Mod pact', reason: null }] }, reason: null } });
      export const useFactionDetails = (_runtime, _track, faction, hour) => { const goals = globalThis.__goals; if (!faction) return { retry() {} };
        goals.details.push([faction, hour]);
        return goals.detailError ? { error: goals.detailError, retry() {} }
          : { data: { goals: [], completed: [${JSON.stringify(definition('detail_goal'))}], cancelled: [], goalReason: null, warnings: [], refusals: [] }, retry() {} }; };
      export const factionNameAt = faction => faction.name; export const noFactionMessage = () => 'none';`,
    '/widgets/faction-goals/goalHistory.ts': `export const partAt = (parts, hour) => parts.find(part => (part.firstHour === null || part.firstHour <= hour) && (part.lastHour === null || hour <= part.lastHour));
      export const useGoalHistory = (_client, _track, faction) => { const goals = globalThis.__goals;
        return { parts: goals.parts[faction] ?? [{ firstHour: null, lastHour: null }], error: goals.error, retry() { goals.retries++; } }; };`,
    '/dashboard/heldFailures.ts': 'export const useHeldFailures = (errors, retry) => { globalThis.__goals.held.push([errors, retry]); return false; };',
    '/dashboard/useDiagnostics.ts': 'export const useDiagnostics = (_report, diagnostics) => { globalThis.__goals.diagnostics.push(diagnostics); };',
    '/components/WidgetState.tsx': `export default props => { globalThis.__goals.states.push(props); return null; };`,
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => globalThis.__goals.hour;',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({}); export const useViewportHeight = () => 1000;',
    '/charts/table/Table.tsx': 'export default props => { globalThis.__goals.rows.push(props.rows.map(row => [row.name, row.progressRaw])); return null; };',
  });
  const props = { widget: { id: 'goals', kind: 'faction-goals', trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'follow' }, options: {} },
    runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [], campaignKey: 'c' } }, branch: { id: '1', firstHour: 0, lastHour: 200 } }],
      client: {}, clock: { getHour: () => state.hour } }, active: true, headerSlot: null, setOption() {}, onDiagnostics() {} } as unknown as WidgetProps;
  const render = (hour: number) => {
    state.hour = hour; state.rows = []; state.details = []; state.states = []; state.held = []; state.diagnostics = [];
    renderToStaticMarkup(createElement(module.default, props));
    return state;
  };
  return { state, render, close: async () => { delete host.__goals; await close(); } };
}

test('prepared hours are looked up locally and send no per-hour request', async () => {
  const { render, close } = await mount();
  try {
    assert.deepEqual(render(20).rows, [[['Mod mod_goal', '25000']]]);
    assert.deepEqual(render(80).rows, [[['Mod mod_goal', '75000']]]);
    assert.deepEqual(render(80).details, []);
    // Unreadable lists keep the goal-reason state, answered locally too.
    const unreadable = render(120);
    assert.deepEqual(unreadable.details, []);
    assert.equal(unreadable.states.at(-1)?.kind, 'unavailable');
  } finally { await close(); }
});

test('an hour before the first run, or still being prepared, asks its own details; once prepared it no longer does', async () => {
  const { state, render, close } = await mount();
  try {
    // Unrecorded: the hour's details answer it, including their own failure.
    state.detailError = new Error('Faction details unavailable at this hour');
    const before = render(5);
    assert.deepEqual(before.details, [['9', 5]]);
    assert.equal(before.states.at(-1)?.kind, 'error');
    assert.equal(before.states.at(-1)?.failure, state.detailError);
    assert.equal(before.states.at(-1)?.failure?.message, 'Faction details unavailable at this hour');
    delete state.detailError;
    const pending = render(190);
    assert.deepEqual(pending.details, [['9', 190]]);
    assert.deepEqual(pending.rows, [[['Mod detail_goal', null]]]);
    // The next publish prepares the hour: the history answers it and the request is dropped.
    state.parts['9'] = [{ firstHour: null, lastHour: null, data: { ...prepared(201), runs: [...prepared(201).runs.slice(0, 2),
      { firstHour: 150, lastHour: 200, list: 0, startsSegment: false }] } }];
    const ready = render(190);
    assert.deepEqual(ready.details, []);
    assert.deepEqual(ready.rows, [[['Mod mod_goal', null]]]);
  } finally { await close(); }
});

test('a proven absence is an error whose retry asks only that hour, never the whole history', async () => {
  const { state, render, close } = await mount();
  try {
    const absent = render(160);
    const shown = absent.states.at(-1)!;
    assert.equal(shown.kind, 'error');
    assert.equal(shown.failure?.name, 'RefusedError');
    assert.equal((shown.failure as RefusedError).text, 'Faction is absent at this hour');
    assert.equal(shown.failure?.message, 'Faction is absent at this hour');
    assert.deepEqual(absent.details, []);
    shown.onRetry!();
    assert.equal(state.retries, 0, 'the history is not asked again');
  } finally { await close(); }
});

test('a failed history range falls back to each hour, names the failure and offers the history retry', async () => {
  const { state, render, close } = await mount();
  try {
    state.parts['9'] = [{ firstHour: null, lastHour: 99, data: prepared(181) }, { firstHour: 100, lastHour: null, error: new Error('Background preparation is off.') }];
    state.error = new Error('Background preparation is off.');
    const failed = render(120);
    assert.deepEqual(failed.details, [['9', 120]]);
    assert.deepEqual(failed.rows, [[['Mod detail_goal', null]]]);
    const [[errors, retry]] = failed.held as [[Error[], () => void]];
    assert.deepEqual(errors, [state.error]);
    assert.deepEqual(errors.map(error => error.message), ['Background preparation is off.']);
    retry();
    assert.equal(state.retries, 1);
    assert.ok((failed.diagnostics.at(-1) as string[]).includes('Goal history unavailable: Background preparation is off.'));
    // The range that did answer is still read locally.
    assert.deepEqual(render(20).details, []);
  } finally { await close(); }
});

test('switching faction never shows the last faction\'s rows', async () => {
  const { state, render, close } = await mount();
  try {
    assert.deepEqual(render(20).rows, [[['Mod mod_goal', '25000']]]);
    state.faction = '10';
    const switched = render(20);
    assert.deepEqual(switched.rows, []);
    assert.equal(switched.states.at(-1)?.kind, 'loading');
    assert.deepEqual(switched.details, []);
  } finally { await close(); }
});

test('goal history diagnostics use failure messages without error names', async () => {
  const { state, render, close } = await mount();
  try {
    for (const [failure, diagnostic] of [
      [new Error('Background preparation failed.'), 'Goal history unavailable: Background preparation failed.'],
      [new RefusedError('Background preparation is off.'), 'Goal history unavailable: Background preparation is off.'],
    ] as const) {
      state.error = failure;
      state.parts['9'] = [{ firstHour: null, lastHour: null, error: failure }];
      assert.deepEqual(render(120).diagnostics, [[diagnostic]]);
    }
  } finally { await close(); }
});
