import assert from 'node:assert/strict';
import { test } from 'node:test';
import { drawnViews } from './drawn.ts';
import { Page } from '../../dashboard/follow/page.ts';
import { Lane, type Bitmapped } from '../lane.ts';
import { bindFamilyManifest } from '../../families/registry.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import type { FamilyTableBinding } from '../../families/table/laneView.ts';
import { tableSettingsOf } from '../../families/table/settings.ts';
import { LANE_THEME } from '../../families/laneRuntime.fixture.ts';

test('astra 7: a table breaks at its filter bounds', async () => {
  const { occupiedStatesProjection } = await import('../../widgets/occupied-states/occupiedProjection.ts');
  const next = (options: unknown) => occupiedStatesProjection.nextChange!(createBoundedReadView({ hour: 100 }), options as never);
  const filtered = { filter: { passing: null, unrecorded: [], bounds: [0, 109, 300] }, role: 'occupier', lawAs: 'dot', filterAll: true };
  assert.equal(next(filtered), 109, 'the filter changes at 109, never sampled a day later');
});

test('lazy projection arriving after first next wakes the lane and reports its true boundary', async () => {
  const pending: { resolve: ((mod: unknown) => void) | null } = { resolve: null };
  bindFamilyManifest([{
    kind: 'synthetic-deferred-next',
    view: 'default',
    family: 'table',
    loadSpec: () => new Promise(resolve => { pending.resolve = resolve as (mod: unknown) => void; }),
  }]);

  const savedOffscreen = globalThis.OffscreenCanvas;
  class StubCanvas {
    width: number;
    height: number;
    constructor(width: number, height: number) { this.width = width; this.height = height; }
    getContext() {
      return {
        setTransform() {}, clearRect() {}, fillRect() {}, strokeRect() {}, beginPath() {}, roundRect() {},
        fill() {}, stroke() {}, rect() {}, clip() {}, save() {}, restore() {}, fillText() {},
        measureText: (t: string) => ({ width: t.length * 7 }),
        createLinearGradient: () => ({ addColorStop() {} }),
        font: '', fillStyle: '', strokeStyle: '', globalAlpha: 1, lineWidth: 1, textAlign: 'left', textBaseline: 'middle',
      };
    }
  }
  (globalThis as Record<string, unknown>).OffscreenCanvas = StubCanvas;

  try {
    type Fake = Bitmapped & { close(): void };
    const drain = async () => { for (let i = 0; i < 64; i++) await Promise.resolve(); };
    const views = new Map<string, ReturnType<typeof drawnViews>>();
    const presented: number[] = [];
    let lane!: Lane<Fake>;
    const page = new Page<Fake>({ lanes: 1, post: (_lane, message) => { if (message.type === 'frame') lane.frame(message); } });
    lane = new Lane<Fake>({
      lane: 0,
      lanes: 1,
      views(kind) {
        let view = views.get(kind);
        if (!view) {
          view = drawnViews(kind, widget => lane.touch(widget))!;
          views.set(kind, view);
        }
        return view;
      },
      crop: async sources => sources.map(source => ({ width: source.width, height: source.height, close() {} })),
      present: presentation => {
        for (const entry of presentation.entries) presented.push(entry.hi);
        page.offer(presentation);
        queueMicrotask(() => page.frame());
      },
      release() {},
      fail(reason) { throw new Error(reason); },
      later: run => queueMicrotask(run),
    });

    const settings = tableSettingsOf(undefined, [], { density: 'normal', sort: { column: '', direction: 'ascending' } });
    const binding: FamilyTableBinding = {
      family: 'table',
      kind: 'synthetic-deferred-next',
      options: {},
      settings,
      sources: {},
      width: 200,
      height: 120,
      scrollLeft: 0,
      scrollTop: 0,
      theme: LANE_THEME,
    };

    lane.mount(1, [], 'table', binding);
    page.mount(1, { route: 1, lane: 0, offset: 0, pin: null, surfaces: [{ install() {}, clear() {} }] });
    lane.bind(1, { generation: 1, surfaces: [{ width: 200, height: 120, dpr: 1 }], offset: 0, pin: null, dated: { h: 100, c: 200 } }, binding);
    page.publish(100, { playing: false });
    await drain();

    assert.ok(pending.resolve, 'next started the lazy projection load');
    assert.deepEqual(presented, [], 'while loading, the lane presents nothing and waits to be woken');
    assert.equal(page.view(1).mode, 'reading', 'and nothing is installed');

    pending.resolve!({
      family: 'table',
      kind: 'synthetic-deferred-next',
      view: 'default',
      projection: {
        family: 'table',
        kind: 'synthetic-deferred-next',
        nextChange: () => 109,
        project: () => ({ columns: [], rows: [], hour: 100, empty: 'None' }),
      },
    });
    const beforeLoad = presented.length;
    await drain();

    assert.ok(presented.length > beforeLoad, 'the lane presents again with no clock frame');
    assert.equal(presented.at(-1), 108, 'the landed projection wakes the lane, which presents its true next change');
  } finally {
    (globalThis as Record<string, unknown>).OffscreenCanvas = savedOffscreen;
  }
});
