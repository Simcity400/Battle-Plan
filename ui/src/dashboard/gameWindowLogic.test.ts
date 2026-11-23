import assert from 'node:assert/strict';
import test from 'node:test';
import type { GameWindowRegion, GameWindowRequest } from './client.ts';
import {
  gameWindowRequestKey,
  hitTestRegion,
  nextScrollOffset,
  normalizeScale,
  regionTooltipContent,
} from './gameWindowLogic.ts';

function baseRequest(overrides: Partial<GameWindowRequest> = {}): GameWindowRequest {
  return {
    sourceId: 'src-1',
    revision: 'rev-1',
    branchId: '0',
    hour: 100,
    kind: 'equipmentDesigner',
    target: { tag: 'GER', variant: 'Panzer II' },
    scale: 1.5,
    scroll: { stats: 0 },
    subView: null,
    ...overrides,
  };
}

test('deduplicates identical requests regardless of target or scroll key order', () => {
  const a = gameWindowRequestKey(
    baseRequest({ target: { variant: 'Panzer II', tag: 'GER' }, scroll: { b: 10, a: 0 } }),
    2,
  );
  const b = gameWindowRequestKey(
    baseRequest({ target: { tag: 'GER', variant: 'Panzer II' }, scroll: { a: 0, b: 10 } }),
    2,
  );
  assert.equal(a, b);
  assert.equal(
    gameWindowRequestKey(baseRequest({ hour: 100.8 }), 2),
    gameWindowRequestKey(baseRequest({ hour: 100.1 }), 2),
  );
  assert.notEqual(gameWindowRequestKey(baseRequest({ hour: 101 }), 2), a);
  assert.notEqual(gameWindowRequestKey(baseRequest({ scale: 2 }), 2), a);
  assert.notEqual(gameWindowRequestKey(baseRequest(), 3), a);
});

test('normalizes DPR scales cleanly', () => {
  assert.equal(normalizeScale(1), 1);
  assert.equal(normalizeScale(1.25), 1.25);
  assert.equal(normalizeScale(1.5), 1.5);
  assert.equal(normalizeScale(2), 2);
  assert.equal(normalizeScale(0.2), 0.5);
  assert.equal(normalizeScale(5), 3);
  assert.equal(normalizeScale(Number.NaN), 1);
});

test('hit-tests top-most matching region and respects clip bounds', () => {
  const regions: GameWindowRegion[] = [
    {
      path: 'win/scroll_area',
      rect: [0, 0, 200, 200],
      clip: null,
      disabled: false,
      close: false,
      scroll: { target: 'stats', vertical: true, wheelFactor: 40 },
      tooltip: null,
    },
    {
      path: 'win/clipped_item',
      rect: [10, 10, 100, 100],
      clip: [10, 10, 40, 40],
      disabled: false,
      close: false,
      scroll: null,
      tooltip: { title: 'Clipped', body: 'Detail', detail: null },
    },
    {
      path: 'win/close_button',
      rect: [160, 8, 24, 24],
      clip: null,
      disabled: false,
      close: true,
      scroll: null,
      tooltip: { title: 'Close', body: null, detail: null },
    },
  ];

  assert.equal(hitTestRegion(regions, 25, 25)?.path, 'win/clipped_item');
  assert.equal(hitTestRegion(regions, 70, 70)?.path, 'win/scroll_area');
  const closeHit = hitTestRegion(regions, 170, 16);
  assert.equal(closeHit?.close, true);
  assert.deepEqual(regionTooltipContent(closeHit!), {
    title: 'Close',
    rows: [],
    description: undefined,
    detail: undefined,
  });
  assert.equal(hitTestRegion(regions, 250, 250), null);
});

test('updates and clamps scroll offsets without emitting no-op changes', () => {
  const region = { target: 'stats', vertical: true, wheelFactor: 40 };
  assert.equal(nextScrollOffset({}, region, -100, 0), null);
  assert.deepEqual(nextScrollOffset({}, region, 120, 0), { stats: 40 });
  assert.deepEqual(nextScrollOffset({ stats: 40 }, region, 120, 0), { stats: 80 });
  assert.deepEqual(nextScrollOffset({ stats: 20 }, region, -120, 0), { stats: 0 });
});
