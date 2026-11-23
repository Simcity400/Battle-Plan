import assert from 'node:assert/strict';
import test from 'node:test';
import { AuditSnapshotStore } from '../audit.ts';
import type { FamilyTheme } from '../contract.ts';
import { createBoundAuditReader, type BoundAuditEntry } from './auditBinding.ts';
import { handleFamilyKeyDown, type FamilyNavTuple } from './keyboard.ts';
import { optionElementId, reconcileFocusedKey, syncListboxMirror } from './mirror.ts';
import { parseDefaultRowHit } from './pointer.ts';
import { computeScrollGeometry, logicalToPhysicalTop, physicalToLogicalTop } from './scroll.ts';
import { reportFamilyShownHour } from './shownHour.ts';
import { reducedMotionEnabled, sameFamilyTheme } from './theme.ts';

const mkEvent = (key: string, extra: Partial<KeyboardEvent> = {}) => {
  let prevented = false;
  const event = {
    key,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    ...extra,
    preventDefault() { prevented = true; },
  } as unknown as KeyboardEvent;
  return { event, prevented: () => prevented };
};

test('compressed scroll geometry preserves linear range and maps large extents accurately', () => {
  const linear = computeScrollGeometry(2400, 400, 8_000_000);
  assert.equal(linear.compressed, false);
  assert.equal(linear.physicalHeight, 2400);
  assert.equal(physicalToLogicalTop(600, linear), 600);
  assert.equal(logicalToPhysicalTop(600, linear), 600);

  const huge = computeScrollGeometry(40_000_000, 500, 8_000_000);
  assert.equal(huge.compressed, true);
  assert.equal(huge.physicalHeight, 8_000_000);
  assert.equal(physicalToLogicalTop(0, huge), 0);
  assert.equal(physicalToLogicalTop(huge.physicalHeight - huge.viewportHeight, huge), huge.logicalHeight - huge.viewportHeight);
  const midPhysical = (huge.physicalHeight - huge.viewportHeight) / 2;
  const midLogical = physicalToLogicalTop(midPhysical, huge);
  assert.ok(Math.abs(midLogical - (huge.logicalHeight - huge.viewportHeight) / 2) < 1);
  assert.ok(Math.abs(logicalToPhysicalTop(midLogical, huge) - midPhysical) < 1);
});

test('reconcileFocusedKey respects fallbackToFirst flag', () => {
  assert.deepEqual(reconcileFocusedKey('b', ['a', 'b', 'c'], false), { key: 'b', present: true, index: 1 });
  assert.deepEqual(reconcileFocusedKey('evicted', ['a', 'b'], false), { key: null, present: false, index: -1 });
  assert.deepEqual(reconcileFocusedKey('evicted', ['a', 'b'], true), { key: 'a', present: false, index: 0 });
});

test('F7 & F10: paged keyboard sends first ArrowDown/ArrowUp from no focus to ordinal 0, and empty Escape returns early', () => {
  const ordinals: number[] = [];
  const down0 = mkEvent('ArrowDown');
  handleFamilyKeyDown(down0.event, {
    nav: [],
    totalCount: 500,
    focusedKey: null,
    pageStep: 10,
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusKey: () => {},
    onFocusOrdinal: ord => ordinals.push(ord),
    onActivate: () => {},
  });
  assert.equal(down0.prevented(), true);

  const up0 = mkEvent('ArrowUp');
  handleFamilyKeyDown(up0.event, {
    nav: [],
    totalCount: 500,
    focusedKey: null,
    pageStep: 10,
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusKey: () => {},
    onFocusOrdinal: ord => ordinals.push(ord),
    onActivate: () => {},
  });
  assert.equal(up0.prevented(), true);

  const end = mkEvent('End');
  handleFamilyKeyDown(end.event, {
    nav: [],
    totalCount: 500,
    focusedKey: null,
    pageStep: 10,
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusKey: () => {},
    onFocusOrdinal: ord => ordinals.push(ord),
    onActivate: () => {},
  });
  assert.deepEqual(ordinals, [0, 0, 499]);

  let unpinned = false;
  const esc = mkEvent('Escape');
  handleFamilyKeyDown(esc.event, {
    nav: [],
    focusedKey: null,
    pageStep: 10,
    blocked: false,
    isPinnedTooltip: () => true,
    onFocusKey: () => {},
    onActivate: () => {},
    onUnpinTooltip: () => { unpinned = true; },
  });
  assert.equal(esc.prevented(), false, 'F10: Escape with zero rows returns early without preventDefault');
  assert.equal(unpinned, false, 'F10: Escape with zero rows does not unpin');
});

test('handleFamilyKeyDown avoids activating row 0 when focus key was evicted and supports typeahead', () => {
  const nav: FamilyNavTuple[] = [
    ['GER', 'Germany', 0, -1, '', null],
    ['FRA', 'France', 1, -1, '', null],
    ['ITA', 'Italy', 2, -1, '', null],
  ];
  const activated: string[] = [];
  const focused: string[] = [];
  const typeaheadRef = { current: { query: '', until: 0 } };

  const enter = mkEvent('Enter');
  handleFamilyKeyDown(enter.event, {
    nav,
    focusedKey: 'evicted-key',
    fallbackToFirstOnActivate: false,
    pageStep: 5,
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusKey: key => focused.push(key),
    onActivate: entry => activated.push(entry[0]),
  });
  assert.equal(enter.prevented(), true);
  assert.deepEqual(activated, []);

  const search = mkEvent('i');
  handleFamilyKeyDown(search.event, {
    nav,
    focusedKey: 'GER',
    pageStep: 5,
    blocked: false,
    typeaheadRef,
    isPinnedTooltip: () => false,
    onFocusKey: key => focused.push(key),
    onActivate: () => {},
  });
  assert.equal(search.prevented(), true);
  assert.deepEqual(focused, ['ITA']);
});

test('F3: sameFamilyTheme compares drawn tokens by value and rejects changed tokens', () => {
  const base: FamilyTheme = {
    surface: '#111',
    text: '#eee',
    muted: '#888',
    line: '#333',
    lineSoft: '#222',
    accent: '#38f',
    blue: '#38f',
    red: '#f44',
    amber: '#fa0',
    green: '#2c5',
    grey: '#888',
    hatch: '#555',
    hatchEdge: '#444',
    font: 'Inter',
    fontSize: { small: 11, body: 13 },
    lineWidth: 1,
    hatchSpacing: 6,
    country: { GER: '#444' },
  };
  assert.equal(sameFamilyTheme(base, { ...base, fontSize: { small: 11, body: 13 }, country: { GER: '#444' } }), true);
  assert.equal(sameFamilyTheme(base, { ...base, surface: '#000' }), false);
  assert.equal(sameFamilyTheme(base, { ...base, fontSize: { small: 12, body: 13 } }), false);
  assert.equal(sameFamilyTheme(base, { ...base, country: { GER: '#555' } }), false);
});

test('F4: reducedMotionEnabled honors app dataset.motion and OS matchMedia independently', () => {
  const prevDoc = globalThis.document;
  const prevMatch = globalThis.matchMedia;
  try {
    let osMatches = false;
    const dataset: Record<string, string | undefined> = {};
    (globalThis as { document: unknown }).document = { documentElement: { dataset } };
    (globalThis as { matchMedia: unknown }).matchMedia = () => ({ matches: osMatches });

    assert.equal(reducedMotionEnabled(), false, 'both off -> false');
    dataset.motion = 'reduced';
    osMatches = false;
    assert.equal(reducedMotionEnabled(), true, 'app setting on, OS off -> true');
    dataset.motion = undefined;
    osMatches = true;
    assert.equal(reducedMotionEnabled(), true, 'app setting off, OS on -> true');
  } finally {
    (globalThis as { document: unknown }).document = prevDoc;
    (globalThis as { matchMedia: unknown }).matchMedia = prevMatch;
  }
});

test('F6: bound audit reader verifies an earlier snapshot with the verifier captured at creation time', async () => {
  const store = new AuditSnapshotStore<BoundAuditEntry<{ readonly tag: string }>>(16);
  const reader = createBoundAuditReader(store);
  const calls: string[] = [];
  const idA = store.create({
    snapshot: { tag: 'snapA' },
    verify: async (hour, snap) => {
      calls.push(`verifierA:${snap.tag}@${hour}`);
      return { values: new Map([['row', 'A']]) };
    },
  });
  const idB = store.create({
    snapshot: { tag: 'snapB' },
    verify: async (hour, snap) => {
      calls.push(`verifierB:${snap.tag}@${hour}`);
      return { values: new Map([['row', 'B']]) };
    },
  });
  const resA = await reader(100, idA);
  const resB = await reader(200, idB);
  assert.deepEqual(calls, ['verifierA:snapA@100', 'verifierB:snapB@200']);
  assert.equal(resA?.values.get('row'), 'A');
  assert.equal(resB?.values.get('row'), 'B');
});

test('F8 & F9: listbox mirror uses stable key-based IDs, invalidates on prefix change, and honors explicit selected state', () => {
  const prevDoc = globalThis.document;
  try {
    interface MockEl {
      id: string;
      textContent: string;
      dataset: Record<string, string>;
      attrs: Map<string, string>;
      children: MockEl[];
      setAttribute(name: string, value: string): void;
      removeAttribute(name: string): void;
      replaceChildren(...nodes: MockEl[]): void;
      append(...nodes: MockEl[]): void;
    }
    const makeEl = (): MockEl => ({
      id: '',
      textContent: '',
      dataset: {},
      attrs: new Map(),
      children: [],
      setAttribute(name, value) { this.attrs.set(name, value); },
      removeAttribute(name) { this.attrs.delete(name); },
      replaceChildren(...nodes) {
        this.children = [];
        for (const n of nodes) {
          if (n.children.length > 0 && !n.attrs.has('role')) this.children.push(...n.children);
          else this.children.push(n);
        }
      },
      append(...nodes) { this.children.push(...nodes); },
    });
    (globalThis as { document: unknown }).document = {
      createElement: () => makeEl(),
      createDocumentFragment: () => makeEl(),
    };

    const listbox = makeEl();
    const lastMirrorKeyRef = { current: '' };
    const items = [
      { key: 'row:1', spoken: 'Alpha', index: 1, selected: true },
      { key: 'row_1', spoken: 'Beta', selected: false },
      { key: 'row:2', spoken: 'Gamma' },
    ];
    const first = syncListboxMirror(listbox as unknown as HTMLElement, items, {
      optionIdPrefix: 'p1',
      total: 3,
      focusedKey: 'row_1',
      lastMirrorKeyRef,
    });
    assert.equal(listbox.children.length, 3);
    const ids = listbox.children.map(el => el.id);
    assert.equal(new Set(ids).size, 3, 'F8: mixed indexed/unindexed and punctuation-similar keys produce unique IDs');
    assert.equal(first.activeDescendantId, optionElementId('p1', 'row_1'));
    assert.equal(listbox.attrs.get('aria-activedescendant'), optionElementId('p1', 'row_1'));
    assert.equal(listbox.children[0]!.attrs.get('aria-selected'), 'true', 'F9: explicit selected:true honored');
    assert.equal(listbox.children[1]!.attrs.get('aria-selected'), 'false', 'F9: explicit selected:false honored even when focused');
    assert.equal(listbox.children[2]!.attrs.get('aria-selected'), 'false', 'F9: unspecified selected defaults to false when unfocused');

    const second = syncListboxMirror(listbox as unknown as HTMLElement, items, {
      optionIdPrefix: 'p2',
      total: 3,
      focusedKey: 'row:2',
      lastMirrorKeyRef,
    });
    assert.equal(second.activeDescendantId, optionElementId('p2', 'row:2'));
    assert.equal(listbox.children[2]!.id, optionElementId('p2', 'row:2'), 'F8: prefix change rebuilds DOM option IDs');
    assert.equal(listbox.children[2]!.attrs.get('aria-selected'), 'true', 'F9: unspecified selected defaults to true when focused');
  } finally {
    (globalThis as { document: unknown }).document = prevDoc;
  }
});

test('parseDefaultRowHit distinguishes row and toggle hits', () => {
  assert.deepEqual(parseDefaultRowHit('row:GER'), { key: 'GER', toggle: false });
  assert.deepEqual(parseDefaultRowHit('toggle:grp-1'), { key: 'grp-1', toggle: true });
  assert.equal(parseDefaultRowHit('header:col'), null);
  assert.equal(parseDefaultRowHit(null), null);
});

test('reportFamilyShownHour prefers explicit installedHour over view.through', () => {
  const reported: (number | null)[] = [];
  const sink = (hour: number | null) => { reported.push(hour); };
  assert.equal(reportFamilyShownHour(sink, true, { mode: 'picture', target: 100, through: 90, painted: 88, dim: false, tooltip: null }, 75), 75);
  assert.equal(reportFamilyShownHour(sink, true, { mode: 'picture', target: 100, through: 90, painted: 88, dim: false, tooltip: null }), 88);
  assert.equal(reportFamilyShownHour(sink, false, { mode: 'reading', target: 100, dim: false, tooltip: null }), null);
  assert.deepEqual(reported, [75, 90, null]);
});
