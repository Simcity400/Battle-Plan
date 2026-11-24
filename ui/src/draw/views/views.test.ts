import assert from 'node:assert/strict';
import { test } from 'node:test';
import { drawnViews } from './drawn.ts';
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
  let resolveSpec: ((mod: any) => void) | null = null;
  bindFamilyManifest([{
    kind: 'synthetic-deferred-next',
    view: 'default',
    family: 'table',
    loadSpec: () => new Promise(resolve => { resolveSpec = resolve; }),
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
    const presented: { hi: number; lo: number; mode: string }[] = [];
    let lane!: Lane<Bitmapped & { close(): void }>;
    const views = new Map<string, ReturnType<typeof drawnViews>>();
    lane = new Lane({
      lane: 0,
      lanes: 1,
      views(kind) {
        let view = views.get(kind);
        if (!view) {
          view = drawnViews(kind, wid => lane.touch(wid))!;
          views.set(kind, view);
        }
        return view;
      },
      crop: async sources => sources.map(source => ({ width: source.width, height: source.height, close() {} })),
      present(presentation) {
        for (const entry of presentation.entries) {
          presented.push({ lo: entry.lo, hi: entry.hi, mode: entry.mode });
        }
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
    lane.bind(1, { generation: 1, surfaces: [{ width: 200, height: 120, dpr: 1 }], offset: 0, pin: null, dated: { h: 100, c: 200 } }, binding);
    lane.frame({ seek: 1, target: 100, speed: 1, exact: true, installed: [], refused: [] });
    for (let i = 0; i < 6; i++) await Promise.resolve();

    assert.ok(presented.length >= 1, 'initial frame is presented while loading');
    assert.equal(presented.at(-1)!.hi, 100, 'while loading, next must not claim Infinity validity');
    assert.ok(resolveSpec, 'lazy projection load started from next/paint');

    resolveSpec!({
      family: 'table',
      kind: 'synthetic-deferred-next',
      view: 'default',
      projection: {
        family: 'table',
        kind: 'synthetic-deferred-next',
        nextChange: (_rv: unknown, _opts: unknown) => 109,
        project: () => ({ columns: [], rows: [], hour: 100, empty: 'None' }),
      },
    });
    for (let i = 0; i < 8; i++) await Promise.resolve();

    const afterLoad = presented.at(-1)!;
    assert.equal(afterLoad.lo, 100);
    assert.equal(afterLoad.hi, 108, 'after projection arrives, wake re-asks next and installs true boundary [100, 108]');
  } finally {
    (globalThis as Record<string, unknown>).OffscreenCanvas = savedOffscreen;
  }
});
