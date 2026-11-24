import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ComponentType, ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { PlaybackController } from './playback.ts';
import { createServer } from 'vite';
import { tokensFromValues } from '../charts/tokens.ts';

const OUTSIDE = 'This time is outside the recording.';
type Host = typeof globalThis & { __stub?: Record<string, (...args: never[]) => unknown> };
const host = globalThis as Host;
const live = { read: () => ({ get: () => undefined, missing: () => 'Not recorded' }), subscribe: () => () => {} };
const head = (extra: Record<string, unknown> = {}) => ({ data: undefined, error: undefined, retry() {}, root() {}, live, outside: false, at: null, hour: null,
  atNow: () => null, shownAt: () => null, current: () => undefined, read: 'asked', ...extra });
type Replacement = string | ((code: string) => string);
const TOKENS = `export const useChartTokens = () => (${JSON.stringify(tokensFromValues(() => '#777', 'sans-serif'))}); export const useViewportHeight = () => 800;`;
const wrapped = (name: string) => (code: string) => `${code.replace(new RegExp(`export function ${name}(?=[<(])`), `function real_${name}`)}
export function ${name}(...args) { const stubbed = globalThis.__stub?.${name}?.(...args); return stubbed === undefined ? real_${name}(...args) : stubbed; }`;

async function mount(replacements: Record<string, Replacement>) {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false }, logLevel: 'silent',
    plugins: [{ name: 'outside-recording', enforce: 'pre', transform(code, id) {
      const found = Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1];
      return typeof found === 'function' ? found(code) : found;
    } }] });
  return { load: <T,>(path: string) => vite.ssrLoadModule(path) as Promise<T>, close: () => vite.close() };
}
const track = (firstHour: number, lastHour: number, axisOffset = 0) => ({ id: 'main', roleId: 'main', axisOffset, selectedTag: 'GER',
  source: { id: 'source', revision: '1', metadata: { countries: [], campaignKey: 'campaign', captureMode: 'full' } }, branch: { id: '1', firstHour, lastHour, gaps: [] } });
const clockAt = (bounds: { firstHour: number; lastHour: number }, hour: number) => {
  const clock = new PlaybackController();
  clock.setBounds(bounds, true);
  clock.seek(hour);
  return clock;
};
const markup = (node: ReactNode) => renderToStaticMarkup(node as never);
type Gated = { outside: { message: string; detail: string | undefined } | null };
const gated = (_runtime: never, gate: Gated) => head(gate.outside ? { outside: true, outsideMessage: gate.outside.message, outsideDetail: gate.outside.detail } : {});

test('a fuel consumption drill told to follow playback, in a widget pinned at 105, reads the playback hour: past its track it is outside the recording, never loading', async () => {
  const { load, close } = await mount({ '/dashboard/headProduct.ts': wrapped('useHeadProduct'), '/widgets/fuel/fuelReads.ts': wrapped('usePartSeries'),
    '/charts/drill/BreakdownChart.tsx': 'export default () => null;', '/charts/drill/DrillParts.tsx': 'export default () => null;', '/charts/useChartTokens.ts': TOKENS });
  try {
    const { default: Breakdown } = await load<{ default: ComponentType<Record<string, unknown>> }>('/ui/src/widgets/fuel/FuelConsumptionBreakdown.tsx');
    const shortTrack = track(100, 109);
    const widget = { id: 'fuel', kind: 'fuel', trackIds: ['main'], countries: { kind: 'tags', tags: ['GER'] }, time: { mode: 'pin', windows: { main: { firstOffset: 5, lastOffset: 5 } } }, options: {} };
    const clock = clockAt({ firstHour: 100, lastHour: 200 }, 110);
    host.__stub = {
      useHeadProduct: gated,
      usePartSeries: () => ({ summaries: [], onSummaryView() {}, error: undefined, retry() {}, owner: null }),
    };
    const shut = { recorded: () => null, within: () => null, span: () => null, follow: () => null };
    const drawn = markup(createElement(Breakdown, { runtime: { tracks: [shortTrack], client: {}, clock }, gate: shut, widget, track: shortTrack, tag: 'GER', hour: null, follows: true,
      digits: 2, campaign: 'campaign', selection: null, onSelection() {}, level: null, onLevel() {}, active: true, pickOwner: {}, extent: [100, 109], branchStart: 100,
      within: undefined, detailHours: 24, onPick() {}, onContextMenu() {} }));
    assert.ok(drawn.includes(OUTSIDE) && !drawn.includes('data-state="loading"'), `following playback at 110, the drill is outside its track: ${drawn}`);
  } finally { delete host.__stub; await close(); }
});

test('a named leader growing with playback, at 99 before its branch 100–109: the career is kept, current details are outside the recording, and no cursor or later event is drawn', async () => {
  const { load, close } = await mount({ '/dashboard/headProduct.ts': wrapped('useHeadProduct'), '/dashboard/products.ts': wrapped('useProduct'),
    '/widgets/generals/portraits.ts': wrapped('useLeaderPortraits'), '/charts/useChartTokens.ts': TOKENS,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/timeline/Timeline.tsx': 'export default props => { globalThis.__timeline = props; return null; };' });
  const drawn = globalThis as typeof globalThis & { __timeline?: { playhead: number | null; labelHour?: number; lanes: { spans: unknown[] }[] } };
  try {
    const { default: CareerView } = await load<{ default: ComponentType<Record<string, unknown>> }>('/ui/src/widgets/general-career/CareerView.tsx');
    const role = { owner: 'GER', kind: 1, category: 0, rank: 'Field Marshal', attributeNames: ['A', 'D', 'P', 'L'], maximumSkill: 9 };
    const career = { reference: '1', name: 'Leader 1', portraitHour: null, runs: [{ firstHour: 100, lastHour: 109, role, skill: 3, components: [1, 1, 1, 1] }],
      traitGains: [[104, 0, 1]], traits: [{ token: 'a', name: 'A', icon: null, missing: null }], commands: [{ firstHour: 102, lastHour: 108, formation: '7', kind: 'army', name: null }],
      battles: [], observed: [[100, 109]], battlesPartialFrom: null, resultsMissing: '', missing: null, warnings: [] };
    host.__stub = { useHeadProduct: gated, useProduct: (_client: never, request: { product: string } | null) => request?.product === 'unit-leader-career' ? { data: career, retry() {} } : undefined,
      useLeaderPortraits: () => ({ at: () => undefined, warnings: [] }) };
    const html = markup(createElement(CareerView, { reference: '1', widget: { id: 'career', kind: 'general-career', trackIds: ['main'], countries: { kind: 'all' },
      time: { mode: 'follow' }, options: { leader: '1', growWithPlayback: true } }, runtime: { tracks: [track(100, 109)], client: {}, clock: clockAt({ firstHour: 0, lastHour: 200 }, 99) },
      active: true, onDiagnostics() {} }));
    const timeline = drawn.__timeline!;
    assert.ok(html.includes(OUTSIDE) && !html.includes('Leader details unavailable'), `current details are outside the recording: ${html}`);
    assert.ok(timeline.playhead === null && timeline.labelHour === undefined, `no cursor is drawn at an hour the recording does not hold: ${timeline.playhead}`);
    assert.deepEqual(timeline.lanes.flatMap(lane => lane.spans), [], 'growing with playback at 99, nothing after 99 is drawn');
  } finally { delete host.__stub; delete drawn.__timeline; await close(); }
});
