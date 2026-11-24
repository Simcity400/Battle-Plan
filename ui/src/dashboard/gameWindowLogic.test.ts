import assert from 'node:assert/strict';
import test from 'node:test';
import type { GameWindowRegion, GameWindowRequest } from './client.ts';
import {
  accessibleRegions,
  fitGameWindowSize,
  followGameWindowRender,
  gameWindowRequestKey,
  gameWindowSubjectKey,
  hitTestClick,
  hitTestHover,
  hitTestScroll,
  isActivatableClose,
  normalizeGameWindowRequest,
  normalizeScale,
  planGameWindowRender,
  regionTooltipContent,
  wheelScrollPosition,
} from './gameWindowLogic.ts';
import {
  claimGameWindowSlot,
  releaseGameWindowSlot,
  updateGameWindowSlotPosition,
} from './gameWindows.ts';

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
  const scrolled = baseRequest({ scroll: { 'win/list': 40 } });
  assert.notEqual(gameWindowRequestKey(scrolled, 2), base);
  assert.equal(gameWindowRequestKey(baseRequest({ scroll: {} }), 2), base);
  assert.equal(gameWindowSubjectKey(scrolled, 2), gameWindowSubjectKey(baseRequest(), 2));
});

test('a wheel over a list moves it one step at a time within its range, the top-most list first', () => {
  const list = (path: string, rect: GameWindowRegion['rect'], position: number): GameWindowRegion => ({
    path, rect, clip: null, disabled: false, close: false, passthrough: true, tooltip: null,
    scroll: { position, range: 100, wheel: 30 },
  });
  const regions: GameWindowRegion[] = [
    list('win/outer', [0, 0, 200, 200], 0),
    { path: 'win/still', rect: [0, 150, 200, 50], clip: null, disabled: false, close: false, tooltip: null,
      scroll: { position: 0, range: 0, wheel: 30 } },
    list('win/inner', [10, 10, 50, 50], 90),
  ];
  assert.equal(hitTestScroll(regions, 20, 20)?.path, 'win/inner');
  assert.equal(hitTestScroll(regions, 100, 100)?.path, 'win/outer');
  assert.equal(hitTestScroll(regions, 100, 170)?.path, 'win/outer');
  assert.equal(hitTestScroll(regions, 300, 300), null);
  const inner = regions[2].scroll!;
  assert.equal(wheelScrollPosition(inner, undefined, 120), 100);
  assert.equal(wheelScrollPosition(inner, undefined, -3), 60);
  assert.equal(wheelScrollPosition(inner, 15, -120), 0);
  assert.equal(wheelScrollPosition(inner, 40, 0), 40);
  assert.equal(hitTestHover(regions, 20, 20), null);
  assert.equal(hitTestClick(regions, 20, 20), null);
});

test('a pass-through tooltip above the close button shows its tooltip but lets the click close', () => {
  const regions: GameWindowRegion[] = [
    { path: 'win/close_button', rect: [160, 8, 24, 24], clip: null, disabled: false, close: true,
      tooltip: { title: 'Close', body: null, detail: null } },
    { path: 'win/frame_tooltip', rect: [0, 0, 200, 40], clip: null, disabled: false, close: false, passthrough: true,
      tooltip: { title: 'Frame', body: null, detail: null } },
  ];
  assert.equal(hitTestHover(regions, 170, 16)?.path, 'win/frame_tooltip');
  assert.equal(isActivatableClose(hitTestClick(regions, 170, 16)), true);
  assert.equal(hitTestClick(regions, 20, 16), null);
});

test('playback of one subject keeps one render in flight and then renders the latest hour', () => {
  const subject = gameWindowSubjectKey(baseRequest(), 2);
  assert.equal(gameWindowSubjectKey(baseRequest({ hour: 140 }), 2), subject);
  assert.notEqual(gameWindowSubjectKey(baseRequest({ target: { tag: 'GER', variant: 'Panzer III' } }), 2), subject);
  const at = (hour: number) => ({ key: gameWindowRequestKey(baseRequest({ hour }), 2), subject, active: true });
  const flight = { key: at(100).key, subject };
  assert.deepEqual(planGameWindowRender(at(101), flight, null), { step: 'wait', abort: false });
  assert.equal(followGameWindowRender(flight, at(105), at(100).key), true);
  assert.equal(followGameWindowRender(flight, at(100), at(100).key), false);
  const other = { key: 'other', subject: 'other', active: true };
  assert.deepEqual(planGameWindowRender(other, flight, null), { step: 'start', abort: true });
  assert.equal(followGameWindowRender(flight, other, at(100).key), false);
  assert.deepEqual(planGameWindowRender(at(100), flight, at(100).key), { step: 'reuse', abort: true });
  assert.deepEqual(planGameWindowRender({ ...at(101), active: false }, flight, null), { step: 'idle', abort: true });
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

  assert.equal(hitTestHover(regions, 25, 25)?.path, 'win/clipped_item');
  assert.equal(hitTestHover(regions, 70, 70), null);
  const disabledClose = hitTestHover(regions, 130, 16);
  assert.equal(disabledClose?.path, 'win/disabled_close');
  assert.equal(isActivatableClose(disabledClose), false);
  const closeHit = hitTestHover(regions, 170, 16);
  assert.equal(isActivatableClose(closeHit), true);
  assert.deepEqual(regionTooltipContent(closeHit!), {
    title: 'Close',
    rows: [],
    description: undefined,
    detail: undefined,
  });
  assert.equal(hitTestHover(regions, 250, 250), null);

  const focusable = accessibleRegions(regions, [200, 120]);
  assert.equal(focusable.length, 3);
  assert.deepEqual(focusable[0].rect, [10, 10, 40, 40]);
  assert.equal(focusable[1].closeable, false);
  assert.equal(focusable[2].closeable, true);
});

test('hit-testing respects alpha masks so transparent pixels pass through to regions behind', () => {
  const maskBits = Buffer.from([0b0000_0110]).toString('base64');
  const regions: GameWindowRegion[] = [
    {
      path: 'win/behind',
      rect: [0, 0, 10, 10],
      clip: null,
      disabled: false,
      close: false,
      tooltip: { title: 'Behind', body: null, detail: null },
    },
    {
      path: 'win/alpha_front',
      rect: [2, 2, 2, 2],
      clip: null,
      disabled: false,
      close: true,
      tooltip: { title: 'Front', body: null, detail: null },
      alphaMask: { width: 2, height: 2, bitsBase64: maskBits },
    },
  ];

  assert.equal(hitTestHover(regions, 2.5, 2.5)?.path, 'win/behind');
  assert.equal(hitTestHover(regions, 3.5, 2.5)?.path, 'win/alpha_front');
  assert.equal(hitTestHover(regions, 2.5, 3.5)?.path, 'win/alpha_front');
  assert.equal(hitTestHover(regions, 3.5, 3.5)?.path, 'win/behind');
});

test('fitGameWindowSize keeps 1:1 when it fits and scales down proportionally without cropping', () => {
  assert.deepEqual(
    fitGameWindowSize([1091, 546], { width: 1440, height: 900 }),
    [1091, 546],
  );
  assert.deepEqual(
    fitGameWindowSize([1091, 546], { width: 1000, height: 600 }),
    [952, 476],
  );
  assert.deepEqual(
    fitGameWindowSize([1091, 546], { width: 1440, height: 500 }),
    [767, 384],
  );
  assert.deepEqual(fitGameWindowSize([0, 100], { width: 1440, height: 900 }), [0, 0]);
});

test('claimGameWindowSlot enforces one pop-up per kind and preserves position across owners', () => {
  let closedA = 0;
  let closedB = 0;
  assert.equal(claimGameWindowSlot('equipmentDesigner', 'widget-a', () => { closedA++; }), null);
  updateGameWindowSlotPosition('equipmentDesigner', 'widget-a', { left: 240, top: 120 });
  assert.deepEqual(
    claimGameWindowSlot('equipmentDesigner', 'widget-a', () => { closedA++; }),
    { left: 240, top: 120 },
  );
  assert.equal(closedA, 0);

  assert.deepEqual(
    claimGameWindowSlot('equipmentDesigner', 'widget-b', () => { closedB++; }),
    { left: 240, top: 120 },
  );
  assert.equal(closedA, 1);
  assert.equal(closedB, 0);

  releaseGameWindowSlot('equipmentDesigner', 'widget-a');
  assert.deepEqual(
    claimGameWindowSlot('equipmentDesigner', 'widget-b', () => { closedB++; }),
    { left: 240, top: 120 },
  );
  releaseGameWindowSlot('equipmentDesigner', 'widget-b');
  assert.equal(claimGameWindowSlot('equipmentDesigner', 'widget-a', () => { closedA++; }), null);
  releaseGameWindowSlot('equipmentDesigner', 'widget-a');
});
