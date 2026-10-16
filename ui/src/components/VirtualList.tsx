import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties, HTMLAttributes, ReactNode, RefObject } from 'react';
import { flushSync } from 'react-dom';
import { VirtualListIndex } from './virtualListIndex';

/** Native sequential stops, rather than every programmatically focusable node. */
function focusables(node: HTMLElement) {
  return [...node.querySelectorAll<HTMLElement>('*')].filter(target => {
    if (target.tabIndex < 0 || target.matches(':disabled, input[type="hidden"]') || target.closest('[inert]')) return false;
    if (target.matches('summary') && !target.hasAttribute('tabindex')
      && (target.parentElement?.tagName !== 'DETAILS' || target.parentElement.querySelector(':scope > summary') !== target)) return false;
    for (let parent = target.parentElement; parent; parent = parent.parentElement) {
      if (parent instanceof HTMLDetailsElement && !parent.open && !parent.querySelector(':scope > summary')?.contains(target)) return false;
    }
    if (!target.getClientRects().length) return false;
    const visibility = getComputedStyle(target).visibility;
    return visibility !== 'hidden' && visibility !== 'collapse';
  }).sort((a, b) => (a.tabIndex || Infinity) - (b.tabIndex || Infinity));
}
/** Variable-height ordered rows, sharing an existing scroll surface when supplied. */
export function VirtualList<T>({ items, itemKey, estimate, label, className, style, flowWhenFits = false, scrollRef, rowProps, isInteractive, children, offset = 0, identity, retainExpansion = true }: {
  items: readonly T[]; itemKey: (item: T) => string; estimate: number; label?: string; className?: string;
  /** Authored root presentation; padding is measured with the row geometry. */
  style?: CSSProperties;
  /** Keep the native flow when the complete list fits the measured viewport. */
  flowWhenFits?: boolean;
  /** Measurements and expansion belong to this data owner. */
  identity?: unknown;
  /** False when the consumer already controls every details element. */
  retainExpansion?: boolean;
  scrollRef: RefObject<HTMLElement | null>; offset?: number;
  rowProps?: (item: T, index: number) => HTMLAttributes<HTMLLIElement>;
  /** Static rows keep their list positions but are not keyboard destinations.
   * Replace items or this predicate when eligibility changes. */
  isInteractive?: (item: T, index: number) => boolean;
  children: (item: T, index: number) => ReactNode;
}) {
  const root = useRef<HTMLOListElement>(null), nodes = useRef(new Map<string, HTMLLIElement>());
  const [scroller, setScroller] = useState<HTMLElement | null>(null);
  // Ancestor refs attach after child layout effects. Read the actual element
  // after each commit so the first mount and subsequent ref changes both bind.
  useEffect(() => { setScroller(scrollRef.current); });
  const [view, setView] = useState({ top: 0, height: 400 });
  const [padding, setPadding] = useState({ top: 0, right: 0, bottom: 0, left: 0 });
  const rowOffset = offset + padding.top;
  const [, update] = useState(0), [focus, setFocus] = useState<string | null>(null);
  // Native DOM owns descendant starting points; keys retain a removed row's
  // preceding logical neighbour even when that neighbour was never mounted.
  const sequential = useRef<{ key: string; target: HTMLElement; keys: readonly string[]; gap: number; pointer?: boolean } | null>(null);
  const [resume, setResume] = useState<string | null>(null);
  const resuming = useRef(false);
  const keys = useMemo(() => items.map(itemKey), [items]);
  const positions = useMemo(() => new Map(keys.map((key, at) => [key, at])), [keys]);
  useLayoutEffect(() => {
    const cursor = sequential.current;
    if (!cursor || cursor.keys === keys) return;
    if (!positions.has(cursor.key)) {
      const oldAt = cursor.keys.indexOf(cursor.key);
      const boundary = oldAt < 0 ? cursor.gap : oldAt;
      // Removal keeps the preceding surviving node as the starting point.
      // Insertions after it stay on the forward side, including later commits.
      cursor.gap = 0;
      for (let at = boundary - 1; at >= 0; at--) {
        const previous = positions.get(cursor.keys[at]);
        if (previous !== undefined) { cursor.gap = previous + 1; break; }
      }
    }
    cursor.keys = keys;
  }, [keys, positions]);
  // Evaluate content eligibility once per revision, never per keyboard move.
  const eligible = useMemo(() => {
    if (!isInteractive) return null;
    const positions: number[] = [];
    items.forEach((item, at) => { if (isInteractive(item, at)) positions.push(at); });
    return positions;
  }, [items, isInteractive]);
  const destination = (at: number, direction: 1 | -1) => {
    if (!eligible) return at >= 0 && at < items.length ? at : null;
    let low = 0, high = eligible.length;
    while (low < high) {
      const middle = Math.floor((low + high) / 2);
      if (eligible[middle] < at) low = middle + 1; else high = middle;
    }
    return eligible[direction === 1 || eligible[low] === at ? low : low - 1] ?? null;
  };
  const measurements = useMemo(() => new Map<string, number>(), [identity]);
  const opened = useMemo(() => new Map<string, Set<number>>(), [identity]);
  const restoring = useMemo(() => new WeakMap<HTMLElement, Set<HTMLDetailsElement>>(), [identity]);
  const mountedDetails = useMemo(() => new WeakMap<HTMLElement, readonly HTMLDetailsElement[]>(), [identity]);
  const restoreDetails = (node: HTMLElement) => {
    if (!retainExpansion) return;
    const key = node.dataset.virtualKey ?? '', details = [...node.querySelectorAll('details')];
    const previous = mountedDetails.get(node);
    let expanded = opened.get(key);
    const mounted = previous && new Set(previous);
    const unseen = previous && expanded && new Set([...expanded].filter(at => !previous[at]));
    if (expanded && previous && (previous.length !== details.length || previous.some((node, at) => node !== details[at]))) {
      // A mounted row can destroy or replace conditional descendants. Keep
      // only surviving nodes' expansion, remapping their positions if needed.
      // Unseen entries still belong to whole-row restoration awaiting content.
      const positions = new Map(details.map((details, at) => [details, at]));
      const pending = restoring.get(node);
      for (const details of previous) if (!positions.has(details)) pending?.delete(details);
      if (!pending?.size) restoring.delete(node);
      expanded = new Set([...expanded].flatMap(at => {
        if (!previous[at]) return [at];
        const current = positions.get(previous[at]);
        return current === undefined ? [] : [current];
      }));
      if (!details.length && !expanded.size) { opened.delete(key); expanded = undefined; }
      else opened.set(key, expanded);
    }
    mountedDetails.set(node, details);
    if (expanded) details.forEach((details, i) => {
      if (mounted && (mounted.has(details) || !unseen?.has(i))) {
        // Native state owns surviving and newly created details. Only saved
        // expansions awaiting a whole-row remount's content are restored here.
        if (details.open) expanded.add(i); else expanded.delete(i);
        return;
      }
      const open = expanded.has(i);
      if (details.open === open) return;
      const pending = restoring.get(node) ?? new Set<HTMLDetailsElement>();
      pending.add(details); restoring.set(node, pending); details.open = open;
    });
  };
  const index = useMemo(() => {
    const next = new VirtualListIndex(items.length, estimate);
    keys.forEach((key, i) => { const height = measurements.get(key); if (height) next.set(i, height); });
    return next;
  }, [keys, estimate, items.length, measurements]);
  const observer = useRef<ResizeObserver | null>(null);
  const tabDirection = useRef<boolean | null>(null);
  const pendingFocus = useRef<{ key: string; backwards: boolean } | null>(null);
  const internalFocus = useRef(false);
  const focusFrame = useRef(0);
  useLayoutEffect(() => {
    const list = root.current;
    if (!list || !scroller) return;
    let frame = 0, width = scroller.clientWidth, scrollerHeight = scroller.offsetHeight;
    const measure = () => {
      frame = 0;
      const rect = scroller.getBoundingClientRect(), bounds = list.getBoundingClientRect();
      const scale = rect.height / (scrollerHeight || rect.height || 1);
      const top = (rect.top - bounds.top) / scale;
      // Absolute rows retain the authored spacing of the list's flow layout.
      // Store it separately from row heights so expansion never adds a tail gap.
      const css = getComputedStyle(list);
      const gap = parseFloat(css.rowGap) || 0;
      const nextPadding = { top: parseFloat(css.paddingTop) || 0, right: parseFloat(css.paddingRight) || 0,
        bottom: parseFloat(css.paddingBottom) || 0, left: parseFloat(css.paddingLeft) || 0 };
      setPadding(previous => previous.top === nextPadding.top && previous.right === nextPadding.right
        && previous.bottom === nextPadding.bottom && previous.left === nextPadding.left ? previous : nextPadding);
      if (index.setGap(gap)) update(value => value + 1);
      if (width !== scroller.clientWidth) { width = scroller.clientWidth; measurements.clear(); keys.forEach((_, i) => index.set(i, estimate)); update(value => value + 1); }
      const height = scroller.clientHeight;
      setView(previous => previous.top === top && previous.height === height ? previous : { top, height });
    };
    const schedule = () => { frame ||= requestAnimationFrame(measure); };
    measure();
    const toggle = (event: Event) => {
      if (!retainExpansion || !(event.target instanceof HTMLDetailsElement)) return;
      const row = event.target.closest<HTMLElement>('[data-virtual-key]'), key = row?.dataset.virtualKey;
      if (key === undefined || !row) return;
      const pending = restoring.get(row);
      pending?.delete(event.target);
      if (!pending?.size) restoring.delete(row);
      const at = [...row.querySelectorAll('details')].indexOf(event.target);
      const expanded = opened.get(key) ?? new Set<number>();
      if (event.target.open) expanded.add(at); else expanded.delete(at);
      opened.set(key, expanded);
      if (pendingFocus.current) queueFocus(row);
    };
    list.addEventListener('toggle', toggle, true);
    // A row may build nested details only after restoring its outer expansion.
    const contents = retainExpansion && typeof MutationObserver !== 'undefined' ? new MutationObserver(records => {
      for (const record of records) {
        const target = record.target as HTMLElement;
        const row = target.dataset?.virtualKey !== undefined ? target : target.closest?.<HTMLElement>('[data-virtual-key]');
        if (row && nodes.current.get(row.dataset.virtualKey ?? '') === row) { restoreDetails(row); focusPending(row); }
      }
    }) : null;
    contents?.observe(list, { childList: true, subtree: true });
    scroller.addEventListener('scroll', schedule, { passive: true });
    // Native Tab can enter any mounted row of a neighbouring virtual list.
    // Remember only the current key's default focus move; pointer and explicit
    // focus changes keep the row they selected.
    let tabTimer = 0;
    const keyboard = (event: KeyboardEvent) => {
      clearTimeout(tabTimer);
      pendingFocus.current = null;
      tabDirection.current = event.key === 'Tab' ? event.shiftKey : null;
      resuming.current = false;
      if (tabDirection.current !== null) tabTimer = window.setTimeout(() => { tabDirection.current = null; resuming.current = false; }, 0);
      const cursor = sequential.current;
      if (event.key === 'Tab' && !event.defaultPrevented && cursor && (cursor.pointer || !cursor.target.isConnected)
        && document.activeElement === document.body && !list.closest('[inert]')) {
        const direction = event.shiftKey ? -1 : 1, at = positions.get(cursor.key);
        resuming.current = true;
        if (at !== undefined) {
          // Make both logical neighbours available before the native default
          // action. The browser retains its own starting point inside this row,
          // including details focus scopes and partial descendant removal.
          flushSync(() => setResume(cursor.key));
          return;
        }
        const next = destination(cursor.gap > 0 ? cursor.gap - 1 : event.shiftKey ? -1 : 0, direction);
        if (next !== null) { event.preventDefault(); seek(next, event.shiftKey); }
      }
    };
    const forgetExternal = (event: Event) => {
      if (!list.contains(event.target as Node)) { sequential.current = null; setFocus(null); setResume(null); }
    };
    const pointer = (event: PointerEvent) => {
      tabDirection.current = null; pendingFocus.current = null; resuming.current = false;
      const target = event.target as HTMLElement;
      const row = target.closest?.<HTMLElement>('[data-virtual-key]'), key = row?.dataset.virtualKey;
      if (row && key !== undefined && nodes.current.get(key) === row) {
        // Static content can leave BODY active while establishing the browser's
        // next sequential starting point. Retain its row until native Tab moves.
        sequential.current = { key, target, keys, gap: positions.get(key) ?? 0, pointer: true };
        setFocus(key); setResume(null);
      } else { sequential.current = null; setFocus(null); setResume(null); }
    };
    const document = list.ownerDocument;
    document.addEventListener('keydown', keyboard, true);
    document.addEventListener('pointerdown', pointer, true);
    document.addEventListener('focusin', forgetExternal);
    const size = new ResizeObserver(entries => {
      const box = entries.find(entry => entry.target === scroller)?.borderBoxSize?.[0];
      if (box) scrollerHeight = box.blockSize;
      schedule();
    }); size.observe(scroller); size.observe(list);
    const spacing = typeof MutationObserver !== 'undefined' ? new MutationObserver(schedule) : null;
    spacing?.observe(list, { attributes: true, attributeFilter: ['class', 'style'] });
    observer.current = new ResizeObserver(entries => {
      // Border-box observations retain subpixel layout heights independently
      // of CSS zoom. Rounded DOM heights are only a fallback before observation.
      const changes = entries.map(entry => ({ node: entry.target as HTMLLIElement,
        height: entry.borderBoxSize?.[0]?.blockSize ?? (entry.target as HTMLLIElement).offsetHeight }));
      let changed = false;
      for (const { node, height } of changes) {
        const at = Number(node.dataset.virtualIndex);
        measurements.set(keys[at], height);
        changed = index.set(at, height) || changed;
      }
      if (changed) update(value => value + 1);
    });
    for (const node of nodes.current.values()) observer.current.observe(node);
    // A content revision can happen before a queued handoff's frame. Its key
    // still names the destination even when that row's position has changed.
    const pending = pendingFocus.current;
    if (pending) {
      const node = nodes.current.get(pending.key);
      if (!positions.has(pending.key)) pendingFocus.current = null;
      else if (node) queueFocus(node);
    }
    return () => {
      if (frame) cancelAnimationFrame(frame); clearTimeout(tabTimer);
      scroller.removeEventListener('scroll', schedule);
      document.removeEventListener('keydown', keyboard, true); document.removeEventListener('pointerdown', pointer, true);
      document.removeEventListener('focusin', forgetExternal);
      size.disconnect(); spacing?.disconnect(); contents?.disconnect(); list.removeEventListener('toggle', toggle, true);
      observer.current?.disconnect(); observer.current = null; tabDirection.current = null;
      if (focusFrame.current) cancelAnimationFrame(focusFrame.current);
      focusFrame.current = 0;
    };
  }, [index, keys, eligible, scroller, estimate, retainExpansion, rowOffset]);
  useEffect(() => () => { pendingFocus.current = null; }, []);
  const flow = flowWhenFits && rowOffset + index.offset(items.length) + padding.bottom <= view.height;
  const [start, end] = flow || typeof ResizeObserver === 'undefined' ? [0, items.length] : index.range(view.top - rowOffset, view.height);
  const visible = Array.from({ length: Math.max(0, end - start) }, (_, i) => start + i);
  const focusedAt = focus === null ? undefined : positions.get(focus);
  if (focusedAt !== undefined && !visible.includes(focusedAt)) visible.push(focusedAt);
  const resumeAt = resume === null ? undefined : positions.get(resume);
  if (resumeAt !== undefined) for (const direction of [-1, 1] as const) {
    const neighbour = destination(resumeAt + direction, direction);
    if (neighbour !== null && !visible.includes(neighbour)) visible.push(neighbour);
  }
  // A wholly static viewport still needs a native row control for Tab entry.
  // Use the real first eligible row, leaving fully static lists to the browser.
  const first = destination(0, 1);
  if (first !== null && !visible.some(at => destination(at, 1) === at)) visible.push(first);
  visible.sort((a, b) => a - b);
  const focusPending = (node: HTMLElement) => {
    const pending = pendingFocus.current;
    if (!pending || node.dataset.virtualKey !== pending.key) return;
    // Restoring details can mount their controls on a later toggle task. Wait
    // for those toggles and retained descendants before choosing the final
    // control; toggle and mutation listeners finish the focus handoff.
    const expanded = retainExpansion && opened.get(node.dataset.virtualKey ?? '');
    if (pending.backwards && (restoring.get(node)?.size || (expanded && expanded.has(0)
      && [...expanded].some(at => at >= node.querySelectorAll('details').length)))) return;
    const targets = focusables(node);
    pendingFocus.current = null;
    // Seeking can unmount the origin, making relatedTarget null. DOM focus
    // events are synchronous, so mark this handoff through its focus dispatch.
    internalFocus.current = true;
    try { (pending.backwards ? targets.at(-1) : targets[0])?.focus(); }
    finally { internalFocus.current = false; }
  };
  const queueFocus = (node: HTMLElement) => {
    // Only the destination's restored controls can replace its focus callback.
    if (node.dataset.virtualKey !== pendingFocus.current?.key) return;
    if (focusFrame.current) cancelAnimationFrame(focusFrame.current);
    focusFrame.current = requestAnimationFrame(() => { focusFrame.current = 0; focusPending(node); });
  };
  const seek = (at: number, backwards = false) => {
    const scroller = scrollRef.current; if (!scroller || !items.length) return;
    at = Math.max(0, Math.min(items.length - 1, at));
    scroller.scrollTop += rowOffset + index.offset(at) - view.top;
    const key = keys[at];
    setView({ top: rowOffset + index.offset(at), height: scroller.clientHeight }); setFocus(key);
    pendingFocus.current = { key, backwards };
    if (focusFrame.current) cancelAnimationFrame(focusFrame.current);
    focusFrame.current = requestAnimationFrame(() => { focusFrame.current = 0; const node = nodes.current.get(key); if (node) focusPending(node); });
  };
  return <ol ref={root} className={className} aria-label={label} style={{ padding: 0, margin: 0, listStyle: 'none', ...style,
    position: flow ? undefined : 'relative', height: flow ? undefined : rowOffset + index.offset(items.length) + padding.bottom }}
    onFocus={event => {
      const backwards = tabDirection.current;
      if (resuming.current) {
        // A backward native move can reach a neighbour before its conditional
        // retained details finish mounting. Complete the usual keyed handoff.
        const row = (event.target as HTMLElement).closest<HTMLElement>('[data-virtual-key]');
        if (backwards && row && row.dataset.virtualKey !== resume && resumeAt !== undefined) {
          const previous = destination(resumeAt - 1, -1);
          if (previous !== null) seek(previous, true);
        }
        return;
      }
      if (internalFocus.current || backwards === null || event.currentTarget.contains(event.relatedTarget) || !items.length) return;
      const at = destination(backwards ? items.length - 1 : 0, backwards ? -1 : 1);
      if (at === null) return;
      const targets = nodes.current.get(keys[at]) && focusables(nodes.current.get(keys[at])!);
      if (event.target !== (backwards ? targets?.at(-1) : targets?.[0])) seek(at, backwards);
    }}
    onBlurCapture={event => { if (event.relatedTarget && !event.currentTarget.contains(event.relatedTarget)) {
      sequential.current = null; setFocus(null); setResume(null);
    } }}>
    {visible.map(at => <li key={keys[at]} {...rowProps?.(items[at], at)} data-virtual-index={at} data-virtual-key={keys[at]} aria-setsize={items.length} aria-posinset={at + 1}
      style={flow ? undefined : { position: 'absolute', top: rowOffset + index.offset(at), left: padding.left, right: padding.right }}
      ref={node => {
        const key = keys[at], old = nodes.current.get(key); if (old) observer.current?.unobserve(old);
        if (node) {
          nodes.current.set(key, node); observer.current?.observe(node);
          restoreDetails(node);
        }
        else nodes.current.delete(key);
      }}
      onFocusCapture={event => {
        setFocus(keys[at]); setResume(null);
        sequential.current = { key: keys[at], target: event.target as HTMLElement, keys, gap: at };
      }}
      onKeyDown={event => {
        rowProps?.(items[at], at).onKeyDown?.(event);
        if (event.defaultPrevented) return;
        if (event.key === 'Tab') {
          const targets = focusables(event.currentTarget);
          const current = event.target as HTMLElement, position = targets.indexOf(current);
          // Programmatically focused negative-tabindex controls also have a
          // native starting point. Let the browser visit any stop in this row
          // on that side before handing off to another logical row.
          const nativeInRow = position >= 0 ? event.shiftKey ? position > 0 : position < targets.length - 1
            : targets.some(target => Boolean(current.compareDocumentPosition(target)
              & (event.shiftKey ? Node.DOCUMENT_POSITION_PRECEDING : Node.DOCUMENT_POSITION_FOLLOWING)));
          if (nativeInRow) return;
          const direction = event.shiftKey ? -1 : 1;
          const next = destination(at + direction, direction);
          if (next !== null) { event.preventDefault(); seek(next, event.shiftKey); }
          return;
        }
        const next = event.key === 'ArrowDown' ? destination(at + 1, 1) : event.key === 'ArrowUp' ? destination(at - 1, -1)
          : event.key === 'Home' ? destination(0, 1) : event.key === 'End' ? destination(items.length - 1, -1) : null;
        if (next !== null && !(event.target instanceof HTMLInputElement)) { event.preventDefault(); seek(next); }
      }}>{children(items[at], at)}</li>)}
  </ol>;
}
