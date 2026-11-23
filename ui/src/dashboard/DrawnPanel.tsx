import { useLayoutEffect, useRef } from 'react';
import type { RefObject } from 'react';
import { panelViews } from '../draw/panels/views.ts';
import type { PanelFeed } from './panelFeed';
import type { PanelGeometry } from '../draw/panels/view';
import type { PlaybackController } from './playback';
import type { LiveMap } from '../draw/views/fields';
import './drawnPanel.css';

export interface PanelInspection { texts: ReadonlyMap<string, string>; live: ReadonlyMap<string, number | string>; date: { since: number; through: number } }

const commonFields = '[data-paint],[data-panel-label],.progress-meter > i';

function identity(field: HTMLElement, { rowAttribute }: PanelGeometry) {
  const row = rowAttribute ? field.closest('[' + rowAttribute + ']')?.getAttribute(rowAttribute) ?? undefined : undefined;
  const label = field.dataset.paint ?? field.dataset.panelLabel ?? 'bar';
  return { key: row === undefined ? label : row + ':' + label, label };
}

const NOT_RECORDED = 'Not recorded';

function write(field: HTMLElement, label: string, value: string | undefined, { barVariable }: PanelGeometry) {
  if (label !== 'bar') { const text = value ?? NOT_RECORDED; if (field.textContent !== text) field.textContent = text; return; }
  const ratio = value === undefined || value === '' ? null : Math.min(1, Math.max(0, Number(value)));
  const host = barVariable ? field.closest<HTMLElement>(barVariable.host) : null;
  if (barVariable && host) host.style.setProperty(barVariable.name, ratio === null ? '0' : String(ratio));
  else field.style.setProperty(field.closest('.progress-meter')?.classList.contains('is-vertical') ? 'height' : 'width', ratio === null ? '0%' : `${ratio * 100}%`);
  const aria = field.closest('[role="meter"]');
  if (ratio === null) { aria?.removeAttribute('aria-valuenow'); aria?.setAttribute('aria-valuetext', NOT_RECORDED); }
  else { aria?.setAttribute('aria-valuenow', String(ratio)); aria?.removeAttribute('aria-valuetext'); }
}

export function paintFields(node: HTMLElement, texts: ReadonlyMap<string, string> | null, layout: PanelGeometry) {
  const selector = layout.extraFields ? commonFields + ',' + layout.extraFields : commonFields;
  for (const field of node.querySelectorAll<HTMLElement>(selector)) {
    const { key, label } = identity(field, layout);
    write(field, label, texts?.get(key), layout);
  }
}

export interface DrawnPanel { inspected(): PanelInspection | null | undefined }

export function useDrawnPanel(root: RefObject<HTMLElement | null>, _clock: PlaybackController, feed: PanelFeed | null, active: boolean): DrawnPanel {
  const latest = useRef(feed), painter = useRef<(() => void) | null>(null);
  latest.current = feed;
  const pipeline = feed?.pipeline ?? null, pipe = feed?.pipe, sub = feed?.follow, view = feed?.view;
  useLayoutEffect(() => {
    const node = root.current;
    if (!active || !node || !pipeline || pipe === undefined || sub === undefined || !view) return;
    pipeline.present(pipe, 'panel');
    const layout = panelViews[view]?.geometry ?? {};
    const paint = () => {
      const shown = pipeline.view(pipe);
      if (shown?.dim) node.setAttribute('data-held', 'stale'); else node.removeAttribute('data-held');
      if (shown?.mode !== 'picture') return;
      paintFields(node, (shown.fields?.[`texts:${sub}`] as Map<string, string> | null | undefined) ?? null, layout);
    };
    painter.current = paint;
    paint();
    const stop = pipeline.onInstall(pipe, paint);
    return () => { stop(); painter.current = null; node.removeAttribute('data-held'); };
  }, [root, active, pipeline, pipe, sub, view, feed?.structure]);
  useLayoutEffect(() => { painter.current?.(); });
  return useRef<DrawnPanel>({ inspected: () => {
    const held = latest.current;
    if (!held?.pipeline || held.pipe === undefined || held.follow === undefined) return undefined;
    const shown = held.pipeline.view(held.pipe);
    if (!shown || shown.mode !== 'picture' || shown.dim || shown.h === undefined || shown.through === undefined) return null;
    const texts = shown.fields?.[`texts:${held.follow}`] as Map<string, string> | null | undefined;
    const live = shown.fields?.[`live:${held.follow}`] as LiveMap | null | undefined;
    return texts ? { texts, live: live ?? new Map(), date: { since: shown.h, through: shown.through } } : null;
  } }).current;
}
