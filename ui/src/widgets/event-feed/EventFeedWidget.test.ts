import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { DashboardSource } from '../../dashboard/client';
import { registerWidgets } from '../../dashboard/definitions.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { bindTracks, clockBounds, createWidget, pinTime, resolveWindow } from '../../dashboard/model.ts';
import { PlaybackController } from '../../dashboard/playback.ts';
import { definition } from './definition.ts';

registerWidgets([definition]);
const host = globalThis as typeof globalThis & { __eventBounds?: {
  ready: boolean; feed?: { active: boolean; hour: number | null; from: number | null };
  labels?: { active: boolean; wanted: unknown }; log?: { playhead: number | null };
} };
function source(id: string, firstHour: number, lastHour: number): DashboardSource {
  return { id, path: `C:/recordings/${id}.battleplan`, revision: '1', metadata: { path: `C:/recordings/${id}.battleplan`, revision: '1',
    name: id, format: 1, captureMode: 'full', campaignKey: 'c0ffee', branches: [{ id: '1', sessionId: '1', firstHour, lastHour,
      parentId: null, parentHour: null, sampleBoundary: 'Sample origin unknown', gaps: [] }],
    countries: [], playerCountryId: null, contentVerified: false, partial: null, warnings: [] } };
}

test('Event feed validates the resolved track cutoff, suspends old viewport reads and offers independent Pin', async () => {
  const tracks = bindTracks({ version: 1, name: 'Comparison', alignment: 'calendar', widgets: [], layouts: {},
    roles: ['short', 'long'].map(id => ({ id, label: id, selectedTag: null })),
    tracks: ['short', 'long'].map(id => ({ id, roleId: id, label: id, branchIndex: 0, anchorOffset: null })) },
  new Map([['short', source('short', 100, 200)], ['long', source('long', 0, 1000)]]));
  const clock = new PlaybackController(); clock.setBounds(clockBounds(tracks), true);
  host.__eventBounds = { ready: true };
  // Keep the actual caller/model/clock. Replace product, artwork and list surfaces; the
  // single visible-row state stands for rows retained after the previous LogList unmount.
  const replacements: Record<string, string> = {
    '/widgets/event-feed/useEventFeed.ts': `export const useEventFeed = (_runtime, _track, active, hour, filter, _key) => {
      globalThis.__eventBounds.feed = { active, hour, from: filter.firstHour };
      return { data: globalThis.__eventBounds.ready ? { unavailable: [] } : undefined,
        pages: [], warnings: [], errors: new Map(), onWantedPages() {}, retry() {}, retryPages() {} };
    };`,
    '/charts/log/LogList.tsx': 'export default props => { globalThis.__eventBounds.log = props; return null; };',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({});',
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = (_client, _track, wanted, active) => {
      globalThis.__eventBounds.labels = { active, wanted }; return { labelAt: () => tag => ({ name: tag }) };
    };`,
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ categories: ['#112233', '#223344', '#334455', '#445566', '#556677', '#667788'] });",
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'event-feed-bounds-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/widgets/event-feed/EventFeedWidget.tsx')) return source.replace('useMemo, useState', 'useMemo')
          .replace("from 'react';", "from 'react'; import { useState } from 'event-visible-row';");
      },
      resolveId(id) { if (id === 'event-visible-row') return '\0event-visible-row'; },
      load(id) { if (id === '\0event-visible-row') return `export const useState = () => [[{
        key: 'previous-visible', hour: 150, item: { event: { nameHour: 150, tags: ['native country'] }, item: null }
      }], () => {}];`; },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/event-feed/EventFeedWidget.tsx');
    const widget = createWidget('event-feed', 'short');
    assert.deepEqual(widget.time, { mode: 'campaign' });
    assert.ok(definition.timeModes.includes('pin'), 'the shared Time settings must offer Pin');
    const render = (hour: number) => {
      clock.seek(hour); delete host.__eventBounds!.log;
      return renderToStaticMarkup(createElement(Widget, { widget, runtime: { tracks, clock, client: {} }, active: true,
        onDiagnostics() {} } as unknown as WidgetProps));
    };
    const invalid = (hour: number) => {
      assert.match(render(hour), /outside the recording/);
      assert.equal(host.__eventBounds!.log, undefined);
      assert.equal(host.__eventBounds!.feed!.active, false);
      assert.equal(host.__eventBounds!.feed!.hour, null);
      assert.equal(host.__eventBounds!.labels!.active, false, 'retained visible rows must stop requesting country looks');
    };
    widget.time = { mode: 'follow' };
    assert.equal(resolveWindow(widget, tracks[0], tracks, 400)?.lastHour, 400);
    invalid(400); invalid(99);
    host.__eventBounds!.ready = false; invalid(400); host.__eventBounds!.ready = true;
    render(150);
    assert.deepEqual(host.__eventBounds!.feed, { active: true, hour: 150, from: null });
    assert.equal(host.__eventBounds!.log!.playhead, 150);
    assert.deepEqual(host.__eventBounds!.labels, { active: true, wanted: [{ hour: 150, tag: 'native country' }] });
    // Cursor requests follow the resolved cutoff directly; there is no separate live flag.
    render(175);
    assert.deepEqual(host.__eventBounds!.feed, { active: true, hour: 175, from: null }, 'Follow advances the feed cutoff with the clock');
    assert.equal(host.__eventBounds!.log!.playhead, 175);
    widget.time = pinTime(widget, tracks, 150);
    render(400);
    assert.deepEqual(host.__eventBounds!.feed, { active: true, hour: 150, from: null });
    assert.equal(host.__eventBounds!.log!.playhead, null, 'Pin remains held while the clock is outside this branch');
    widget.time = { mode: 'pin', windows: { short: { firstOffset: 0, lastOffset: 101 } } }; invalid(150);
    widget.time = { mode: 'pin', windows: { short: { firstOffset: 0, lastOffset: -1 } } }; invalid(150);
    widget.time = { mode: 'pin', windows: {} }; invalid(150);
    widget.time = { mode: 'campaign' }; render(400);
    assert.deepEqual(host.__eventBounds!.feed, { active: true, hour: 200, from: null });
    widget.time = { mode: 'range', firstOffset: -50, lastOffset: 300 }; render(400);
    assert.deepEqual(host.__eventBounds!.feed, { active: true, hour: 200, from: 50 }, 'an overlapping range keeps its recorded portion');
    widget.time = { mode: 'range', firstOffset: 20, lastOffset: 80 }; render(400);
    assert.deepEqual(host.__eventBounds!.feed, { active: true, hour: 180, from: 120 });
    widget.time = { mode: 'range', firstOffset: 101, lastOffset: 300 }; invalid(150);
    widget.time = { mode: 'range', firstOffset: -100, lastOffset: -1 }; invalid(150);
  } finally { delete host.__eventBounds; clock.stop(); await vite.close(); }
});
