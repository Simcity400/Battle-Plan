import assert from 'node:assert/strict';
import { test } from 'node:test';
import { drawnViews } from './drawn.ts';
import { Page } from '../../dashboard/follow/page.ts';
import { Lane, type Bitmapped, type Scene } from '../lane.ts';
import { bindFamilyManifest } from '../../families/registry.ts';
import type { FamilyTableBinding } from '../../families/table/laneView.ts';
import { tableSettingsOf } from '../../families/table/settings.ts';

const scene = (model: unknown): Scene => ({ widget: 1, subs: [], pictures: [], model });
const reply = { missing: null, countries: [{ tag: 'GER', countryId: 1, total: 4, counts: [] }], types: [], warnings: [] };

test('astra 7: a race breaks at its membership change hour, not at midnight', () => {
  const race = drawnViews('race', () => {})!;
  const model = { top: null, tags: ['GER'], split: false, overlay: false, offset: 0, reply, membership: new Map([['GER', [{ firstHour: 109, lastHour: 500 }]]]) };
  assert.equal(race.next!(scene(model), 100), 109, 'GER joins the selection at 109');
  assert.equal(race.next!(scene(model), 109), 501);
});

test('astra 7: a table breaks at its filter bounds and at the hours its country names and flags change', async () => {
  let wakeResolve: ((widget: number) => void) | null = null;
  const woken = new Promise<number>(resolve => { wakeResolve = resolve; });
  const table = drawnViews('table', widget => { wakeResolve?.(widget); })!;
  const base = { family: 'table', kind: 'occupied-states', settings: {}, sources: {}, width: 100, height: 100, scrollLeft: 0, scrollTop: 0 };
  const filtered = {
    ...base,
    options: { filter: { passing: null, unrecorded: [], bounds: [0, 109, 300] }, role: 'occupier', labels: {}, lawAs: 'dot', filterAll: true },
  };
  const initial = table.next!(scene(filtered as unknown as FamilyTableBinding), 100);
  if (initial === 101) {
    assert.equal(await woken, 1, 'lazy projection load wakes the widget once loaded');
  }
  assert.equal(table.next!(scene(filtered as unknown as FamilyTableBinding), 100), 109, 'the filter changes at 109, never sampled a day later');
  const named = {
    ...base,
    options: { filter: { passing: null, unrecorded: [], bounds: [] }, role: 'occupier', labels: { GER: [{ from: 0, name: 'German Reich' }, { from: 130, name: 'Germany' }] }, lawAs: 'dot', filterAll: true },
  };
  assert.equal(table.next!(scene(named as unknown as FamilyTableBinding), 100), 130, 'the name changes at 130');
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

    const settings = tableSettingsOf(undefined, [], { density: 'normal', overflow: 'tooltip', sort: { column: '', direction: 'ascending' } });
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
    };

    lane.mount(1, [], 'table', binding);
    page.mount(1, { route: 1, lane: 0, offset: 0, pin: null, surfaces: [{ install() {}, clear() {} }] });
    lane.bind(1, { generation: 1, surfaces: [{ width: 200, height: 120, dpr: 1 }], offset: 0, pin: null, dated: { h: 100, c: 200 } }, binding);
    page.publish(100, { playing: false });
    await drain();

    assert.ok(pending.resolve, 'next started the lazy projection load');
    assert.equal(presented.at(-1), 100, 'while loading, the lane claims no validity past the displayed hour');
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
