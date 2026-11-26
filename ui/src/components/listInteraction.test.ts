import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { SelectOption } from './Select';

test('windowed controls retain focus, sequential row access and nested expansion', async () => {
  let cursor = 0;
  const cells: any[] = [], layouts: (() => unknown)[] = [];
  const fixture = {
    rows: Array.from({ length: 5000 }, (_, slot) => ({ key: String(slot), item: slot, slot, phase: 'staying', since: 0 })),
    useId: () => 'synthetic-menu',
    useRef(value: unknown) { const at = cursor++; return cells[at] ??= { current: value }; },
    useState(value: unknown) { const at = cursor++; cells[at] ??= { value }; return [cells[at].value, (next: any) => { cells[at].value = typeof next === 'function' ? next(cells[at].value) : next; }]; },
    useMemo(make: () => unknown, deps: readonly unknown[]) { const at = cursor++, old = cells[at]; if (!old || old.deps.some((value: unknown, i: number) => value !== deps[i])) cells[at] = { value: make(), deps }; return cells[at].value; },
    useLayoutEffect(run: () => unknown) { layouts.push(run); },
    useEffect(run: () => unknown) { layouts.push(run); },
  };
  const host = globalThis as typeof globalThis & { __listInteraction?: typeof fixture };
  host.__listInteraction = fixture;
  const globals = new Map(['ResizeObserver', 'MutationObserver', 'HTMLInputElement', 'HTMLDetailsElement', 'getComputedStyle', 'requestAnimationFrame', 'cancelAnimationFrame'].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  const mutations: ((records: any[]) => void)[] = [];
  class Details { open = false; row: any; closest() { return this.row; } }
  class Input {}
  Object.assign(globalThis, { ResizeObserver: class { observe() {} unobserve() {} disconnect() {} },
    MutationObserver: class { constructor(callback: (records: any[]) => void) { mutations.push(callback); } observe() {} disconnect() {} },
    HTMLInputElement: Input, HTMLDetailsElement: Details, getComputedStyle: () => ({ rowGap: 'normal' }), requestAnimationFrame: () => 1, cancelAnimationFrame() {} });
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'list-interaction-hooks', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (['/components/VirtualList.tsx', '/components/Select.tsx'].some(suffix => path.endsWith(suffix)))
          return source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__listInteraction;');
        if (path.endsWith('/components/Popover.tsx')) return 'export const usePopover = () => ({ open: true, trigger: { current: null }, close() {} }); export const Popover = () => null;';
      } }] });
  const draw = (render: () => any) => { cursor = 0; layouts.length = 0; return render(); };
  const reset = () => { cells.length = 0; };
  const scroller = { clientHeight: 300, clientWidth: 300, offsetHeight: 300, scrollTop: 0, getBoundingClientRect: () => ({ top: 0, height: 300 }), addEventListener() {} };
  try {
    const { Select } = await vite.ssrLoadModule('/ui/src/components/Select.tsx');
    const options: SelectOption[] = fixture.rows.map(row => ({ value: row.key, label: `Country ${row.key}` }));
    const menu = draw(() => Select({ label: 'Countries', value: '0', options, onChange() {} })).props.children[1].props.children;
    reset();
    let select = draw(() => menu.type(menu.props));
    select.props.onScroll({ currentTarget: { scrollTop: 4990 * 32 } });
    select = draw(() => menu.type(menu.props));
    const optionIndices = select.props.children.props.children.map((node: any) => node.props['data-option-index']);
    assert.ok(optionIndices.includes(0), 'scrolling cannot unmount the focused option');
    assert.ok(optionIndices.includes(4999)); assert.ok(optionIndices.length < 40);
    select.props.ref.current = scroller;
    const focusedOption = () => select.props.children.props.children.find((node: any) => node.props.tabIndex === 0)?.props['data-option-index'];
    let keyPrevented = 0;
    const selectKey = (key: string, shiftKey = false) => {
      select.props.onKeyDown({ key, shiftKey, preventDefault() { keyPrevented++; } });
      select = draw(() => menu.type(menu.props));
    };
    options[1] = { ...options[1], disabled: true };
    selectKey('ArrowDown');
    assert.equal(keyPrevented, 1, 'ArrowDown moves to the next enabled option');
    assert.equal(focusedOption(), 2, 'disabled choices are skipped');
    selectKey('ArrowUp');
    assert.equal(focusedOption(), 0, 'ArrowUp moves to the previous enabled option');
    selectKey('End');
    assert.equal(focusedOption(), 4999, 'End reaches beyond the mounted window');
    keyPrevented = 0;
    selectKey('Tab');
    selectKey('Tab', true);
    assert.equal(keyPrevented, 0, 'Tab and Shift+Tab leave the menu instead of moving between options');
    assert.equal(focusedOption(), 4999);

    reset();
    const { VirtualList } = await vite.ssrLoadModule('/ui/src/components/VirtualList.tsx');
    const rowItems = [{ key: 'first' }], identity = {};
    const render = (owner = identity) => draw(() => VirtualList({ items: rowItems, itemKey: (item: any) => item.key, estimate: 56,
      label: 'Research', scrollRef: { current: scroller }, identity: owner, children: () => null }));
    let virtual = render();
    const handlers = new Map<string, (event: any) => void>();
    virtual.props.ref.current = { className: '', style: {}, getBoundingClientRect: () => ({ top: 0 }), addEventListener: (name: string, handler: any) => handlers.set(name, handler),
      ownerDocument: { addEventListener() {} } };
    const row = (count = 2, open = false) => {
      const details = Array.from({ length: count }, () => new Details());
      const node = { dataset: { virtualIndex: '0', virtualKey: 'first' }, querySelector: () => details[0], querySelectorAll: () => details };
      details.forEach(detail => { detail.row = node; detail.open = open; });
      virtual.props.children[0].props.ref(node); return { node, details };
    };
    const initial = row(); layouts.forEach(run => run());
    virtual = render(); layouts.forEach(run => run());
    for (const detail of initial.details) { detail.open = true; handlers.get('toggle')!({ target: detail }); }
    virtual.props.children[0].props.ref(null); virtual = render();
    const remounted = row(1);
    assert.deepEqual(remounted.details.map(detail => detail.open), [true], 'the outer expansion survives offscreen remount');
    const delayed = new Details(); delayed.row = remounted.node; remounted.details.push(delayed);
    mutations.forEach(mutation => mutation([{ target: remounted.node }]));
    assert.equal(delayed.open, true, 'conditionally mounted description details restore after the row reopens');
    remounted.details.pop();
    const replacement = new Details(); replacement.row = remounted.node; remounted.details.push(replacement);
    mutations.forEach(mutation => mutation([{ target: remounted.node }]));
    assert.equal(replacement.open, false, 'a description destroyed inside a mounted row starts fresh when recreated');
    remounted.details.pop();
    const authored = new Details(); authored.open = true; authored.row = remounted.node; remounted.details.push(authored);
    mutations.forEach(mutation => mutation([{ target: remounted.node }]));
    assert.equal(authored.open, true, 'newly created details keep their native initial expansion');
    remounted.details.length = 0;
    mutations.forEach(mutation => mutation([{ target: remounted.node }]));
    virtual.props.children[0].props.ref(null); virtual = render();
    assert.equal(row(1, true).details[0].open, true, 'destroying every details node drops its retained state before a later whole-row remount');
    virtual = render({});
    assert.deepEqual(row().details.map(detail => detail.open), [false, false], 'a different data owner cannot inherit expanded rows');
  } finally {
    delete host.__listInteraction; await vite.close();
    for (const [key, descriptor] of globals) { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key); }
  }
});
