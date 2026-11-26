import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ComponentType, ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { PlaybackController } from './playback.ts';
import { createServer } from 'vite';

const OUTSIDE = 'This time is outside the recording.';
type Host = typeof globalThis & { __stub?: Record<string, (...args: never[]) => unknown> };
const host = globalThis as Host;
const live = { read: () => ({ get: () => undefined, missing: () => 'Not recorded' }), subscribe: () => () => {} };
const head = (extra: Record<string, unknown> = {}) => ({ data: undefined, error: undefined, retry() {}, root() {}, live, outside: false, at: null, hour: null,
  atNow: () => null, shownAt: () => null, current: () => undefined, read: 'asked', ...extra });
type Replacement = string | ((code: string) => string);
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
test('a fuel consumption drill told to follow playback, in a widget pinned at 105, reads the playback hour: past its track it is outside the recording, never loading', async () => {
  const { load, close } = await mount({
    '/families/host/FamilyHost.tsx': 'export default ({ blockedState }) => blockedState ?? null;',
  });
  try {
    const { default: Breakdown } = await load<{ default: ComponentType<Record<string, unknown>> }>('/ui/src/widgets/fuel/FuelConsumptionBreakdown.tsx');
    const shortTrack = track(100, 109);
    const widget = { id: 'fuel', kind: 'fuel', trackIds: ['main'], countries: { kind: 'tags', tags: ['GER'] }, time: { mode: 'pin', windows: { main: { firstOffset: 5, lastOffset: 5 } } }, options: {} };
    const clock = clockAt({ firstHour: 100, lastHour: 200 }, 110);
    const shut = { recorded: () => null, within: () => null, span: () => null, follow: () => null };
    const region = { widget, action: { kind: 'follow' } };
    const drawn = markup(createElement(Breakdown, { runtime: { tracks: [shortTrack], client: {}, clock }, gate: shut, widget, track: shortTrack, tag: 'GER', hour: null, follows: true,
      digits: 2, campaign: 'campaign', history: null, selection: null, onSelection() {}, level: null, onLevel() {}, active: true, pickOwner: {}, extent: [100, 109], playhead: 110,
      within: null, detailHours: 24, region, onPick() {} }));
    assert.ok(drawn.includes(OUTSIDE) && !drawn.includes('data-state="loading"'), `following playback at 110, the drill is outside its track: ${drawn}`);
  } finally { await close(); }
});

test('nested technology and entity-stat effect windows pin to their clicked hour instead of following playback', async () => {
  const { load, close } = await mount({
    '/dashboard/headProduct.ts': wrapped('useHeadProduct'),
    '/components/FloatingPanel.tsx': 'export default ({ children }) => children;',
    '/widgets/research/GameText.tsx': 'export const GameArtFor = ({ children }) => children; export const GameBlocks = () => null; export const GameIcons = ({ children }) => children; export const GameRuns = () => null;',
    '/widgets/research/useTechnologyArtwork.ts': 'export const useTechnologyArtwork = () => ({ artwork: new Map(), failures: new Map() });',
  });
  try {
    const { default: TechnologyWindow, TechnologyEffects } = await load<{
      default: ComponentType<Record<string, unknown>>;
      TechnologyEffects: ComponentType<Record<string, unknown>>;
    }>('/ui/src/widgets/research/TechnologyWindow.tsx');
    const { default: EntityStatsWindow } = await load<{ default: ComponentType<Record<string, unknown>> }>('/ui/src/widgets/research/EntityStatsWindow.tsx');
    const fullTrack = track(100, 200);
    const widget = { id: 'proj', kind: 'special-projects', trackIds: ['main'], countries: { kind: 'tags', tags: ['GER'] }, time: { mode: 'follow' }, options: {} };
    const clock = clockAt({ firstHour: 100, lastHour: 200 }, 180);
    const seen: { product: string; action: unknown; hour: number | null }[] = [];
    host.__stub = {
      useHeadProduct: (_runtime: never, gate: { region: { action: unknown }; hour: number | null }, product: string) => {
        seen.push({ product, action: gate.region.action, hour: gate.hour });
        return head();
      },
    };
    markup(createElement(TechnologyEffects, { runtime: { tracks: [fullTrack], client: {}, clock }, widget, tag: 'GER', hour: 120, token: 'mod_tech', databaseIndex: null, active: true }));
    markup(createElement(TechnologyWindow, {
      runtime: { tracks: [fullTrack], client: {}, clock }, widget, tag: 'GER', hour: 122,
      link: { kind: 'technology', token: 'mod_tech_2', databaseIndex: null, text: 'Tech 2', x: 0, y: 0 }, active: true, onClose() {},
    }));
    markup(createElement(EntityStatsWindow, {
      runtime: { tracks: [fullTrack], client: {}, clock }, widget, tag: 'GER', hour: 125,
      link: { kind: 'equipment', token: 'mod_eq', databaseIndex: null, text: 'Rifle', x: 0, y: 0 }, active: true, onClose() {},
    }));
    assert.deepEqual(seen, [
      { product: 'technology-effects', action: { kind: 'at', hour: 120 }, hour: 120 },
      { product: 'technology-effects', action: { kind: 'at', hour: 122 }, hour: 122 },
      { product: 'content-entity-stats', action: { kind: 'at', hour: 125 }, hour: 125 },
    ]);
  } finally { delete host.__stub; await close(); }
});
