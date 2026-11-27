import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const PROBE_PATH = new URL('./probe.js', import.meta.url);
const PROBE_SOURCE = readFileSync(PROBE_PATH, 'utf8');

function loadProbe(dom = {}) {
  const rafs = new Map();
  let nextRaf = 0, rafCalls = 0, now = 0;
  const microtasks = [];
  class FakeChannel {
    constructor() {
      this.port1 = { onmessage: null };
      this.port2 = { postMessage: data => { microtasks.push(() => this.port1.onmessage?.({ data })); } };
    }
  }
  class FakeObserver {
    observe() {}
  }
  const sandbox = {
    window: null,
    performance: { now: () => now, timeOrigin: 1000 },
    requestAnimationFrame: cb => { rafCalls++; const id = ++nextRaf; rafs.set(id, cb); return id; },
    cancelAnimationFrame: id => { rafs.delete(id); },
    setTimeout: (fn) => { microtasks.push(fn); return 1; },
    MessageChannel: FakeChannel,
    PerformanceObserver: FakeObserver,
    addEventListener: () => {},
    document: {
      createElement: () => ({ width: 0, height: 0, getContext: () => ({ clearRect() {}, drawImage() {}, getImageData: () => ({ data: [] }) }) }),
      querySelector: dom.querySelector ?? (() => null),
      querySelectorAll: dom.querySelectorAll ?? (() => []),
    },
    CSS: { escape: value => String(value) },
    innerWidth: 1280,
    innerHeight: 720,
    getComputedStyle: dom.getComputedStyle ?? (el => el?.__style ?? { overflowX: 'visible', overflowY: 'visible', contain: '' }),
  };
  sandbox.window = sandbox;
  vm.runInNewContext(PROBE_SOURCE, sandbox);
  const stepFrame = dt => {
    now += dt;
    const batch = [...rafs.entries()];
    rafs.clear();
    for (const [, cb] of batch) cb(now);
    while (microtasks.length) microtasks.shift()();
  };
  return {
    probe: sandbox.__e2e,
    pendingRafs: () => rafs.size,
    rafCalls: () => rafCalls,
    stepFrame,
  };
}

test('probe schedules no animation frames while idle, and runs only while tracking, watching, or recording frame gaps', async () => {
  const host = loadProbe();
  assert.equal(host.pendingRafs(), 0, 'idle probe schedules no animation frames on load');
  assert.equal(host.rafCalls(), 0);

  host.probe.watch();
  assert.equal(host.pendingRafs(), 1, 'watch starts the frame loop');
  host.stepFrame(16);
  assert.equal(host.pendingRafs(), 1, 'watch keeps running while active');
  host.probe.unwatch();
  assert.equal(host.pendingRafs(), 0, 'unwatch stops the frame loop');

  host.probe.mark(null);
  assert.equal(host.pendingRafs(), 1, 'mark starts the frame loop');
  const settled = host.probe.settle(1000, 0, null, ({ settled: done }) => done);
  host.stepFrame(16);
  host.stepFrame(16);
  host.stepFrame(16);
  await settled;
  assert.equal(host.pendingRafs(), 0, 'settle leaves no animation frames scheduled once done');

  const beforeDrag = host.rafCalls();
  host.probe.startFrames();
  assert.equal(host.pendingRafs(), 1, 'startFrames starts the frame loop for drag gap measurement');
  host.stepFrame(16);
  host.stepFrame(20);
  host.stepFrame(18);
  const gaps = host.probe.gaps(0, 1000);
  assert.ok(gaps.length >= 2, 'recorded frame intervals while active');
  host.probe.stopFrames();
  assert.equal(host.pendingRafs(), 0, 'stopFrames stops the frame loop');
  const afterDrag = host.rafCalls();
  assert.ok(afterDrag > beforeDrag);
  host.stepFrame(16);
  assert.equal(host.rafCalls(), afterDrag, 'no extra frames run after stopFrames');
});

function makeEl({ tag = 'div', classes = [], attrs = {}, text = '', rect = { left: 10, top: 10, right: 110, bottom: 50 }, style = { overflowX: 'visible', overflowY: 'visible', contain: '' }, children = [] } = {}) {
  const matchOne = part => (
    (part === '[data-state=loading]' || part === '[data-state="loading"]') ? attrs['data-state'] === 'loading'
      : part === '[aria-busy="true"]' ? attrs['aria-busy'] === 'true'
      : (part === '[role=status]' || part === '[role="status"]') ? attrs.role === 'status'
      : (part === '[data-state=error]' || part === '[role=alert]') ? attrs['data-state'] === 'error' || attrs.role === 'alert'
      : part === '.sidebar-row.library-row' ? classes.includes('sidebar-row') && classes.includes('library-row')
      : part === 'article.dash-widget' ? tag === 'article' && classes.includes('dash-widget')
      : part.startsWith('.') ? classes.includes(part.slice(1))
      : part === tag
  );
  const el = {
    tagName: tag.toUpperCase(), className: classes.join(' '), classList: { contains: name => classes.includes(name) },
    textContent: text, parentElement: null, __style: style, __children: [],
    checkVisibility: () => true, getBoundingClientRect: () => rect,
    getAttribute: name => (Object.hasOwn(attrs, name) ? attrs[name] : null),
    matches: sel => sel.split(',').some(part => matchOne(part.trim())),
    closest(sel) { for (let c = this; c; c = c.parentElement) if (c.matches(sel)) return c; return null; },
    querySelector(sel) { return this.querySelectorAll(sel)[0] ?? null; },
    querySelectorAll(sel) {
      const out = [];
      const walk = n => { for (const c of n.__children) { if (c.matches(sel)) out.push(c); walk(c); } };
      walk(this);
      return out;
    },
  };
  for (const child of children) { child.parentElement = el; el.__children.push(child); }
  return el;
}

test('probe ignores aria-busy on hidden screen-reader mirrors and the library row while keeping real loading widgets failing', () => {
  const hiddenRect = { left: 10, top: 10, right: 11, bottom: 11 };
  const srMirror = makeEl({
    tag: 'table', classes: ['fh-sr-only'], attrs: { 'aria-busy': 'true' }, rect: hiddenRect,
    style: { position: 'absolute', overflowX: 'hidden', overflowY: 'hidden', contain: 'strict', clip: 'rect(0px, 0px, 0px, 0px)', clipPath: 'inset(50%)' },
  });
  const clipParent = makeEl({
    rect: hiddenRect, style: { position: 'absolute', overflowX: 'hidden', overflowY: 'hidden', contain: '', clip: 'rect(0px, 0px, 0px, 0px)' },
    children: [makeEl({ tag: 'span', attrs: { 'aria-busy': 'true' }, rect: hiddenRect })],
  });
  const insetHidden = makeEl({ attrs: { 'aria-busy': 'true' }, style: { overflowX: 'visible', overflowY: 'visible', contain: '', clipPath: 'inset(50%)' } });
  const hiddenOnlyWidget = makeEl({
    tag: 'article', classes: ['dash-widget'], attrs: { 'aria-label': 'Occupied states' },
    children: [makeEl({ classes: ['dash-widget-content'], text: 'Occupied states', children: [makeEl({ tag: 'canvas' }), srMirror, clipParent, insetHidden] })],
  });
  const libRow = makeEl({
    tag: 'button', classes: ['sidebar-row', 'library-row'], attrs: { 'aria-busy': 'true' },
    children: [makeEl({ tag: 'span', classes: ['campaign-loading'], children: [makeEl({ tag: 'span', classes: ['spinner'] })] })],
  });
  const root = makeEl({ children: [hiddenOnlyWidget, libRow] });
  const host = loadProbe({ querySelectorAll: sel => root.querySelectorAll(sel) });
  assert.equal(host.probe.stateOf(hiddenOnlyWidget).state, 'content');
  host.probe.watch();
  host.stepFrame(16);
  host.stepFrame(32);
  const clean = host.probe.unwatch();
  assert.equal(clean.ms, 0, 'hidden mirrors and opening library row must not record placeholder time');
  assert.deepEqual([...clean.where], []);

  const visiblePartialClip = makeEl({
    attrs: { 'aria-busy': 'true' },
    style: { position: 'absolute', overflowX: 'visible', overflowY: 'visible', contain: '', clip: 'rect(0px, 100px, 50px, 0px)', clipPath: 'inset(10%)' },
  });
  const loadingWidget = makeEl({
    tag: 'article', classes: ['dash-widget'], attrs: { 'aria-label': 'Fuel' },
    children: [makeEl({ classes: ['dash-widget-content'], children: [visiblePartialClip] })],
  });
  const campaignRow = makeEl({
    tag: 'button', classes: ['sidebar-row', 'campaign-row'],
    children: [makeEl({ tag: 'span', classes: ['campaign-loading'], children: [makeEl({ tag: 'span', classes: ['spinner'] })] })],
  });
  const liveRoot = makeEl({ children: [loadingWidget, campaignRow] });
  const liveHost = loadProbe({ querySelectorAll: sel => liveRoot.querySelectorAll(sel) });
  assert.equal(liveHost.probe.stateOf(loadingWidget).state, 'loading');
  liveHost.probe.watch();
  liveHost.stepFrame(16);
  liveHost.stepFrame(40);
  const flagged = liveHost.probe.unwatch();
  assert.ok(flagged.ms > 0, 'visible loading widget and campaign row spinner must still record placeholder time');
  assert.deepEqual([...flagged.where], ['Fuel', 'spinner']);
  liveHost.probe.mark(null);
  assert.deepEqual([...liveHost.probe.report(100, false, []).placeholderStill], ['Fuel', 'spinner']);
  liveRoot.__children = [];
  assert.deepEqual([...liveHost.probe.report(110, false, []).placeholderStill], []);
});

test('probe countryRows and revealCountry handle both named and unnamed country options', async () => {
  const namedName = { tagName: 'SPAN', textContent: 'Germany', getAttribute: () => null, matches: s => s === '[data-overflow-label]' || s === 'span', querySelector: () => null, querySelectorAll: () => [], __children: [] };
  const namedTag = { tagName: 'SMALL', textContent: 'GER', previousElementSibling: namedName, getAttribute: () => null, matches: s => s === 'small', querySelector: () => null, querySelectorAll: () => [], __children: [] };
  const namedOpt = makeEl({ tag: 'div', attrs: { role: 'option', 'aria-posinset': '1', 'aria-setsize': '3', 'aria-selected': 'true' } });
  namedOpt.__children = [namedName, namedTag];

  const makeUnnamed = (pos, tag) => {
    const note = { tagName: 'SMALL', nodeType: 1, textContent: 'name not recorded', matches: s => s === 'small', querySelector: () => null, querySelectorAll: () => [], __children: [] };
    const span = {
      tagName: 'SPAN', nodeType: 1, className: 'country-picker-unnamed',
      childNodes: [{ nodeType: 3, textContent: tag }, note],
      textContent: `${tag}name not recorded`,
      matches: s => s === '.country-picker-unnamed' || s === 'span',
      querySelector: s => (s === 'small' ? note : null),
      querySelectorAll: s => (s === 'small' ? [note] : []),
      __children: [note],
    };
    const opt = makeEl({ tag: 'div', attrs: { role: 'option', 'aria-posinset': String(pos), 'aria-setsize': '3', 'aria-selected': 'false' } });
    opt.__children = [span];
    opt.scrollIntoView = () => {};
    return opt;
  };
  namedOpt.scrollIntoView = () => {};
  const unnamedA = makeUnnamed(2, 'AAA');
  const unnamedB = makeUnnamed(3, 'BBB');
  const list = {
    scrollTop: 0, clientHeight: 100, scrollHeight: 100,
    querySelector: s => (s === '[role=option]' ? namedOpt : null),
    querySelectorAll: s => (s === '[role=option]' ? [namedOpt, unnamedA, unnamedB] : []),
  };
  const host = loadProbe({ querySelector: s => (s.includes('country-picker-list') ? list : null) });
  const pump = async (promise, limit = 10) => {
    let settled = false, value;
    promise.then(v => { settled = true; value = v; });
    for (let i = 0; i < limit && !settled; i++) {
      await Promise.resolve();
      host.stepFrame(16);
      await Promise.resolve();
    }
    assert.equal(settled, true, 'probe async helper must settle');
    return value;
  };
  const rows = await pump(host.probe.countryRows());
  assert.equal(rows.size, 3);
  assert.deepEqual([...rows.rows].map(r => ({ name: r.name, tag: r.tag, selected: r.selected })), [
    { name: 'Germany', tag: 'GER', selected: true },
    { name: '', tag: 'AAA', selected: false },
    { name: '', tag: 'BBB', selected: false },
  ]);

  assert.equal(await pump(host.probe.revealCountry('BBB')), true);
});
