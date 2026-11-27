import assert from 'node:assert/strict';
import test from 'node:test';
import type { GameWindowRegion, GameWindowRequest, GameWindowScroll } from './client.ts';
import {
  accessibleRegions,
  fitGameWindowSize,
  formatGameWindowText,
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
  acceptRender,
  frameShown,
  regionTooltipContent,
  listKeyInput,
  listPosition,
  scrollClickInput,
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

test('a wheel, the scrollbar arrows, its track and its thumb move a list through one list-position action', () => {
  const scroll = (list: string, position: number, control: GameWindowScroll['control'] = { kind: 'list' }): GameWindowScroll => ({
    list, vertical: true, position, range: 100, step: 30, page: 50, control,
  });
  const list = (path: string, rect: GameWindowRegion['rect'], position: number): GameWindowRegion => ({
    path, rect, clip: null, disabled: false, close: false, passthrough: true, tooltip: null, scroll: scroll(path, position),
  });
  const part = (path: string, rect: GameWindowRegion['rect'], control: GameWindowScroll['control']): GameWindowRegion => ({
    path: `win/inner:scrollbar/${path}`, rect, clip: null, disabled: false, close: false, tooltip: null, scroll: scroll('win/inner', 90, control),
  });
  const regions: GameWindowRegion[] = [
    list('win/outer', [0, 0, 200, 200], 0),
    { path: 'win/still', rect: [0, 150, 200, 50], clip: null, disabled: false, close: false, tooltip: null,
      scroll: { ...scroll('win/still', 0), range: 0 } },
    list('win/inner', [10, 10, 50, 50], 90),
    part('up', [60, 10, 10, 10], { kind: 'step', direction: -1 }),
    part('track', [60, 20, 10, 30], { kind: 'track' }),
    part('thumb', [60, 40, 10, 5], { kind: 'thumb', travel: 25, thumb: 22 }),
    part('down', [60, 50, 10, 10], { kind: 'step', direction: 1 }),
  ];
  assert.equal(hitTestScroll(regions, 20, 20)?.path, 'win/inner');
  assert.equal(hitTestScroll(regions, 100, 100)?.path, 'win/outer');
  assert.equal(hitTestScroll(regions, 100, 170)?.path, 'win/outer');
  assert.equal(hitTestScroll(regions, 300, 300), null);
  assert.equal(hitTestScroll(regions, 65, 30)?.scroll?.list, 'win/inner', 'a wheel over the scrollbar moves its list');
  const inner = regions[2].scroll!;
  assert.equal(listPosition(inner, undefined, { kind: 'wheel', delta: 120 }), 100);
  assert.equal(listPosition(inner, undefined, { kind: 'wheel', delta: -3 }), 60);
  assert.equal(listPosition(inner, 15, { kind: 'wheel', delta: -120 }), 0);
  assert.equal(listPosition(inner, 40, { kind: 'wheel', delta: 0 }), 40);
  assert.equal(hitTestHover(regions, 20, 20), null);
  assert.equal(hitTestClick(regions, 20, 20), null);

  const click = (x: number, y: number) => scrollClickInput(regions, hitTestClick(regions, x, y), x, y);
  assert.deepEqual(click(65, 12), { kind: 'step', direction: -1 });
  assert.deepEqual(click(65, 55), { kind: 'step', direction: 1 });
  assert.deepEqual(click(65, 25), { kind: 'page', direction: -1 }, 'a click on the track above the thumb pages up');
  assert.equal(click(65, 42), null, 'the thumb drags; it does not page');
  assert.equal(click(20, 20), null);
  assert.equal(listPosition(inner, 50, { kind: 'step', direction: -1 }), 20);
  assert.equal(listPosition(inner, 50, { kind: 'page', direction: 1 }), 100);
  assert.equal(listPosition(inner, 50, { kind: 'page', direction: -1 }), 0);
  assert.equal(listPosition(inner, 50, { kind: 'end', direction: 1 }), 100);
  const thumb = regions[5].scroll!;
  assert.equal(listPosition(thumb, undefined, { kind: 'drag', moved: 0 }), 88, 'a drag starts where the thumb is drawn');
  assert.equal(listPosition(thumb, undefined, { kind: 'drag', moved: -11 }), 44);
  assert.equal(listPosition(thumb, undefined, { kind: 'drag', moved: 90 }), 100);
  assert.deepEqual(listKeyInput('PageDown'), { kind: 'page', direction: 1 });
  assert.deepEqual(listKeyInput('ArrowUp'), { kind: 'step', direction: -1 });
  assert.deepEqual(listKeyInput('Home'), { kind: 'end', direction: -1 });
  assert.equal(listKeyInput('a'), null);
});

test('tabs and other view actions are buttons, lists are scrollbars, and notes keep only a tooltip', () => {
  const base = { clip: null, disabled: false, close: false, tooltip: null } as const;
  const listScroll = { list: 'win/list', vertical: true, position: 0, range: 5, step: 1, page: 2 } as const;
  const regions: GameWindowRegion[] = [
    { ...base, path: 'win/close', rect: [0, 0, 10, 10], close: true },
    { ...base, path: 'win/tab', rect: [10, 0, 10, 10], view: 'history' },
    { ...base, path: 'win/note', rect: [20, 0, 10, 10], tooltip: { title: 'Size', body: null, detail: null } },
    { ...base, path: 'win/list', rect: [0, 20, 50, 50], passthrough: true, scroll: { ...listScroll, control: { kind: 'list' } } },
    { ...base, path: 'win/list:scrollbar/up', rect: [50, 20, 5, 5], scroll: { ...listScroll, control: { kind: 'step', direction: -1 } } },
  ];
  const roles = accessibleRegions(regions, [100, 100]).map(entry => [entry.region.path, entry.role]);
  assert.deepEqual(roles, [
    ['win/close', 'close'],
    ['win/tab', 'view'],
    ['win/note', 'note'],
    ['win/list', 'list'],
  ]);
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

test('one native render per window: a new hour, tab or subject waits for the outstanding one, which is never aborted while the window is open', () => {
  const subject = gameWindowSubjectKey(baseRequest(), 2);
  assert.equal(gameWindowSubjectKey(baseRequest({ hour: 140 }), 2), subject);
  assert.notEqual(gameWindowSubjectKey(baseRequest({ target: { tag: 'GER', variant: 'Panzer III' } }), 2), subject);
  const at = (hour: number, generation = 0) => ({ key: gameWindowRequestKey(baseRequest({ hour }), 2), subject, generation, active: true });
  const flight = { key: at(100).key, subject, generation: 0 };
  assert.equal(planGameWindowRender(at(101), flight, null), 'wait');
  assert.equal(followGameWindowRender(flight, at(105), null), true, 'the latest hour starts when the outstanding render settles');
  assert.equal(followGameWindowRender(flight, at(100), null), false);
  const other = { key: 'other', subject: 'other', generation: 0, active: true };
  assert.equal(planGameWindowRender(other, flight, null), 'wait', 'another subject or tab also waits: no second native call');
  assert.equal(followGameWindowRender(flight, other, null), true);
  assert.equal(planGameWindowRender(at(100), null, { key: at(100).key, subject, generation: 0 }), 'reuse');
  assert.equal(planGameWindowRender({ ...at(101), active: false }, flight, null), 'idle');
  assert.equal(planGameWindowRender(at(101), null, null), 'start');
});

test('a seek hides the displayed frame and a reply from before the seek is never installed', () => {
  const subject = gameWindowSubjectKey(baseRequest(), 2);
  const want = (hour: number, generation: number) => ({ key: gameWindowRequestKey(baseRequest({ hour }), 2), subject, generation, active: true });
  const shown = { key: want(100, 3).key, subject, generation: 3 };
  assert.equal(frameShown(shown, want(101, 3)), true, 'playback keeps the last frame up while the next hour renders');
  assert.equal(frameShown(shown, want(50, 4)), false, 'after a seek back the hour-100 picture and its regions are gone');
  assert.equal(frameShown(shown, { ...want(100, 3), subject: 'other' }), false);
  const late = { key: want(101, 3).key, subject, generation: 3 };
  assert.equal(acceptRender(late, want(50, 4)), false, 'the hour-101 reply arrives after the seek: rejected');
  assert.equal(acceptRender(late, want(102, 3)), true, 'within one stretch of playback a reply for an earlier hour is still the latest picture');
  assert.equal(acceptRender(late, { ...want(102, 3), subject: 'other' }), false);
  assert.equal(planGameWindowRender(want(50, 4), null, shown), 'start');
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
  assert.equal(focusable[1].role, 'note');
  assert.equal(focusable[2].role, 'close');
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

test('hitTestScroll selects the horizontal axis when horizontal wheel is requested on a 2D viewport', () => {
  const mk = (list: string, vertical: boolean): GameWindowRegion => ({
    path: list,
    rect: [0, 0, 200, 200],
    clip: null,
    disabled: false,
    close: false,
    passthrough: true,
    tooltip: null,
    scroll: { list, vertical, position: 0, range: 120, step: 40, page: 100, control: { kind: 'list' } },
  });
  const regions = [mk('tree/scroll:h', false), mk('tree/scroll:v', true)];
  assert.equal(hitTestScroll(regions, 50, 50, false)?.path, 'tree/scroll:v');
  assert.equal(hitTestScroll(regions, 50, 50, true)?.path, 'tree/scroll:h');
  const reversed = [mk('tree/scroll:v', true), mk('tree/scroll:h', false)];
  assert.equal(hitTestScroll(reversed, 50, 50, false)?.path, 'tree/scroll:v');
  assert.equal(hitTestScroll(reversed, 50, 50, true)?.path, 'tree/scroll:h');
  assert.equal(hitTestScroll([mk('only/v', true)], 50, 50, true)?.path, 'only/v');
  assert.equal(hitTestScroll([mk('only/h', false)], 50, 50, false)?.path, 'only/h');
});

test('formatGameWindowText formats visible values and unread missing reasons for screen readers', () => {
  assert.equal(
    formatGameWindowText({ path: 'production:0', label: 'Synthetic Cruiser', value: 'Light Hull — Units produced: 14', missing: null }),
    'Synthetic Cruiser: Light Hull — Units produced: 14',
  );
  assert.equal(
    formatGameWindowText({ path: 'history:0', label: 'Designed equipment', value: null, missing: 'design dates not recorded' }),
    'Designed equipment: not recorded (design dates not recorded)',
  );
  assert.equal(
    formatGameWindowText({ path: 'policy:0', label: 'Policy', value: null, missing: null }),
    'Policy: not recorded (no reason given)',
  );
});
