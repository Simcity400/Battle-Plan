import assert from 'node:assert/strict';
import test from 'node:test';
import { tooltipStore } from '../../charts/tooltip.ts';
import { engineHour } from '../../dashboard/calendar.ts';
import type { Pipeline, PointerHitMeta } from '../../dashboard/follow/pipeline.ts';
import { AuditSnapshotStore } from '../audit.ts';
import { barsHostAdapter, parseBarsHit } from '../bars/hostAdapter.ts';
import { displayedHour } from '../boundedView.ts';
import type { FamilyTheme, InstalledAction, InstalledPicture } from '../contract.ts';
import { heatLanesHostAdapter, heatMatrixHostAdapter, parseHeatLanesHit, parseHeatMatrixHit } from '../heat/hostAdapter.ts';
import { parseTimeChartHit, timeChartHostAdapter } from '../time-chart/hostAdapter.ts';
import { lazyActionKeys, type ParsedFamilyHit } from './adapter.ts';
import { createBoundAuditReader, type BoundAuditEntry } from './auditBinding.ts';
import { boundDated } from './dated.ts';
import { NO_EXPANSION, toggleExpansion } from './expansion.ts';
import {
  handleFamilyGridKeyDown,
  handleFamilyKeyDown,
  handleFamilyPointsKeyDown,
  type FamilyNavTuple,
} from './keyboard.ts';
import { optionElementId, reconcileFocusedKey, syncListboxMirror, syncTableMirror } from './mirror.ts';
import { bindFamilyPointer, CLICK_CANCEL_PX, parseDefaultRowHit } from './pointer.ts';
import { computeScrollGeometry, logicalToPhysicalTop, physicalToLogicalTop } from './scroll.ts';
import { reportFamilyShownHour } from './shownHour.ts';
import { reducedMotionEnabled, sameFamilyTheme } from './theme.ts';
import {
  buildTooltipContent,
  FamilyTooltipController,
  resolveTooltipEntryContent,
  type FamilyActionPayload,
  type FamilyTooltipEntry,
} from './tooltips.ts';
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
    pageStep: 5,
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusKey: key => focused.push(key),
    onActivate: entry => activated.push(entry[0]),
  });
  assert.equal(enter.prevented(), true);
  assert.deepEqual(activated, [], 'Enter on an evicted row activates nothing, never the first remaining row');
  const tree: FamilyNavTuple[] = [['GRP', 'Group', 0, 0, '', null], ['GER', 'Germany', 1, -1, 'GRP', null]];
  const toggled: string[] = [];
  for (const key of ['ArrowRight', 'ArrowLeft']) {
    handleFamilyKeyDown(mkEvent(key).event, { nav: tree, focusedKey: 'evicted-key', pageStep: 5, blocked: false, isPinnedTooltip: () => false, onFocusKey: k => focused.push(k), onActivate: () => {}, onToggleTree: k => toggled.push(k) });
  }
  assert.deepEqual([toggled, focused], [[], []], 'tree keys on an evicted row expand or move nothing');

  const keyOptions = (focusedKey: string, blocked: boolean) => ({
    nav,
    focusedKey,
    pageStep: 5,
    blocked,
    isPinnedTooltip: () => false,
    onFocusKey: (key: string) => (focused as string[]).push(key),
    onActivate: (entry: FamilyNavTuple) => (activated as string[]).push(entry[0]),
  });
  handleFamilyKeyDown(mkEvent('Enter').event, keyOptions('FRA', true));
  assert.deepEqual(activated, [], 'a blocked host activates nothing');
  handleFamilyKeyDown(mkEvent(' ').event, keyOptions('FRA', false));
  assert.deepEqual(activated, ['FRA'], 'Space activates the focused row');
  activated.length = 0;

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

test('toggleExpansion flips the installed tree state of a row and ignores rows that are not trees', () => {
  const nav: FamilyNavTuple[] = [['war:1', 'War one', 0, 1, ''], ['war:2', 'War two', 1, 0, ''], ['war:2:GER', 'German Reich', 2, -1, 'war:2']];
  const closed = toggleExpansion(NO_EXPANSION, nav, 'war:1');
  assert.deepEqual(closed, { next: { 'war:1': false }, open: false, label: 'War one' });
  const opened = toggleExpansion(closed!.next, nav, 'war:2');
  assert.deepEqual(opened?.next, { 'war:1': false, 'war:2': true });
  assert.equal(toggleExpansion(NO_EXPANSION, nav, 'war:2:GER'), null);
  assert.equal(toggleExpansion(NO_EXPANSION, nav, 'missing'), null);
  assert.deepEqual(toggleExpansion(NO_EXPANSION, nav, 'war:2', true)?.next, { 'war:1': false, 'war:2': true }, 'exclusive: opening one closes the one that was open');
  assert.deepEqual(toggleExpansion(NO_EXPANSION, nav, 'war:1', true)?.next, { 'war:1': false }, 'closing touches nothing else');
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
      focus(): void;
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
        focus() { (globalThis as unknown as { document: { activeElement: unknown } }).document.activeElement = node; },
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
      rowsFocusable: true,
      focusedKeyRef: { current: null as string | null },
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

    const rows = (keys: readonly string[]) => ({ first: 0, total: keys.length, rows: keys.map((key, rowIndex) => ({ key, name: key, rowIndex, cells: [] })) });
    syncTableMirror(body as unknown as HTMLTableSectionElement, rows(['GER', 'FRA']), { ...opts, focusedKeyRef: { current: 'GER' } });
    const focusedRef = { current: 'GER' as string | null };
    const prevElement = (globalThis as { HTMLElement?: unknown }).HTMLElement;
    (globalThis as { HTMLElement?: unknown }).HTMLElement = Object;
    body.contains = () => true;
    body.children[0]!.focus();
    const moved = syncTableMirror(body as unknown as HTMLTableSectionElement, rows(['FRA']), { ...opts, focusedKeyRef: focusedRef, focusInsideRef: { current: true } });
    assert.equal(moved, true, 'when the focused row goes, the mirror reports that it moved focus');
    assert.equal(focusedRef.current, 'FRA', 'and the host focus key follows the row that took DOM focus');
    (globalThis as { HTMLElement?: unknown }).HTMLElement = prevElement;

    const bodyFocus = makeNode();
    syncTableMirror(bodyFocus as unknown as HTMLTableSectionElement, rows(['GER', 'FRA']), { ...opts, rowsFocusable: false, lastMirrorKeyRef: { current: '' } });
    assert.ok(bodyFocus.children.every(tr => !tr.hasAttribute('tabindex') && tr.tabIndex === -1), 'a mirror of a body-focused family has no tabbable rows outside the keyboard handler');
  } finally {
    (globalThis as { document: unknown }).document = prevDoc;
  }
});

test('bindFamilyPointer resolves actions against the installed picture, rejects stale owners, separates click from drag pan, and keeps continuous hover flicker-free', () => {
  type Listener = (event: unknown) => void;
  const makeTarget = () => {
    const listeners = new Map<string, Set<Listener>>();
    return {
      addEventListener(type: string, fn: Listener) {
        let set = listeners.get(type);
        if (!set) listeners.set(type, set = new Set());
        set.add(fn);
      },
      removeEventListener(type: string, fn: Listener) {
        listeners.get(type)?.delete(fn);
      },
      dispatch(type: string, event: Record<string, unknown>) {
        for (const fn of listeners.get(type) ?? []) fn(event);
      },
      getBoundingClientRect: () => ({ left: 10, top: 20, width: 400, height: 200 }),
      contains: () => false,
    };
  };

  const body = makeTarget();
  const menuHost = makeTarget();
  const nav: readonly FamilyNavTuple[] = [['GER', 'Germany', 0, -1, '', { tag: 'GER' }]];
  let installed: InstalledPicture | null = {
    id: 7,
    owner: 'owner-A',
    generation: { seek: 1, reset: 0, binding: 1 },
    factRevisions: {},
    validInterval: [100, 120],
    through: 100,
    hour: displayedHour(100),
    viewport: { width: 200, height: 100, dpr: 1, scrollLeft: 0, scrollTop: 0 },
    optionsRevision: '',
    themeRevision: '',
    actionKeys: lazyActionKeys(nav, displayedHour(100)),
    accessibleDescription: null,
  };

  let hitReply: { key: string | null; meta?: PointerHitMeta } = { key: 'row:GER', meta: { installedId: 7, owner: 'owner-A' } };
  const inspectedTokens: (string | null)[] = [];
  const pointerPoints: (readonly [number, number] | null)[] = [];
  const fakePipeline = {
    pointer(_pipe: number, _slot: number, pt: readonly [number, number] | null, cb?: (key: string | null, meta?: PointerHitMeta) => void) {
      pointerPoints.push(pt);
      cb?.(hitReply.key, hitReply.meta);
    },
    inspect(_pipe: number, token: string | null) {
      inspectedTokens.push(token);
    },
    view() { return null; },
  } as unknown as Pipeline;

  const tooltips = new FamilyTooltipController('test-host');
  const activations: InstalledAction[] = [];
  const hoverHits: (ParsedFamilyHit | null)[] = [];
  const dragEvents: string[] = [];
  let doubleClicks = 0;

  const stop = bindFamilyPointer({
    bodyNode: body as unknown as HTMLElement,
    menuHost: menuHost as unknown as HTMLElement,
    pipeline: fakePipeline,
    pipe: 1,
    tooltips,
    inspectedKeyRef: { current: null },
    inspectedHitRef: { current: null },
    lastPointerRef: { current: null },
    focusInsideRef: { current: false },
    focusedKeyRef: { current: null },
    focusAnchorRef: { current: { x: 0, y: 0 } },
    accessibleRef: { current: null },
    pointerMode: 'continuous',
    isBlocked: () => !installed,
    getInstalled: () => installed,
    getViewportHeight: () => 800,
    parseHit: (hitKey, inst) => parseTimeChartHit(hitKey) ?? parseDefaultRowHit(hitKey) ?? (inst ? null : null),
    onHoverChange: hit => hoverHits.push(hit),
    gestures: {
      onDragStart: () => dragEvents.push('start'),
      onDragMove: (_pt, delta) => dragEvents.push(`move:${delta[0]},${delta[1]}`),
      onDragEnd: () => dragEvents.push('end'),
      onDoubleClick: () => { doubleClicks++; return true; },
    },
    onActivate: (_key, _toggle, action) => { if (action) activations.push(action); },
  });

  body.dispatch('pointerdown', { button: 0, clientX: 110, clientY: 70, pointerId: 1 });
  body.dispatch('pointerup', { button: 0, clientX: 110, clientY: 70, pointerId: 1 });
  assert.deepEqual(pointerPoints[0], [50, 25], 'client point scales through installed viewport (200x100 vs DOM 400x200)');
  assert.equal(activations.length, 1);
  assert.equal(activations[0]!.installedId, 7);
  assert.equal(activations[0]!.owner, 'owner-A');
  assert.equal(activations[0]!.subject, 'GER');
  assert.equal(activations[0]!.hour, 100);

  hitReply = { key: 'row:GER', meta: { installedId: 6, owner: 'owner-OLD' } };
  body.dispatch('pointerdown', { button: 0, clientX: 110, clientY: 70, pointerId: 1 });
  body.dispatch('pointerup', { button: 0, clientX: 110, clientY: 70, pointerId: 1 });
  assert.equal(activations.length, 1, 'stale owner/installedId hit is rejected');

  hitReply = { key: 'plot:150', meta: { installedId: 7, owner: 'owner-A' } };
  body.dispatch('pointerdown', { button: 0, clientX: 100, clientY: 70, pointerId: 2 });
  body.dispatch('pointermove', { isTrusted: true, clientX: 100 + CLICK_CANCEL_PX + 10, clientY: 70, pointerId: 2 });
  body.dispatch('pointerup', { button: 0, clientX: 100 + CLICK_CANCEL_PX + 10, clientY: 70, pointerId: 2 });
  assert.equal(activations.length, 1, 'drag past CLICK_CANCEL_PX cancels click activation');
  assert.deepEqual(dragEvents, ['start', `move:${CLICK_CANCEL_PX + 10},0`, 'end']);

  let dblPrevented = false;
  body.dispatch('dblclick', { clientX: 100, clientY: 70, preventDefault() { dblPrevented = true; } });
  assert.equal(doubleClicks, 1);
  assert.equal(dblPrevented, true);

  hitReply = { key: 'line:GER@150', meta: { installedId: 7, owner: 'owner-A' } };
  body.dispatch('pointermove', { isTrusted: true, clientX: 120, clientY: 80 });
  hitReply = { key: 'line:GER@151', meta: { installedId: 7, owner: 'owner-A' } };
  body.dispatch('pointermove', { isTrusted: true, clientX: 122, clientY: 80 });
  assert.deepEqual(hoverHits.slice(-2).map(h => h?.hour), [150, 151]);
  assert.equal(inspectedTokens.at(-1), 'line:GER@151');

  const tip150: FamilyTooltipEntry = { hour: 150, rowKey: 'line:GER@150', payload: { title: 'Germany', hour: 150 } };
  tooltips.syncFromInstall({
    dim: false,
    held: false,
    inspectedKey: 'line:GER@150',
    inspectedToken: 'line:GER@150',
    pointerPos: { x: 120, y: 80 },
    hoverTip: tip150,
    hoverFieldKey: 'line:GER@150',
    keepHoverWhilePending: true,
    focusInside: false,
    focusedKey: null,
    focusAnchor: { x: 0, y: 0 },
    focusTip: null,
    viewportHeight: 800,
  });
  assert.equal(tooltipStore.get()?.content.title, 'Germany');
  tooltips.syncFromInstall({
    dim: false,
    held: false,
    inspectedKey: 'line:GER@151',
    inspectedToken: 'line:GER@151',
    pointerPos: { x: 122, y: 80 },
    hoverTip: tip150,
    hoverFieldKey: 'line:GER@150',
    keepHoverWhilePending: true,
    focusInside: false,
    focusedKey: null,
    focusAnchor: { x: 0, y: 0 },
    focusTip: null,
    viewportHeight: 800,
  });
  assert.equal(tooltipStore.get()?.content.title, 'Germany', 'continuous hover keeps previous tooltip while adjacent hour renders');
  tooltips.syncFromInstall({
    dim: false,
    held: false,
    inspectedKey: 'line:GER@151',
    inspectedToken: 'line:GER@151',
    pointerPos: { x: 122, y: 80 },
    hoverTip: null,
    hoverFieldKey: 'line:GER@151',
    keepHoverWhilePending: true,
    focusInside: false,
    focusedKey: null,
    focusAnchor: { x: 0, y: 0 },
    focusTip: null,
    viewportHeight: 800,
  });
  assert.equal(tooltipStore.get(), null, 'explicit null tooltip for current token hides immediately');

  stop.clearHover();
  assert.equal(inspectedTokens.at(-1), null, 'a zoom or pan drops the inspected hour under a still pointer');
  assert.equal(hoverHits.at(-1), null, 'and publishes no hover hour');

  stop();
  tooltips.dispose();
});

test('grid and points keyboard navigation and family host adapters cover all four ported families', () => {
  const cells: string[] = [];
  const gridActivated: string[] = [];
  handleFamilyGridKeyDown(mkEvent('ArrowRight').event, {
    rows: 3,
    columns: 4,
    current: { row: 1, column: 1 },
    pageStep: 2,
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusCell: (r, c) => cells.push(`${r}:${c}`),
    onActivate: (r, c) => gridActivated.push(`${r}:${c}`),
  });
  handleFamilyGridKeyDown(mkEvent('End', { ctrlKey: true }).event, {
    rows: 3,
    columns: 4,
    current: { row: 1, column: 1 },
    pageStep: 2,
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusCell: (r, c) => cells.push(`${r}:${c}`),
    onActivate: (r, c) => gridActivated.push(`${r}:${c}`),
  });
  handleFamilyGridKeyDown(mkEvent('Enter').event, {
    rows: 3,
    columns: 4,
    current: { row: 2, column: 3 },
    pageStep: 2,
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusCell: (r, c) => cells.push(`${r}:${c}`),
    onActivate: (r, c) => gridActivated.push(`${r}:${c}`),
  });
  assert.deepEqual(cells, ['1:2', '2:3']);
  assert.deepEqual(gridActivated, ['2:3']);

  const pointsNav: FamilyNavTuple[] = [
    ['plot:100', 'Jan 1936', 0, -1, ''],
    ['plot:200', 'Feb 1936', 1, -1, ''],
    ['plot:300', 'Mar 1936', 2, -1, ''],
  ];
  const focusedPts: string[] = [];
  const activatedPts: string[] = [];
  handleFamilyPointsKeyDown(mkEvent('ArrowRight').event, {
    nav: pointsNav,
    focusedKey: 'plot:100',
    pageStep: 2,
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusKey: key => focusedPts.push(key),
    onActivate: entry => activatedPts.push(entry[0]),
  });
  handleFamilyPointsKeyDown(mkEvent('End').event, {
    nav: pointsNav,
    focusedKey: 'plot:100',
    pageStep: 2,
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusKey: key => focusedPts.push(key),
    onActivate: entry => activatedPts.push(entry[0]),
  });
  handleFamilyPointsKeyDown(mkEvent('Enter').event, {
    nav: pointsNav,
    focusedKey: 'plot:200',
    pageStep: 2,
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusKey: key => focusedPts.push(key),
    onActivate: entry => activatedPts.push(entry[0]),
  });
  assert.deepEqual(focusedPts, ['plot:200', 'plot:300']);
  assert.deepEqual(activatedPts, ['plot:200']);

  assert.deepEqual(parseBarsHit('right:GER'), { key: 'GER', subject: 'GER', kind: 'right-row', toggle: false, inspectToken: 'right:GER' });
  assert.deepEqual(parseBarsHit('remove:reg1'), { key: 'reg1', subject: 'reg1', kind: 'remove-region', toggle: true, inspectToken: 'remove:reg1' });
  assert.deepEqual(parseHeatMatrixHit('cell:2:5'), { key: '2:5', subject: '2:5', toggle: false, inspectToken: 'cell:2:5' });
  assert.equal(parseHeatLanesHit('lane:#0@144')?.hour, 144);
  assert.equal(parseTimeChartHit('line:GER@512')?.hour, 512);
  assert.equal(parseTimeChartHit('line:GER@512')?.subject, 'GER');
  assert.equal(barsHostAdapter.family, 'bars');
  assert.equal(heatMatrixHostAdapter.family, 'heat-matrix');
  assert.equal(heatLanesHostAdapter.family, 'heat-lanes');
  assert.equal(timeChartHostAdapter.family, 'time-chart');

  let zoomWindow: readonly [number, number] | null = null;
  const gCtx = {
    fields: { meta: { plotRect: { left: 50, top: 10, width: 400, height: 200 }, transform: { xFrom: 0, xTo: 1000, axisFrom: 0, axisTo: 1000 } } },
    viewport: { width: 500, height: 240, dpr: 1, scrollLeft: 0, scrollTop: 0, zoomWindow: null as readonly [number, number] | null },
    extent: [0, 1000] as readonly [number, number],
    setZoomWindow: (next: readonly [number, number] | null) => { zoomWindow = next; },
  };
  assert.equal(timeChartHostAdapter.gestures!.onWheel!({ deltaY: -120 } as WheelEvent, [250, 100], gCtx), true);
  assert.ok(zoomWindow !== null && zoomWindow[1] - zoomWindow[0] < 1000, 'wheel in zooms window');
  assert.equal(timeChartHostAdapter.gestures!.onDoubleClick!([250, 100], { ...gCtx, viewport: { ...gCtx.viewport, zoomWindow } }), true);
  assert.equal(zoomWindow, null, 'double-click resets zoom window');
});


test('a press, release or menu waits for its own hit and acts only on the picture that hit, with a delayed worker', () => {
  type Listener = (event: unknown) => void;
  const listeners = new Map<string, Set<Listener>>();
  const body = {
    addEventListener(type: string, fn: Listener) { (listeners.get(type) ?? listeners.set(type, new Set()).get(type)!).add(fn); },
    removeEventListener(type: string, fn: Listener) { listeners.get(type)?.delete(fn); },
    dispatch(type: string, event: Record<string, unknown>) { for (const fn of listeners.get(type) ?? []) fn({ target: body, preventDefault() {}, ...event }); },
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 200, height: 100 }),
    contains: (node: unknown) => node === body,
  };
  const picture = (id: number, keys: readonly string[], owner = 'owner-A'): InstalledPicture => ({
    id,
    owner,
    generation: { seek: 1, reset: 0, binding: 1 },
    factRevisions: {},
    validInterval: [100, 120],
    through: 100,
    hour: displayedHour(100 + id),
    viewport: { width: 200, height: 100, dpr: 1, scrollLeft: 0, scrollTop: 0 },
    optionsRevision: '',
    themeRevision: '',
    actionKeys: lazyActionKeys(keys.map((key, i) => [key, key, i, -1, '', null] as FamilyNavTuple), displayedHour(100 + id)),
    accessibleDescription: null,
  });
  let installed = picture(7, ['GER']);
  const pending: ((key: string | null, meta?: PointerHitMeta) => void)[] = [];
  const fakePipeline = {
    pointer(_pipe: number, _slot: number, _pt: unknown, cb?: (key: string | null, meta?: PointerHitMeta) => void) { if (cb) pending.push(cb); },
    inspect() {},
    view() { return null; },
  } as unknown as Pipeline;
  const reply = (key: string | null, id: number, owner = 'owner-A') => pending.shift()!(key, { installedId: id, owner });
  const activations: InstalledAction[] = [];
  const menus: (string | null)[] = [];
  const inspectedKeyRef = { current: null as string | null };
  const tooltips = new FamilyTooltipController('delayed-worker');
  const stop = bindFamilyPointer({
    bodyNode: body as unknown as HTMLElement,
    menuHost: body as unknown as HTMLElement,
    pipeline: fakePipeline,
    pipe: 1,
    tooltips,
    inspectedKeyRef,
    inspectedHitRef: { current: null },
    lastPointerRef: { current: null },
    focusInsideRef: { current: false },
    focusedKeyRef: { current: null },
    focusAnchorRef: { current: { x: 0, y: 0 } },
    accessibleRef: { current: body as unknown as HTMLElement },
    isBlocked: () => false,
    getInstalled: () => installed,
    getViewportHeight: () => 800,
    parseHit: hit => parseDefaultRowHit(hit),
    onActivate: (_key, _toggle, action) => { if (action) activations.push(action); },
    onContextMenu: key => { menus.push(key); return true; },
  });

  body.dispatch('pointermove', { isTrusted: true, clientX: 20, clientY: 10 });
  reply('row:GER', 7);
  assert.equal(inspectedKeyRef.current, 'GER', 'GER hovered on picture 7');

  body.dispatch('pointerdown', { button: 0, clientX: 20, clientY: 10, pointerId: 1 });
  installed = picture(8, ['FRA']);
  body.dispatch('pointerup', { button: 0, clientX: 20, clientY: 10, pointerId: 1 });
  assert.equal(activations.length, 0, 'the cached hover is not a hit: nothing activates before this press resolves');
  reply('row:GER', 7);
  reply('row:FRA', 8);
  assert.equal(activations.length, 0, 'press on GER in picture 7 and release on FRA in picture 8 is not a click');

  body.dispatch('contextmenu', { clientX: 20, clientY: 10 });
  assert.deepEqual(menus, [], 'the menu waits for its own hit');
  installed = picture(9, ['ITA']);
  reply('row:FRA', 8);
  assert.deepEqual(menus, ['FRA'], 'a menu answers from the picture it was opened on, though the same owner repainted since');
  body.dispatch('contextmenu', { clientX: 20, clientY: 10 });
  reply('row:ITA', 9);
  assert.deepEqual(menus, ['FRA', 'ITA']);

  body.dispatch('pointerdown', { button: 0, clientX: 20, clientY: 10, pointerId: 2 });
  reply('row:ITA', 9);
  installed = picture(10, ['ENG']);
  body.dispatch('pointerup', { button: 0, clientX: 20, clientY: 10, pointerId: 2 });
  reply('row:ENG', 10);
  assert.equal(activations.length, 0, 'press on ITA and release on ENG is not a click');

  body.dispatch('pointerdown', { button: 0, clientX: 20, clientY: 10, pointerId: 3 });
  reply('row:ENG', 10);
  body.dispatch('pointerup', { button: 0, clientX: 20, clientY: 10, pointerId: 3 });
  installed = picture(11, ['ENG']);
  reply('row:ENG', 10);
  assert.equal(activations.length, 1, 'a valid release hit is kept when the same owner repaints before it arrives');
  assert.equal(activations[0]!.installedId, 10, 'and the action belongs to the picture the release hit');
  assert.equal(activations[0]!.hour, 110);

  body.dispatch('pointerdown', { button: 0, clientX: 20, clientY: 10, pointerId: 4 });
  reply('row:ENG', 11);
  body.dispatch('pointerup', { button: 0, clientX: 20, clientY: 10, pointerId: 4 });
  reply('row:ENG', 11);
  assert.equal(activations.length, 2);
  assert.equal(activations[1]!.installedId, 11);
  assert.equal(activations[1]!.subject, 'ENG');
  assert.equal(activations[1]!.hour, 111);

  body.dispatch('pointerdown', { button: 0, clientX: 20, clientY: 10, pointerId: 5 });
  reply('row:ENG', 11);
  installed = picture(12, ['ENG'], 'owner-B');
  body.dispatch('pointerup', { button: 0, clientX: 20, clientY: 10, pointerId: 5 });
  assert.equal(pending.length, 0, 'a press on owner A does not authorise a release on owner B');
  assert.equal(activations.length, 2);

  body.dispatch('pointerdown', { button: 0, clientX: 20, clientY: 10, pointerId: 6 });
  reply('row:ENG', 12, 'owner-B');
  body.dispatch('pointerup', { button: 0, clientX: 20, clientY: 10, pointerId: 6 });
  installed = picture(13, ['ENG'], 'owner-C');
  reply('row:ENG', 12, 'owner-B');
  assert.equal(activations.length, 2, 'an owner change before the release hit arrives refuses it');

  stop();
  tooltips.dispose();
});

test('a followed bounded read dates its picture by its proof even when the widget also has subscriptions', () => {
  const followed = { hour: 120, follows: true, provenInterval: [100, 140] as const };
  assert.deepEqual(boundDated(followed, true, false, null), { h: 100, c: 140 }, 'held back between a seek back and the new bounded reply');
  assert.equal(boundDated({ hour: 120 }, true, false, null), undefined, 'a pinned read beside subscriptions is dated by the stream');
  assert.deepEqual(boundDated({ hour: 120 }, false, false, null), { h: 120, c: 120 });
  assert.deepEqual(boundDated(null, false, true, [0, 500]), { h: 0, c: 500 });
});
