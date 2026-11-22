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

function write(field: HTMLElement, label: string, value: string) {
  if (label !== 'bar') { if (field.textContent !== value) field.textContent = value; return; }
  const meter = field.closest<HTMLElement>('.progress-meter');
  const ratio = value === '' ? null : Math.min(1, Math.max(0, Number(value)));
  const vertical = meter?.classList.contains('is-vertical');
  field.style.setProperty(vertical ? 'height' : 'width', ratio === null ? '0%' : `${ratio * 100}%`);
  const aria = field.closest('[role="meter"]');
  if (ratio === null) { aria?.removeAttribute('aria-valuenow'); aria?.setAttribute('aria-valuetext', 'Not recorded'); }
  else { aria?.setAttribute('aria-valuenow', value); aria?.removeAttribute('aria-valuetext'); }
}

export interface DrawnPanel { inspected(): PanelInspection | null | undefined }

export function useDrawnPanel(root: RefObject<HTMLElement | null>, _clock: PlaybackController, feed: PanelFeed | null, active: boolean): DrawnPanel {
  const latest = useRef(feed);
  latest.current = feed;
  const pipeline = feed?.pipeline ?? null, pipe = feed?.pipe, sub = feed?.follow, view = feed?.view;
  useLayoutEffect(() => {
    const node = root.current;
    if (!active || !node || !pipeline || pipe === undefined || sub === undefined || !view) return;
    pipeline.present(pipe, 'panel');
    const layout = panelViews[view]?.geometry ?? {};
    const owned = layout.owns ?? (() => true);
    const selector = layout.extraFields ? commonFields + ',' + layout.extraFields : commonFields;
    const paint = () => {
      const shown = pipeline.view(pipe);
      if (shown?.dim) node.setAttribute('data-held', 'stale'); else node.removeAttribute('data-held');
      const texts = shown?.mode === 'picture' ? shown.fields?.[`texts:${sub}`] as Map<string, string> | null | undefined : undefined;
      if (!texts) return;
      for (const field of node.querySelectorAll<HTMLElement>(selector)) {
        if (!owned(field)) continue;
        const { key, label } = identity(field, layout), value = texts.get(key);
        if (value !== undefined) write(field, label, value);
      }
    };
    paint();
    const stop = pipeline.onInstall(pipe, paint);
    return () => { stop(); node.removeAttribute('data-held'); };
  }, [root, active, pipeline, pipe, sub, view, feed?.structure]);
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
