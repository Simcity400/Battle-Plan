import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { createElement } from 'react';
import { transformSync } from 'rolldown/utils';

test('VirtualList consumes authored CSS row spacing for collapsed, expanded and changed styles', async () => {
  let cursor = 0, gap = 1, zoom = 1, viewportHeight = 400, padding = 0;
  const cells: { value?: any; deps?: readonly unknown[]; cleanup?: () => unknown }[] = [], effects: (() => unknown)[] = [], frames: (() => unknown)[] = [];
  const effect = (run: () => unknown, deps?: readonly unknown[]) => {
    const cell = cells[cursor++] ??= {};
    if (deps && cell.deps && deps.length === cell.deps.length && deps.every((value, index) => Object.is(value, cell.deps![index]))) return;
    cell.deps = deps;
    effects.push(() => { cell.cleanup?.(); const cleanup = run(); cell.cleanup = typeof cleanup === 'function' ? cleanup as () => unknown : undefined; });
  };
  const fixture = {
    createElement,
    useRef(value: unknown) { const cell = cells[cursor++] ??= {}; return cell.value ??= { current: value }; },
    useState(value: unknown) {
      const cell = cells[cursor++] ??= {};
      if (!('value' in cell)) cell.value = typeof value === 'function' ? (value as () => unknown)() : value;
      return [cell.value, (next: unknown) => { cell.value = typeof next === 'function' ? (next as (old: unknown) => unknown)(cell.value) : next; }];
    },
    useMemo(make: () => unknown, deps: readonly unknown[]) {
      const cell = cells[cursor++] ??= {};
      if (!cell.deps || cell.deps.length !== deps.length || deps.some((value, index) => !Object.is(value, cell.deps![index]))) {
        cell.deps = deps; cell.value = make();
      }
      return cell.value;
    },
    useLayoutEffect: effect,
    useEffect: effect,
  };
  const host = globalThis as typeof globalThis & { __virtualSpacing?: typeof fixture };
  host.__virtualSpacing = fixture;
  const globals = new Map(['ResizeObserver', 'MutationObserver', 'getComputedStyle', 'requestAnimationFrame', 'cancelAnimationFrame', 'HTMLInputElement']
    .map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  const sizes: { changed(entries: any[]): void; observed: Set<unknown> }[] = [];
  Object.assign(globalThis, {
    getComputedStyle: () => ({ rowGap: `${gap}px`, paddingTop: `${padding}px`, paddingRight: padding ? '2px' : '0px',
      paddingBottom: padding ? '4px' : '0px', paddingLeft: padding ? '2px' : '0px' }),
    requestAnimationFrame: (run: () => unknown) => { frames.push(run); return frames.length; }, cancelAnimationFrame() {}, HTMLInputElement: class {},
    ResizeObserver: class {
      observed = new Set<unknown>();
      changed: (entries: any[]) => void;
      constructor(changed: (entries: any[]) => void) { this.changed = changed; sizes.push(this); }
      observe(node: unknown) { this.observed.add(node); } unobserve(node: unknown) { this.observed.delete(node); } disconnect() {}
    },
    MutationObserver: class { constructor(_run: () => unknown) {} observe() {} disconnect() {} },
  });
  try {
    const input = await readFile(new URL('./VirtualList.tsx', import.meta.url), 'utf8');
    const source = input.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__virtualSpacing;')
      .replace("'react-dom'", JSON.stringify(import.meta.resolve('react-dom')))
      .replace("'./virtualListIndex'", JSON.stringify(new URL('./virtualListIndex.ts', import.meta.url).href));
    const compiled = transformSync('VirtualList.tsx', source, { lang: 'tsx', jsx: { runtime: 'classic', pragma: 'globalThis.__virtualSpacing.createElement' } }).code;
    const { VirtualList } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`);
    const scroller = { clientHeight: 400, clientWidth: 400, offsetHeight: 400, scrollTop: 0,
      getBoundingClientRect: () => ({ top: 0, height: viewportHeight * zoom }), addEventListener() {}, removeEventListener() {} };
    const root = { className: '', style: {}, getBoundingClientRect: () => ({ top: -scroller.scrollTop * zoom }), addEventListener() {}, removeEventListener() {}, ownerDocument: { addEventListener() {}, removeEventListener() {} } };
    const items = [0, 1, 2, 3, 4], rows = items.map(at => ({ dataset: { virtualIndex: String(at), virtualKey: String(at) }, offsetHeight: 52, querySelectorAll: () => [] }));
    const props = { items, itemKey: String, estimate: 52, retainExpansion: false, flowWhenFits: false,
      scrollRef: { current: scroller }, children: () => null };
    const render = () => {
      cursor = 0; effects.length = 0;
      const element = VirtualList(props);
      element.props.ref.current = root;
      for (const row of element.props.children) row.props.ref(rows[row.props['data-virtual-index']]);
      for (const run of effects) run();
      return element;
    };
    render(); render();
    let list = render();
    assert.equal(list.props.children[1].props.style.top, 53, 'CSS spacing follows each measured row');
    assert.equal(list.props.style.height, 5 * 52 + 4, 'the last row has no trailing gap');
    const rowObserver = sizes.find(size => size.observed.has(rows[0]))!;
    rows[0].offsetHeight = 172; rowObserver.changed([{ target: rows[0] }]); list = render();
    assert.equal(list.props.children[1].props.style.top, 173, 'the next row follows the complete expanded body and its CSS gap');
    assert.equal(list.props.style.height, 5 * 52 + 4 + 120);
    list.props.children[0].props.onKeyDown({ key: 'End', defaultPrevented: false, target: {}, preventDefault() {} });
    assert.equal(scroller.scrollTop, 4 * 53 + 120, 'keyboard reveal uses the same spaced prefixes');
    gap = 2.5;
    sizes.find(size => size.observed.has(scroller))!.changed([]);
    for (const run of frames.splice(0)) run();
    list = render();
    assert.equal(list.props.children[1].props.style.top, 174.5, 'a changed authored gap updates retained measurements');
    assert.equal(list.props.style.height, 5 * 52 + 4 * 2.5 + 120);
    const fractional = 128.984375;
    rows[0].offsetHeight = Math.round(fractional);
    rowObserver.changed([{ target: rows[0], borderBoxSize: [{ blockSize: fractional, inlineSize: 400 }] }]);
    list = render();
    assert.equal(list.props.children.find((row: any) => row.props['data-virtual-index'] === 1).props.style.top, fractional + 2.5, 'fractional expanded layout pixels survive integer offsetHeight rounding');
    assert.equal(list.props.style.height, 4 * 52 + 4 * 2.5 + fractional);
    zoom = 1.25; viewportHeight = 399.984375; scroller.scrollTop = 7.125;
    sizes.find(size => size.observed.has(scroller))!.changed([{ target: scroller, borderBoxSize: [{ blockSize: viewportHeight, inlineSize: 400 }] }]);
    for (const run of frames.splice(0)) run();
    list = render();
    list.props.children[0].props.onKeyDown({ key: 'End', defaultPrevented: false, target: {}, preventDefault() {} });
    assert.equal(scroller.scrollTop, fractional + 3 * 52 + 4 * 2.5, 'zoomed keyboard seeking uses the unrounded scroller box as its coordinate scale');
    const resized = () => sizes.find(size => size.observed.has(scroller))!.changed([{ target: scroller, borderBoxSize: [{ blockSize: viewportHeight, inlineSize: 400 }] }]);
    zoom = 1; padding = 3; props.flowWhenFits = true;
    viewportHeight = scroller.clientHeight = scroller.offsetHeight = 100; scroller.scrollTop = 0;
    resized(); render(); list = render();
    assert.equal(list.props.style.height, 4 * 52 + 4 * 2.5 + fractional + 7, 'authored padding contributes once to the full extent');
    assert.deepEqual(list.props.children[0].props.style, { position: 'absolute', top: 3, left: 2, right: 2 });
    list.props.children[0].props.onKeyDown({ key: 'End', defaultPrevented: false, target: {}, preventDefault() {} });
    assert.equal(scroller.scrollTop, fractional + 3 * 52 + 4 * 2.5 + 3, 'keyboard reveal includes the leading padding');
    const keysBefore = list.props.children.map((row: any) => row.key);
    viewportHeight = scroller.clientHeight = scroller.offsetHeight = 400; scroller.scrollTop = 0;
    resized(); render(); list = render();
    assert.equal(list.props.style.height, undefined, 'fitting rows retain native grid stretching');
    assert.equal(list.props.style.position, undefined);
    assert.ok(list.props.children.every((row: any) => row.props.style === undefined));
    assert.deepEqual(list.props.children.map((row: any) => row.key), keysBefore, 'layout changes retain keyed row controls');
    viewportHeight = scroller.clientHeight = scroller.offsetHeight = 100;
    resized(); render(); list = render();
    assert.equal(list.props.style.position, 'relative', 'overflow restores windowing without changing owners');
    assert.deepEqual(list.props.children.map((row: any) => row.key), keysBefore);
  } finally {
    delete host.__virtualSpacing;
    for (const [key, descriptor] of globals) { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key); }
  }
});
