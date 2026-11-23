import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import { coveredBy } from './mapOverlays.ts';
import type { MapBounds } from './api';

test('a map is covered only when its holes together leave none of it', () => {
  const map = { x: 0, y: 0, width: 100, height: 100 };
  assert.equal(coveredBy(map, []), false);
  assert.equal(coveredBy(map, [{ x: 20, y: 20, width: 40, height: 40 }]), false, 'a popover inside the map leaves the rest of it shown');
  assert.equal(coveredBy(map, [{ x: -10, y: -10, width: 200, height: 200 }]), true);
  assert.equal(coveredBy(map, [{ x: 0, y: 0, width: 60, height: 100 }, { x: 50, y: 0, width: 50, height: 100 }]), true, 'overlapping holes can cover it together');
  assert.equal(coveredBy(map, [{ x: 0, y: 0, width: 60, height: 100 }, { x: 61, y: 0, width: 50, height: 100 }]), false, 'a one-pixel strip still shows');
  assert.equal(coveredBy({ x: 0, y: 0, width: 0, height: 10 }, []), true);
});

class FakeElement {
  attributes = new Map<string, string>();
  open = false;
  isConnected = true;
  visibility = 'visible';
  rect = { left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0 };
  constructor(role: string | null) { this.attributes.set('popover', 'auto'); if (role) this.attributes.set('role', role); }
  hasAttribute(name: string) { return this.attributes.has(name); }
  getAttribute(name: string) { return this.attributes.get(name) ?? null; }
  setAttribute(name: string, value: string) { this.attributes.set(name, value); }
  removeAttribute(name: string) { this.attributes.delete(name); }
  matches(selector: string) { assert.equal(selector, ':popover-open'); return this.open; }
  getBoundingClientRect() { return this.rect; }
  place(left: number, top: number, width: number, height: number) { this.rect = { left, top, right: left + width, bottom: top + height, width, height }; }
}

test('a popover stays transparent until the map has cut the hole it is shown in, and the hole lasts through its exit', async () => {
  const frames: (() => void)[] = [];
  const handlers = new Map<string, (event: unknown) => void>();
  const observers: (() => void)[] = [];
  const sent: { holes: MapBounds[]; done: () => void }[] = [];
  const host = globalThis as Record<string, unknown>;
  const saved = ['document', 'window', 'HTMLElement', 'ResizeObserver', 'MutationObserver', 'getComputedStyle', '__holeFrames'].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const);
  const define = (key: string, value: unknown) => Object.defineProperty(globalThis, key, { configurable: true, writable: true, value });
  define('__holeFrames', frames);
  define('document', { addEventListener: (type: string, handler: (event: unknown) => void) => handlers.set(type, handler), removeEventListener() {},
    documentElement: { clientWidth: 1000, clientHeight: 800 }, querySelector: () => null });
  define('window', { devicePixelRatio: 2, addEventListener() {}, removeEventListener() {} });
  define('HTMLElement', FakeElement);
  class Observer { constructor(callback: () => void) { observers.push(callback); } observe() {} unobserve() {} disconnect() {} }
  define('ResizeObserver', Observer);
  define('MutationObserver', Observer);
  define('getComputedStyle', (element: FakeElement) => ({ visibility: element.visibility }));
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'hole-frames', enforce: 'pre',
      transform(_source, id) {
        if (id.replaceAll('\\', '/').endsWith('/engine/frame.ts')) return 'export const engine = { next(_phase, run) { globalThis.__holeFrames.push(run); return () => {}; } };';
      },
    }] });
  try {
    const { mapHoles } = await vite.ssrLoadModule('/ui/src/shell/mapOverlays.ts') as typeof import('./mapOverlays.ts');
    const settle = () => new Promise(resolve => setImmediate(resolve));
    const frame = async () => { while (frames.length) frames.shift()!(); await settle(); };
    const toggle = (element: FakeElement, open: boolean) => {
      handlers.get('beforetoggle')!({ target: element, newState: open ? 'open' : 'closed' });
      element.open = open;
    };
    const moved = () => observers.forEach(observer => observer());
    const pending = (element: FakeElement) => element.hasAttribute('data-map-hole-pending');
    let changes = 0;
    const stopListening = mapHoles.subscribe(() => changes++);
    const stop = mapHoles.start(holes => new Promise(done => sent.push({ holes, done })));

    const tip = new FakeElement('tooltip');
    toggle(tip, true);
    tip.place(10, 10, 50, 20);
    assert.equal(pending(tip), false, 'a tooltip with no popover open stays clear of the map by itself');

    const menu = new FakeElement('dialog');
    menu.visibility = 'hidden';
    toggle(menu, true);
    menu.place(0, 0, 200, 100);
    assert.equal(pending(menu), true);
    await frame();
    assert.deepEqual(sent.map(send => send.holes), [[{ x: 0, y: 0, width: 400, height: 200 }]], 'holes are physical pixels');
    sent.shift()!.done(); await settle();
    assert.equal(pending(menu), true, 'a hole measured during the hidden first layout does not reveal the popover');

    menu.visibility = 'visible';
    menu.place(300, 100, 100, 100);
    moved(); await frame();
    assert.deepEqual(sent[0].holes, [{ x: 600, y: 200, width: 200, height: 200 }]);
    assert.equal(pending(menu), true, 'it waits for the map to apply the hole');
    menu.place(300, 100, 150, 100);
    sent.shift()!.done(); await settle();
    assert.equal(pending(menu), true, 'a popover that grew before the next measurement is not revealed by the smaller hole');
    menu.place(300, 100, 200, 100);
    moved(); await frame();
    assert.equal(pending(menu), true, 'a popover that grew while its hole was on the way waits for the larger hole');
    assert.deepEqual(sent[0].holes, [{ x: 600, y: 200, width: 400, height: 200 }]);
    sent.shift()!.done(); await settle();
    assert.equal(pending(menu), false);
    assert.deepEqual(mapHoles.holes(), [{ x: 600, y: 200, width: 400, height: 200 }]);

    const nested = new FakeElement('tooltip');
    toggle(nested, true);
    nested.place(320, 210, 40, 20);
    assert.equal(pending(nested), true, 'a tooltip over an open popover cuts its own hole');
    await frame();
    assert.equal(sent[0].holes.length, 2);
    sent.shift()!.done(); await settle();
    assert.equal(pending(nested), false);

    toggle(nested, false);
    nested.place(0, 0, 0, 0);
    toggle(menu, false);
    moved(); await frame();
    assert.deepEqual(sent[0].holes, [{ x: 600, y: 200, width: 400, height: 200 }], 'a closing popover keeps its hole while it still draws its exit');
    sent.shift()!.done(); await settle();
    menu.place(0, 0, 0, 0);
    moved(); await frame();
    assert.deepEqual(sent.shift()!.holes, [], 'the hole closes once the popover stops drawing');
    moved(); await frame();
    assert.equal(sent.length, 0, 'an unchanged hole list is not sent again');
    assert.ok(changes >= 3);
    stop(); stopListening();
  } finally {
    await vite.close();
    for (const [key, descriptor] of saved) { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else delete host[key]; }
  }
});
