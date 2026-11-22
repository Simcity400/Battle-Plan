import { useLayoutEffect, useRef } from 'react';
import type { RefObject } from 'react';
import { hideTooltip, tooltipStore } from '../charts/tooltip.ts';
import { panelValues, panelViews } from '../draw/panels/views.ts';
import { attachPanel } from '../draw/host.ts';
import type { PanelFeed } from './panelFeed';
import type { PanelGeometry, RowCell } from '../draw/panels/view';
import type { PlaybackController } from './playback';
import { measuredWidget } from './widgetPerf.ts';
import { perfLog } from '../shell/perf.ts';
import './drawnPanel.css';

let metrics: CanvasRenderingContext2D | null | undefined;
const commonFields = '[data-paint],[data-panel-label],.progress-meter > i';

function identity(field: HTMLElement, { rowAttribute }: PanelGeometry) {
  const row = rowAttribute ? field.closest('[' + rowAttribute + ']')?.getAttribute(rowAttribute) ?? undefined : undefined;
  const label = field.dataset.paint ?? field.dataset.panelLabel ?? 'bar';
  return { row: row ?? label, key: row === undefined ? label : row + ':' + label, label };
}

function geometry(root: HTMLElement, fields: HTMLElement[], layout: PanelGeometry) {
  const origin = root.getBoundingClientRect(), cells: RowCell[] = [];
  const left = origin.left + root.clientLeft - root.scrollLeft, top = origin.top + root.clientTop - root.scrollTop;
  metrics ??= document.createElement('canvas').getContext('2d');
  if (!metrics) return cells;
  const range = document.createRange();
  for (const field of fields) {
    const { row, key, label } = identity(field, layout);
    const style = getComputedStyle(field);
    if (label === 'bar') {
      const meter = field.closest<HTMLElement>(layout.meters ? '.progress-meter,' + layout.meters : '.progress-meter');
      if (!meter) continue;
      const box = meter.getBoundingClientRect();
      if (!box.width || !box.height) continue;
      cells.push({ key, row, text: '', x: box.left - left, y: box.top - top, width: box.width, height: box.height,
        color: style.backgroundColor, baseline: 0, font: '', align: 'left',
        meter: meter.classList.contains('is-vertical') ? 'vertical' : 'horizontal',
        knob: layout.knob?.(field) });
      continue;
    }
    range.selectNodeContents(field);
    const box = range.getBoundingClientRect(), slot = field.getBoundingClientRect(), text = field.textContent ?? '';
    if (!slot.height || !slot.width) continue;
    const bounds = field.hasAttribute('data-paint') ? slot : box;
    const right = style.textAlign === 'right' || style.textAlign === 'end' && style.direction === 'ltr' ||
      style.textAlign === 'start' && style.direction === 'rtl';
    let width = bounds.width;
    if (style.display === 'inline' && !right && style.textAlign !== 'center' && layout.extendInline && field.matches(layout.extendInline)) {
      const line = field.closest<HTMLElement>('summary,p,li,dd,pre');
      if (line) width = Math.max(width, line.getBoundingClientRect().right - parseFloat(getComputedStyle(line).paddingRight) - bounds.left);
    }
    metrics.font = style.fontWeight + ' ' + (parseFloat(style.fontSize) * devicePixelRatio) + 'px ' + style.fontFamily;
    const baseline = (box.height ? box.top : slot.top) - top + metrics.measureText(text || '0').fontBoundingBoxAscent / devicePixelRatio;
    cells.push({ key, row, text, baseline, x: bounds.left - left, y: bounds.top - top,
      width, height: bounds.height, align: right ? 'right' : style.textAlign === 'center' ? 'center' : 'left',
      font: style.fontWeight + ' ' + style.fontSize + ' ' + style.fontFamily, color: style.color });
  }
  return cells;
}

interface Attached {
  node: HTMLElement; clock: PlaybackController; widget: ReturnType<typeof measuredWidget>; view: PanelFeed['view'];
  inputs: readonly unknown[]; reconcile: (rebinding: boolean) => void; close: () => void;
}

export function useDrawnPanel(root: RefObject<HTMLElement | null>, clock: PlaybackController, feed: PanelFeed | null, active: boolean) {
  const widget = measuredWidget(), latest = useRef(feed);
  latest.current = feed;
  const attached = useRef<Attached | null>(null);
  const binding = feed && `${feed.follow}|${feed.fixed}|${feed.offset}`;
  const inputs = [binding, feed?.structure, feed?.reply, feed?.follow === undefined ? feed?.date?.through : undefined];
  useLayoutEffect(() => {
    const node = active && feed ? root.current : null, view = feed?.view;
    const held = attached.current;
    const kept = held?.node === node && held.clock === clock && held.widget === widget && held.view === view ? held : null;
    if (held && !kept) { held.close(); attached.current = null; }
    if (!kept) {
      if (!node || !view) return;
      attached.current = { node, clock, widget, view, inputs, ...attachRoot(node, view) };
      attached.current.reconcile(false);
      return;
    }
    if (inputs.every((input, at) => input === kept.inputs[at])) return;
    const rebinding = kept.inputs[0] !== binding;
    kept.inputs = inputs;
    kept.reconcile(rebinding);
  });
  useLayoutEffect(() => () => { attached.current?.close(); attached.current = null; }, []);

  function attachRoot(node: HTMLElement, view: PanelFeed['view']) {
    const layout = panelViews[view]?.geometry ?? {};
    const owned = layout.owns ?? (() => true);
    const fieldsSelector = layout.extraFields ? commonFields + ',' + layout.extraFields : commonFields;
    const layer = (field: HTMLElement) => (layout.layers ? field.closest<HTMLElement>(layout.layers) : null) ?? node;
    const fieldsIn = (content: HTMLElement) => [...content.querySelectorAll<HTMLElement>(fieldsSelector)].filter(f => {
      if (!owned(f) || layer(f) !== content) return false;
      for (let closed = f.closest('details:not([open])'); closed; closed = closed.parentElement?.closest('details:not([open])') ?? null) {
        if (!closed.querySelector(':scope > summary')?.contains(f)) return false;
      }
      return true;
    });
    const handles = new Map<HTMLElement, { refresh: () => void; rebind: (exact: boolean) => void; inspect: () => Promise<ReadonlyMap<string, string> | undefined>; close: () => void; readonly stale: boolean }>();
    let interaction = 0;
    const hold = () => {
      interaction++;
      const tip = tooltipStore.get();
      if (tip && node.matches(':hover, :focus-within')) hideTooltip(tip.owner);
    };
    const attach = (content: HTMLElement) => {
      let fields: HTMLElement[] = [];
      const model = (id: number, owner: number) => {
        const reading = latest.current;
        if (!reading) return null;
        fields = fieldsIn(content);
        const { exactRead, ...model } = reading;
        if (exactRead) perfLog?.reader(id, owner, async hour => {
          const raw = await exactRead(hour);
          const values = panelValues(view, raw, null, hour, reading.structure);
          return values && { values };
        });
        return { ...model, widget: widget ?? undefined, cells: geometry(content, fields, layout) };
      };
      const own = (accepted: boolean, pending = false) => {
        content.toggleAttribute('data-worker-panel', accepted);
        for (const field of fields) field.dataset.rowInk = accepted ? 'worker' : pending || clock.getSnapshot().playing && latest.current?.fixed === undefined ? 'pending' : 'page';
      };
      const hit = (row: string | null) => {
        if (row !== null) content.dataset.drawnRow = row;
        else { delete content.dataset.drawnRow; }
      };
      const handle = attachPanel(content, clock, model, own, hit, hold);
      if (handle) handles.set(content, { refresh: handle.refresh, rebind: handle.rebind, inspect: handle.inspect, get stale() { return handle.stale; }, close() {
        handle.close();
        for (const f of fields) delete f.dataset.rowInk;
      } });
    };
    const held = (target: HTMLElement) => handles.get(layer(target))?.stale === true;
    const reconcile = (rebinding: boolean) => {
      const feed = latest.current, exact = !!feed && !feed.held && (feed.fixed !== undefined || !clock.getSnapshot().playing);
      const contents = new Set([...node.querySelectorAll<HTMLElement>(fieldsSelector)].filter(owned).map(layer));
      for (const [content, handle] of handles) {
        if (!contents.has(content)) { handle.close(); handles.delete(content); }
        else if (rebinding) handle.rebind(exact);
        else handle.refresh();
      }
      for (const content of contents) if (!handles.has(content)) attach(content);
    };
    const inspect = async (event: Event) => {
      const target = event.target;
      if (!event.isTrusted || !(target instanceof HTMLElement) || !owned(target)) return;
      if (held(target)) { event.stopPropagation(); return; }
      const content = layer(target), handle = handles.get(content);
      if (!handle || latest.current?.follow === undefined) return;
      const ticket = ++interaction;
      event.stopPropagation();
      const values = await handle.inspect();
      if (ticket !== interaction || handle.stale || !node.contains(target)) return;
      if (values && handles.get(content) === handle) for (const field of fieldsIn(content)) {
        const { key, label } = identity(field, layout), value = values.get(key);
        if (value === undefined) continue;
        if (label === 'bar') {
          const meter = field.closest('[role="meter"]');
          if (value !== '') { meter?.setAttribute('aria-valuenow', value); meter?.removeAttribute('aria-valuetext'); }
          else { meter?.removeAttribute('aria-valuenow'); meter?.setAttribute('aria-valuetext', 'Not recorded'); }
        } else if (field.textContent !== value) field.textContent = value;
      }
      if (event instanceof PointerEvent) target.dispatchEvent(new PointerEvent(event.type, event));
      else if (event instanceof FocusEvent) target.dispatchEvent(new FocusEvent(event.type, event));
    };
    const cancel = () => { interaction++; };
    let seek = clock.getSnapshot().seekSequence;
    const stop = clock.subscribe(() => {
      const s = clock.getSnapshot();
      if (seek !== s.seekSequence) {
        seek = s.seekSequence; cancel();
        const tip = tooltipStore.get();
        if (tip && !s.forwardSeek) hideTooltip(tip.owner);
      }
    });
    node.addEventListener('pointermove', inspect, true); node.addEventListener('focusin', inspect, true);
    node.addEventListener('focusout', cancel); node.addEventListener('pointerleave', cancel);
    return { reconcile, close() {
      cancel(); stop(); node.removeEventListener('pointermove', inspect, true); node.removeEventListener('focusin', inspect, true);
      node.removeEventListener('focusout', cancel); node.removeEventListener('pointerleave', cancel);
      handles.forEach(handle => handle.close());
    } };
  }
}
