import assert from 'node:assert/strict';
import test from 'node:test';
import { AuditSnapshotStore } from '../audit.ts';
import type { FamilyTheme } from '../contract.ts';
import { createBoundAuditReader, type BoundAuditEntry } from './auditBinding.ts';
import { handleFamilyKeyDown, type FamilyNavTuple } from './keyboard.ts';
import { optionElementId, reconcileFocusedKey, syncListboxMirror, syncTableMirror } from './mirror.ts';
import { parseDefaultRowHit } from './pointer.ts';
import { computeScrollGeometry, logicalToPhysicalTop, physicalToLogicalTop } from './scroll.ts';
import { engineHour } from '../../dashboard/calendar.ts';
import { reportFamilyShownHour } from './shownHour.ts';
import { reducedMotionEnabled, sameFamilyTheme } from './theme.ts';
import { buildTooltipContent, resolveTooltipEntryContent, type FamilyActionPayload, type FamilyTooltipEntry } from './tooltips.ts';
import { centerWindowPosition, clampWindowPosition } from './windowPlacement.ts';

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

test('tooltip payloads carry full truncated text, exact hours, date bounds, missing reasons, and structured lists', () => {
  const hour = engineHour({ year: 1941, month: 9, day: 14, hour: 8 })!;
  const entry: FamilyTooltipEntry = {
    hour,
    rowKey: 'GER:battle:1',
    payload: {
      title: 'Battle of Smol…',
      fullText: 'Battle of Smolensk',
      dateBound: { kind: 'by', hour, reason: 'First seen after a recording gap.' },
      exactValue: '124,850',
      missingReason: 'Air losses not recorded in this build.',
      items: [
        { label: 'Germany', value: '82,100', swatch: '#4f86f7' },
        { label: 'Soviet Union', missingReason: 'Not sampled on this hour' },
      ],
    },
  };
  const content = resolveTooltipEntryContent(entry);
  assert.equal(content.title, 'Battle of Smolensk');
  assert.equal(content.subtitle, 'By 14 Sep 1941 · 08:00');
  assert.equal(content.note, 'First seen after a recording gap.');
  assert.equal(content.rows[0]?.value, '124,850');
  assert.equal(content.rows[1]?.valueState?.kind, 'missing');
  assert.equal(content.rows[2]?.label, 'Germany');
  assert.equal(content.rows[3]?.valueState?.kind, 'missing');

  const dated = buildTooltipContent({ title: 'Factory count', hour });
  assert.equal(dated.subtitle, '14 Sep 1941 · 08:00');

  const action: FamilyActionPayload = {
    actionId: 'open-battle',
    kind: 'battle',
    subject: 'battle:42',
    hour,
    rowKey: 'GER:battle:42',
  };
  assert.equal(action.subject, 'battle:42');
  assert.equal(action.hour, hour);
});

test('window placement clamps and centres within viewport bounds and sameFamilyTheme detects presentation token changes', () => {
  assert.deepEqual(centerWindowPosition(400, 300, 1200, 800), { left: 400, top: 250 });
  assert.deepEqual(clampWindowPosition(-50, 999, 400, 300, 1200, 800), { left: 12, top: 488 });
  assert.deepEqual(clampWindowPosition(200, 200, 2000, 1500, 800, 600), { left: 12, top: 12 });

  const base: FamilyTheme = {
    surface: '#111',
    text: '#eee',
    muted: '#888',
    line: '#333',
    lineSoft: '#222',
    accent: '#38f',
    blue: '#38f',
    red: '#f44',
    shortage: '#ff6b6f',
    amber: '#fa0',
    green: '#2c5',
    grey: '#888',
    hatch: '#555',
    hatchEdge: '#444',
    font: 'Inter',
    fontSize: { small: 11, body: 13 },
    lineWidth: 1,
    hatchSpacing: 6,
  };
  assert.equal(sameFamilyTheme(base, { ...base, shortage: '#e5484d' }), false);
});

test('syncTableMirror updates existing table rows and cells in place without recreating nodes', () => {
  const prevDoc = (globalThis as { document?: unknown }).document;
  try {
    let createdElements = 0;
    interface MockCell {
      textContent: string;
      tabIndex: number;
      dataset: Record<string, string>;
      attrs: Map<string, string>;
      children: MockCell[];
      readonly lastElementChild: MockCell | null;
      getAttribute(name: string): string | null;
      hasAttribute(name: string): boolean;
      setAttribute(name: string, value: string): void;
      removeAttribute(name: string): void;
      appendChild(node: MockCell): MockCell;
      append(...nodes: MockCell[]): void;
      replaceChildren(...nodes: MockCell[]): void;
      contains(node: unknown): boolean;
      remove(): void;
    }
    const makeNode = (isFrag = false): MockCell => {
      if (!isFrag) createdElements++;
      const node: MockCell = {
        textContent: '',
        tabIndex: -1,
        dataset: {},
        attrs: new Map(),
        children: [],
        get lastElementChild() { return this.children[this.children.length - 1] ?? null; },
        getAttribute(name) { return this.attrs.get(name) ?? null; },
        hasAttribute(name) { return this.attrs.has(name); },
        setAttribute(name, value) { this.attrs.set(name, value); },
        removeAttribute(name) { this.attrs.delete(name); },
        appendChild(child) {
          const items = child.attrs.get('__frag') === '1' ? child.children : [child];
          for (const item of items) {
            this.children.push(item);
            item.remove = () => {
              const idx = this.children.indexOf(item);
              if (idx >= 0) this.children.splice(idx, 1);
            };
          }
          return child;
        },
        append(...nodes) { for (const n of nodes) this.appendChild(n); },
        replaceChildren(...nodes) { this.children = []; for (const n of nodes) this.appendChild(n); },
        contains() { return false; },
        remove() {},
      };
      if (isFrag) node.attrs.set('__frag', '1');
      return node;
    };
    (globalThis as { document: unknown }).document = {
      createElement: () => makeNode(false),
      createDocumentFragment: () => makeNode(true),
      activeElement: null,
    };
    const body = makeNode();
    const opts = {
      selectedKey: null,
      focusedKeyRef: { current: null },
      focusInsideRef: { current: false },
      restoringFocusRef: { current: false },
      lastMirrorKeyRef: { current: '' },
    };
    syncTableMirror(body as unknown as HTMLTableSectionElement, {
      first: 0,
      total: 2,
      rows: [
        { key: 'GER', name: 'Germany', rowIndex: 0, rank: 1, cells: [{ column: 'name', spoken: 'Germany' }, { column: 'val', spoken: '100' }] },
        { key: 'FRA', name: 'France', rowIndex: 1, rank: 2, cells: [{ column: 'name', spoken: 'France' }, { column: 'val', spoken: '80' }] },
      ],
    }, opts);
    const createdFirst = createdElements;
    const tr0 = body.children[0]!;
    const td2 = tr0.children[2]!;
    assert.equal(td2.textContent, '100');

    syncTableMirror(body as unknown as HTMLTableSectionElement, {
      first: 0,
      total: 2,
      rows: [
        { key: 'GER', name: 'Germany', rowIndex: 0, rank: 1, cells: [{ column: 'name', spoken: 'Germany' }, { column: 'val', spoken: '105' }] },
        { key: 'FRA', name: 'France', rowIndex: 1, rank: 2, cells: [{ column: 'name', spoken: 'France' }, { column: 'val', spoken: '80' }] },
      ],
    }, opts);
    assert.equal(createdElements, createdFirst, 'updating values reuses all existing tr and td nodes');
    assert.equal(body.children[0], tr0);
    assert.equal(tr0.children[2], td2);
    assert.equal(td2.textContent, '105');

    syncTableMirror(body as unknown as HTMLTableSectionElement, {
      first: 0,
      total: 1,
      rows: [{ key: 'GER', name: 'Germany', rowIndex: 0, rank: 1, cells: [{ column: 'name', spoken: 'Germany' }, { column: 'val', spoken: '105' }] }],
    }, opts);
    assert.equal(body.children.length, 1, 'shrinking rows trims trailing tr nodes');
  } finally {
    (globalThis as { document: unknown }).document = prevDoc;
  }
});

