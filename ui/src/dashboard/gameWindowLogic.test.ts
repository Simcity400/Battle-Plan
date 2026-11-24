import assert from 'node:assert/strict';
import test from 'node:test';
import type { GameWindowRegion, GameWindowRequest } from './client.ts';
import {
  accessibleRegions,
  gameWindowRequestKey,
  hitTestRegion,
  isActivatableClose,
  normalizeGameWindowRequest,
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
    subView: null,
    ...overrides,
  };
}

test('deduplicates identical requests and varies when any single key field changes', () => {
  const base = gameWindowRequestKey(baseRequest(), 2);
  const reordered = gameWindowRequestKey(
    baseRequest({ target: { variant: 'Panzer II', tag: 'GER' } }),
    2,
  );
  assert.equal(base, reordered);
  assert.equal(
    gameWindowRequestKey(baseRequest({ hour: 100.8 }), 2),
    gameWindowRequestKey(baseRequest({ hour: 100.1 }), 2),
  );
  assert.deepEqual(
    normalizeGameWindowRequest(baseRequest({ hour: 100.9, scale: 1.254 })),
    normalizeGameWindowRequest(baseRequest({ hour: 100.1, scale: 1.251 })),
  );
  assert.equal(normalizeScale(0.2), 0.5);
  assert.equal(normalizeScale(5), 3);
  assert.equal(normalizeScale(Number.NaN), 1);
  assert.notEqual(gameWindowRequestKey(baseRequest({ sourceId: 'src-2' }), 2), base);
  assert.notEqual(gameWindowRequestKey(baseRequest({ revision: 'rev-2' }), 2), base);
  assert.notEqual(gameWindowRequestKey(baseRequest({ branchId: '1' }), 2), base);
  assert.notEqual(gameWindowRequestKey(baseRequest({ hour: 101 }), 2), base);
  assert.notEqual(gameWindowRequestKey(baseRequest({ kind: 'mioWindow' }), 2), base);
  assert.notEqual(
    gameWindowRequestKey(baseRequest({ target: { tag: 'GER', variant: 'Panzer III' } }), 2),
    base,
  );
  assert.notEqual(gameWindowRequestKey(baseRequest({ scale: 2 }), 2), base);
  assert.notEqual(gameWindowRequestKey(baseRequest({ subView: 'stats' }), 2), base);
  assert.notEqual(gameWindowRequestKey(baseRequest(), 3), base);
});

test('hit-tests top-most matching region, respects clips, and blocks disabled close clicks', () => {
  const regions: GameWindowRegion[] = [
    {
      path: 'win/clipped_item',
      rect: [10, 10, 100, 100],
      clip: [10, 10, 40, 40],
      disabled: false,
      close: false,
      tooltip: { title: 'Clipped', body: 'Detail', detail: null },
    },
    {
      path: 'win/disabled_close',
      rect: [120, 8, 24, 24],
      clip: null,
      disabled: true,
      close: true,
      tooltip: { title: 'Locked close', body: null, detail: null },
    },
    {
      path: 'win/close_button',
      rect: [160, 8, 24, 24],
      clip: null,
      disabled: false,
      close: true,
      tooltip: { title: 'Close', body: null, detail: null },
    },
  ];

  assert.equal(hitTestRegion(regions, 25, 25)?.path, 'win/clipped_item');
  assert.equal(hitTestRegion(regions, 70, 70), null);
  const disabledClose = hitTestRegion(regions, 130, 16);
  assert.equal(disabledClose?.path, 'win/disabled_close');
  assert.equal(isActivatableClose(disabledClose), false);
  const closeHit = hitTestRegion(regions, 170, 16);
  assert.equal(isActivatableClose(closeHit), true);
  assert.deepEqual(regionTooltipContent(closeHit!), {
    title: 'Close',
    rows: [],
    description: undefined,
    detail: undefined,
  });
  assert.equal(hitTestRegion(regions, 250, 250), null);

  const focusable = accessibleRegions(regions, [200, 120]);
  assert.equal(focusable.length, 3);
  assert.deepEqual(focusable[0].rect, [10, 10, 40, 40]);
  assert.equal(focusable[1].closeable, false);
  assert.equal(focusable[2].closeable, true);
});
