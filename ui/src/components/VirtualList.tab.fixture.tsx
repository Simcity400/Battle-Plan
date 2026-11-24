import { memo, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { flushSync } from 'react-dom';
import { VirtualList } from './VirtualList';

export type TabShape = 'static' | 'all' | 'start' | 'middle' | 'end' | 'alternating' | 'runs';
export type TabControls = 'summary' | 'nested' | 'excluded';
export type TabViewport = 'start' | 'middle' | 'end';
export type TabMutation = 'reverse' | 'remove-before' | 'remove-after' | 'insert-before' | 'insert-after' | 'toggle-focus' | 'toggle-before' | 'toggle-after' | 'predicate-focus'
  | 'remove-focus' | 'remove-control' | 'remove-neighbours' | 'replace-focus' | 'clear';
type Description = 'conditional' | 'unconditional';
export type TabCase = { count: number; shape: TabShape; controls: TabControls; viewport: TabViewport; wrapped?: boolean; description?: Description };
type Item = { id: string; interactive: boolean; removedControl?: string; replaced?: boolean; wrapped?: boolean };
type Side = 'reference' | 'virtual';
const height = 96;
const key = (item: Item) => item.id;
const Row = memo(function Row({ item, controls, description }: { item: Item; controls: TabControls; description?: Description }) {
  const [open, setOpen] = useState(false);
  if (!item.interactive) return <div style={{ height }}>
    {item.removedControl !== 'static' && <span tabIndex={-1} data-control={`${item.id}:static`}>Static {item.id}</span>}
  </div>;
  if (item.replaced) return <div style={{ height }}><button data-control={`${item.id}:replacement`}>Replacement</button></div>;
  if (description) return <details onToggle={event => setOpen(event.currentTarget.open)}>
    <summary style={{ height }} data-control={`${item.id}:summary`}>Completion {item.id}</summary>
    {(description === 'unconditional' || open) && <details>
      <summary data-control={`${item.id}:description`}>Description</summary>
      <div style={{ height: 60 }} data-control={`${item.id}:text`}>Technology description {item.id}</div>
    </details>}
  </details>;
  const input = item.removedControl !== 'input' && <input aria-label={`Input ${item.id}`} data-control={`${item.id}:input`} />;
  return <details open style={{ minHeight: height }}>
    {item.removedControl !== 'summary' && <summary data-control={`${item.id}:summary`}>Row {item.id}</summary>}
    {controls !== 'summary' && <>
      {item.wrapped ? <div>{input}</div> : input}
      {item.removedControl !== 'button' && <button data-control={`${item.id}:button`} tabIndex={controls === 'excluded' ? -1 : undefined}>Action</button>}
      {controls === 'excluded' && <>
        <button disabled tabIndex={0} data-control={`${item.id}:disabled`}>Disabled</button>
        <fieldset disabled style={{ display: 'inline', padding: 0, border: 0, margin: 0 }}><input tabIndex={0} data-control={`${item.id}:fieldset`} /></fieldset>
        <input type="hidden" tabIndex={0} data-control={`${item.id}:hidden-input`} />
        <button hidden data-control={`${item.id}:hidden`}>Hidden</button>
        <button style={{ display: 'none' }} data-control={`${item.id}:display`}>No display</button>
        <button style={{ visibility: 'hidden' }} data-control={`${item.id}:visibility`}>Invisible</button>
        <div inert style={{ display: 'inline' }}><button data-control={`${item.id}:inert`}>Inert</button></div>
        <details style={{ display: 'inline' }}><summary tabIndex={-1}>Closed</summary><button data-control={`${item.id}:closed`}>Closed child</button></details>
      </>}
    </>}
  </details>;
});
const ReferenceRow = memo(function ReferenceRow({ item, controls, disabled, description }: { item: Item; controls: TabControls; disabled: boolean; description?: Description }) {
  return <li data-key={item.id}><Row item={disabled ? { ...item, interactive: false } : item} controls={controls} description={description} /></li>;
});
function Lists({ items, controls, disabledKey, description }: { items: readonly Item[]; controls: TabControls; disabledKey?: string; description?: Description }) {
  const reference = useRef<HTMLDivElement>(null), virtual = useRef<HTMLDivElement>(null);
  const rows = useMemo(() => items, [items]);
  const interactive = useMemo(() => (item: Item) => item.interactive && item.id !== disabledKey, [disabledKey]);
  const row = (item: Item) => <Row item={interactive(item) === item.interactive ? item : { ...item, interactive: interactive(item) }} controls={controls} description={description} />;
  return <>{(['reference', 'virtual'] as const).map(side => <section key={side} data-side={side}>
    <button data-control="before">Before</button>
    <span data-control="pointer-before">Outside list</span>
    <div data-scroll data-control="scroller" ref={side === 'reference' ? reference : virtual}
      style={{ height: 180, overflow: 'auto', width: 700 }}>
      {side === 'reference' ? <ol style={{ padding: 0, margin: 0, listStyle: 'none' }}>
        {rows.map(item => <ReferenceRow key={item.id} item={item} controls={controls} disabled={item.id === disabledKey} description={description} />)}
      </ol> : <VirtualList items={rows} itemKey={key} estimate={height} isInteractive={interactive} scrollRef={virtual}>
        {row}
      </VirtualList>}
    </div><button data-control="after">After</button><span data-control="pointer-after">Outside list</span>
  </section>)}</>;
}

export function createTabFixture(render: (node: ReactNode) => void) {
  let items: Item[] = [], controls: TabControls = 'summary', generation = 0;
  let disabledKey: string | undefined;
  let description: Description | undefined;
  let initial: Item[] = [], initialViewport: TabViewport = 'start';
  let gated: Map<number, FrameRequestCallback> | undefined;
  const nativeFrame = window.requestAnimationFrame.bind(window), nativeCancel = window.cancelAnimationFrame.bind(window);
  let deferredKey = false;
  let nativeKey = false;
  document.addEventListener('keydown', event => {
    nativeKey = event.key === 'Tab';
    const active = document.activeElement, control = active?.getAttribute('data-control');
    deferredKey = event.defaultPrevented || (active?.closest('[data-side]')?.getAttribute('data-side') === 'virtual'
      && (control === 'before' || control === 'after'));
  });
  document.addEventListener('keyup', () => { nativeKey = false; });
  document.addEventListener('focusin', event => {
    if (nativeKey && !event.relatedTarget
      && (event.target as HTMLElement).closest('[data-side]')?.getAttribute('data-side') === 'virtual') deferredKey = true;
  });
  const frame = () => new Promise<void>(done => requestAnimationFrame(() => done()));
  const settle = async () => { await frame(); await frame(); };
  const section = (side: Side) => document.querySelector<HTMLElement>(`[data-side="${side}"]`)!;
  const draw = () => render(<Lists key={generation} items={items} controls={controls} disabledKey={disabledKey} description={description} />);
  const mutate = (kind: TabMutation, id: string, control = 'summary') => {
    const at = items.findIndex(item => item.id === id);
    if (kind === 'reverse') items = [...items].reverse();
    else if (kind === 'remove-before') items = items.filter((_, index) => index !== at - 1);
    else if (kind === 'remove-after') items = items.filter((_, index) => index !== at + 1);
    else if (kind === 'insert-before' || kind === 'insert-after') {
      const position = at + (kind === 'insert-after' ? 1 : 0);
      items = [...items.slice(0, position), { id: 'inserted', interactive: true }, ...items.slice(position)];
    }
    else if (kind === 'remove-focus') items = items.filter(item => item.id !== id);
    else if (kind === 'remove-neighbours') items = items.filter((_, index) => index !== at - 1 && index !== at + 1);
    else if (kind === 'remove-control') items = items.map(item => item.id === id ? { ...item, removedControl: control } : item);
    else if (kind === 'replace-focus') items = items.map(item => item.id === id ? { id, interactive: !item.interactive, replaced: true } : item);
    else if (kind === 'clear') items = [];
    else if (kind === 'predicate-focus') disabledKey = id;
    else {
      const target = at + (kind === 'toggle-before' ? -1 : kind === 'toggle-after' ? 1 : 0);
      items = items.map((item, index) => index === target ? { ...item, interactive: !item.interactive } : item);
    }
    flushSync(draw);
  };
  const viewport = (side: Side, position: TabViewport) => {
    const scroller = section(side).querySelector<HTMLElement>('[data-scroll]')!;
    scroller.scrollTop = position === 'start' ? 0 : position === 'middle' ? scroller.scrollHeight / 2 : scroller.scrollHeight;
  };
  return {
    async mount(spec: TabCase) {
      controls = spec.controls; description = spec.description; disabledKey = undefined; generation++;
      items = Array.from({ length: spec.count }, (_, at) => ({ id: String(at), wrapped: spec.wrapped, interactive:
        spec.shape === 'all' || (spec.shape === 'start' && at === 0) || (spec.shape === 'middle' && at === Math.floor(spec.count / 2))
        || (spec.shape === 'end' && at === spec.count - 1) || (spec.shape === 'alternating' && at % 2 === 0)
        || (spec.shape === 'runs' && [0, Math.floor(spec.count / 2), spec.count - 1].includes(at)) }));
      initial = items; initialViewport = spec.viewport;
      draw(); await settle();
      for (const side of ['reference', 'virtual'] as const) viewport(side, spec.viewport);
      await settle();
    },
    async reset() {
      for (const side of ['reference', 'virtual'] as const) section(side).inert = false;
      section('reference').querySelector<HTMLElement>('[data-control="before"]')!.focus({ preventScroll: true });
      items = initial; disabledKey = undefined; flushSync(draw);
      for (const side of ['reference', 'virtual'] as const) viewport(side, initialViewport);
      await settle();
    },
    async activate(side: Side, control: string) {
      for (const other of ['reference', 'virtual'] as const) section(other).inert = other !== side;
      section(side).querySelector<HTMLElement>(`[data-control="${control}"]`)!.focus({ preventScroll: true });
    },
    async anchor(side: Side, id: string, control = 'summary') {
      const at = items.findIndex(item => item.id === id);
      section(side).querySelector<HTMLElement>('[data-scroll]')!.scrollTop = at * height;
      const find = () => section(side).querySelector<HTMLElement>(`[data-control="${id}:${control}"]`)
        ?? section(side).querySelector<HTMLElement>(`[data-control="${id}:static"]`);
      let target = find();
      if (side === 'virtual') {
        await settle();
        for (let tries = 0; tries < 30 && !(target = find()); tries++) await frame();
      }
      target!.focus();
      if (side === 'virtual') await frame();
    },
    async scrollAway(side: Side) {
      section(side).querySelector<HTMLElement>('[data-scroll]')!.scrollTop = 0;
      if (side === 'virtual') await settle();
    },
    async viewport(side: Side, position: TabViewport) {
      const before = section(side).querySelector<HTMLElement>('[data-scroll]')!.scrollTop;
      viewport(side, position);
      if (side === 'virtual' && before !== section(side).querySelector<HTMLElement>('[data-scroll]')!.scrollTop) await settle();
    },
    async mutate(kind: TabMutation, id: string, control = 'summary') {
      mutate(kind, id, control);
      if (document.activeElement?.closest('[data-side]')?.getAttribute('data-side') === 'virtual'
        || document.activeElement === document.body) await settle();
    },
    mutateImmediately: mutate,
    gateFrames() {
      gated = new Map(); let next = 0;
      window.requestAnimationFrame = callback => { const id = --next; gated!.set(id, callback); return id; };
      window.cancelAnimationFrame = id => { if (id < 0) gated!.delete(id); else nativeCancel(id); };
    },
    async releaseFrames() {
      window.requestAnimationFrame = nativeFrame; window.cancelAnimationFrame = nativeCancel;
      for (const callback of gated?.values() ?? []) nativeFrame(callback);
      gated = undefined; await settle();
    },
    async programmatic(side: Side, control: string) {
      section(side).querySelector<HTMLElement>(`[data-control="${control}"]`)!.focus();
      if (side === 'virtual') await frame();
    },
    external(side: Side, interaction: 'focus' | 'pointer', control: string) {
      const target = section(side).querySelector<HTMLElement>(`[data-control="${interaction === 'pointer' ? 'pointer-' : ''}${control}"]`)!;
      if (interaction === 'focus') target.focus({ preventScroll: true });
      else { const bounds = target.getBoundingClientRect(); return { x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height / 2 }; }
    },
    async toggle(side: Side, control: string) {
      section(side).querySelector<HTMLElement>(`[data-control="${control}"]`)!.click();
      for (let i = 0; i < 8; i++) await frame();
    },
    point(side: Side, control: string) {
      const bounds = section(side).querySelector<HTMLElement>(`[data-control="${control}"]`)!.getBoundingClientRect();
      return { x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height / 2 };
    },
    details(side: Side, id: string) {
      const row = section(side).querySelector<HTMLElement>(`[data-control="${id}:summary"]`)!.closest('li')!;
      return [...row.querySelectorAll('details')].map(details => details.open);
    },
    mounted(side: Side, id: string) { return Boolean(section(side).querySelector(`[data-control="${id}:summary"]`)); },
    async settled() { for (let i = 0; i < 8; i++) await frame(); },
    active() { return document.activeElement?.getAttribute('data-control') ?? document.activeElement?.tagName; },
    rows() { return section('virtual').querySelectorAll('li').length; },
    ids() { return items.filter(item => item.interactive).map(item => item.id); },
    async keySettled() { if (deferredKey) await frame(); },
    settle,
  };
}
