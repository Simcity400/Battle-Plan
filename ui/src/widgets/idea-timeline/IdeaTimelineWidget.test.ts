import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { DashboardSource } from '../../dashboard/client';
import { registerWidgets } from '../../dashboard/definitions.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { bindTracks, clockBounds, createWidget, resolveWindow } from '../../dashboard/model.ts';
import { PlaybackController } from '../../dashboard/playback.ts';
import type { TimelineLane } from '../../charts/timeline/layout';
import { definition } from './definition.ts';
import { definition as spiritsDefinition } from '../spirit-history/definition.ts';
import type { IdeaHistoryReply, IdeaInfo } from '../../generated/dashboard';

registerWidgets([definition, spiritsDefinition]);
const info = (token: string, category: string, slot: string | null): IdeaInfo => ({ token, category, slot, law: false, designer: false,
  effects: [], targeted: [], equipment: [], research: [], traits: [], tooltips: [], effectsKnown: true, unknown: null, nameMissing: null });
const data: IdeaHistoryReply = { branchId: '1', tag: 'mod_country', asked: 'mod_country', unobserved: [], identityStarts: [100], modifiers: [], texts: [], warnings: [],
  categories: [
    { token: 'mod_laws', name: 'Mod laws', kind: null, hidden: false, politicsTab: true,
      slots: [{ token: 'mod_slot', name: 'Mod slot', character: false, seats: 1, laws: ['mod_law'] }] },
    { token: 'mod_spirits', name: 'Mod spirits', kind: 'national_spirit', hidden: false, politicsTab: true, slots: [] },
  ], ideas: [info('mod_law', 'mod_laws', 'mod_slot'), info('mod_spirit', 'mod_spirits', null)],
  spans: ['mod_law', 'mod_spirit'].map(token => ({ token, firstHour: 120, lastHour: 200, slot: null, character: null })),
  artwork: [], holders: [] };
function source(id: string, firstHour: number, lastHour: number): DashboardSource {
  return { id, path: `C:/recordings/${id}.battleplan`, revision: '1', metadata: { path: `C:/recordings/${id}.battleplan`, revision: '1',
    name: id, format: 1, captureMode: 'full', campaignKey: 'c0ffee', branches: [{ id: '1', sessionId: '1', firstHour, lastHour,
      main: true, fork: null, joins: [], sampleBoundary: 'Sample origin unknown', gaps: [] }],
    countries: [], playerCountryId: null, contentVerified: false, partial: null, warnings: [], buildWarnings: [] } };
}
type ShownTimeline = { extent: [number, number]; playhead: number | null; labelHour: number; preserveZoomOnExtentChange: boolean; lanes: TimelineLane[];
  tooltip: (lane: TimelineLane, span: TimelineLane['spans'][number], hour: number, full: boolean) => { note?: string; rows?: { label: string; value?: string }[] } };
const host = globalThis as typeof globalThis & { __politicsTimeline?: ShownTimeline; __politicsHistory?: unknown };

test('Idea and spirit history growth uses each resolved track hour, preserving held clocks and recorded spans', async () => {
  const tracks = bindTracks({ version: 1, name: 'Comparison', alignment: 'calendar', widgets: [], layouts: {},
    roles: ['short', 'long'].map(id => ({ id, label: id, selectedTag: null })),
    tracks: ['short', 'long'].map(id => ({ id, roleId: id, label: id, branchIndex: 0, anchorOffset: null })) },
  new Map([['short', source('short', 100, 200)], ['long', source('long', 0, 1000)]]));
  const clock = new PlaybackController();
  clock.setBounds(clockBounds(tracks), true);
  host.__politicsHistory = { track: tracks[0], tag: data.tag, data };
  const replacements: Record<string, string> = {
    '/dashboard/ideaHistory.ts': `import { resolveWindow } from '/ui/src/dashboard/model.ts'; export const useIdeaHour = (widget, runtime, track) => { const hour = track ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour())?.lastHour : undefined; return hour === undefined || hour < track.branch.firstHour || hour > track.branch.lastHour ? null : Math.floor(hour); }; export const useIdeaHistory = () => globalThis.__politicsHistory;`,
    '/dashboard/useIdeaTooltips.ts': 'export const useIdeaTooltips = () => undefined;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/timeline/Timeline.tsx': 'export default props => { globalThis.__politicsTimeline = props; return null; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'politics-timeline-fixture', enforce: 'pre',
      transform(_source, id) { return Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1]; } }] });
  try {
    for (const [kind, path] of [['idea-timeline', 'idea-timeline/IdeaTimelineWidget'], ['spirit-history', 'spirit-history/SpiritHistoryWidget']]) {
      const { default: Widget } = await vite.ssrLoadModule(`/ui/src/widgets/${path}.tsx`);
      const widget = createWidget(kind, 'short', kind);
      widget.time = { mode: 'follow' };
      const render = (hour: number) => {
        clock.seek(hour);
        delete host.__politicsTimeline;
        return renderToStaticMarkup(createElement(Widget, { widget, runtime: { tracks, clock, client: {} }, active: true,
          onDiagnostics() {} } as unknown as WidgetProps));
      };
      render(150);
      if (kind === 'idea-timeline') {
        const timeline = host.__politicsTimeline!;
        const tip = timeline.tooltip(timeline.lanes[0], timeline.lanes[0].spans[0], 150, true);
        assert.ok(tip.rows?.some(row => row.label === 'Held by' && row.value));
        assert.ok(!tip.rows?.some(row => row.label === 'First observed'));
        const note = tip.note ?? '';
        assert.doesNotMatch(note, /first observed/);
        assert.doesNotMatch(note, /Active ideas are read hourly/);
        assert.doesNotMatch(note, /advisors use their daily supplying read/);
      }
      assert.deepEqual(host.__politicsTimeline?.extent, [100, 201], `${kind}: default axis includes the final recorded hour`);
      assert.equal(host.__politicsTimeline?.preserveZoomOnExtentChange, false);
      widget.options = { grow: true };
      render(150);
      assert.deepEqual(host.__politicsTimeline?.extent, [100, 150]);
      assert.equal(host.__politicsTimeline?.labelHour, 150);
      assert.equal(host.__politicsTimeline?.playhead, 150);
      assert.equal(host.__politicsTimeline?.preserveZoomOnExtentChange, true);
      assert.equal(host.__politicsTimeline?.lanes[0].spans[0].lastHour, 200, 'growth does not truncate the history');
      widget.time = { mode: 'pin', windows: { short: { firstOffset: 0, lastOffset: 50 } } };
      render(400);
      assert.deepEqual(host.__politicsTimeline?.extent, [100, 150]);
      assert.equal(host.__politicsTimeline?.labelHour, 150, 'held time stays independent of the live clock');
      widget.time = { mode: 'campaign' };
      render(400);
      assert.deepEqual(host.__politicsTimeline?.extent, [100, 201]);
      assert.equal(host.__politicsTimeline?.labelHour, 200);
      assert.equal(host.__politicsTimeline?.playhead, null);
      widget.time = { mode: 'range', firstOffset: 20, lastOffset: 80 };
      render(400);
      assert.deepEqual(host.__politicsTimeline?.extent, [120, 181]);
      assert.equal(host.__politicsTimeline?.labelHour, 180);
      assert.equal(host.__politicsTimeline?.playhead, null);
      widget.time = { mode: 'follow' };
      assert.equal(resolveWindow(widget, tracks[0], tracks, 400)?.lastHour, 400);
      assert.match(render(400), /outside the recording/);
      assert.equal(host.__politicsTimeline, undefined, 'another aligned track cannot extend this branch');
      if (kind === 'spirit-history') {
        host.__politicsHistory = { track: tracks[0], tag: data.tag, data: { ...data, ideas: [data.ideas[0], { ...data.ideas[1], category: null }] } };
        const unknown = render(150);
        assert.match(unknown, /could not be classified/);
        assert.doesNotMatch(unknown, /No national spirits recorded yet/);
        host.__politicsHistory = { track: tracks[0], tag: data.tag, data };
      }
    }
  } finally { delete host.__politicsTimeline; delete host.__politicsHistory; await vite.close(); }
});
