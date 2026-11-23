import { useCallback, useContext, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack, WidgetSpec } from './model';
import { resolveWindow } from './model.ts';
import { useFollowedProduct } from './products.ts';
import type { ProductState } from './products';
import type { Failure } from './queryError';
import { panelReply } from './follow/panel.ts';
import { canonical } from './follow/canonical.ts';
import { useWidgetKey } from './follow/useFollow.ts';
import { datedStore, installedLive, installedModel, installedPanel, useFollowedSub, useFollowedWidget, useInstalled, usePipeline, useSurfaceLabel } from './follow/useFollowed.ts';
import type { ModelField } from '../draw/views/fields';
import { perfLog } from '../shell/perf.ts';
import { exactLive, panelAuditRows, panelTexts } from './follow/panelAudit.ts';
import { repliedCells } from './follow/liveCells.ts';
import type { Live, LiveValues } from '../charts/live';
import { PreparingPage } from './preparing.ts';
import { useRecordedRows } from './widgetPerf.ts';
import { recordedPanelRows } from '../draw/panels/views.ts';
import { asFailure } from './queryError.ts';
import type { Pipeline } from './follow/pipeline';

export function headRead(widget: Pick<WidgetSpec, 'time' | 'kind' | 'trackIds'>, track: BoundTrack | undefined, tracks: BoundTrack[],
  hoverHour: number | null): { follows: boolean; hourAt: (clockHour: number) => number | null } {
  return {
    follows: widget.time.mode === 'follow' && hoverHour === null,
    hourAt: clockHour => {
      if (!track) return null;
      const hour = hoverHour ?? resolveWindow(widget as WidgetSpec, track, tracks, clockHour)?.lastHour;
      return hour === undefined || hour < track.branch.firstHour || hour > track.branch.lastHour ? null : Math.floor(hour);
    },
  };
}

export const headKey = (widgetKey: string, offset: number, hoverHour: number | null) => `${widgetKey}|${offset}${hoverHour === null ? '' : '|detail'}`;

let pinnedOwners = -1000;

export interface HeadProductOptions<T> {
  paint?: (reply: T) => boolean | void;
  active: boolean;
  validate: (data: unknown) => T;
  structure?: (reply: T, hour: number) => string;
  hoverHour?: number | null;
}

export interface HeadProduct<T> extends ProductState<T> {
  exactRead?: (hour: number) => Promise<unknown>;
  structure?: string;
  outside: boolean;
  workerFollow?: number;
  followedWidget?: number;
  placement?: number;
  pipeline?: Pipeline | null;
  root: (element: Element | null) => void;
  read: 'followed' | 'asked' | 'off';
  reason?: string;
  hour: number | null;
  at: number | null;
  atNow: () => number | null;
  live: Live<LiveValues<number>>;
  shownAt: () => number | null;
  current: () => T | undefined;
  inspected: () => { texts: ReadonlyMap<string, string>; hour: number; data: T | undefined } | undefined;
}

export function momentHour(moment: string | undefined): number | null {
  if (!moment) return null;
  const [hour] = JSON.parse(moment) as unknown[];
  return typeof hour === 'number' ? hour : null;
}

export function headSubject(track: Pick<BoundTrack, 'source' | 'branch'> | undefined, product: string, params: Readonly<Record<string, unknown>> | null): string | null {
  return track && params ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, product, canonical(params)]) : null;
}

export interface Retained<T> { client: unknown; subject: string; data: T; hour: number | null }
type TargetHour = { exact: number | null } | { through: number };

export function retain<T>(held: Retained<T> | null, client: unknown, subject: string | null, asked: T | undefined, hour: number | null, target?: TargetHour): Retained<T> | null {
  if (subject === null) return null;
  const eligible = (at: number | null) => !target || (at !== null && ('exact' in target ? at === target.exact : at <= target.through));
  if (asked !== undefined && eligible(hour)) return { client, subject, data: asked, hour };
  return held && held.client === client && held.subject === subject && eligible(held.hour) ? held : null;
}

export function useHeadProduct<T>(runtime: WidgetRuntime, widget: WidgetSpec, product: string, params: Readonly<Record<string, unknown>> | null,
  options: HeadProductOptions<T>): HeadProduct<T> {
  const { active, validate, paint, hoverHour = null } = options;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const { follows, hourAt } = useMemo(() => headRead(widget, track, runtime.tracks, hoverHour), [widget, track, runtime.tracks, hoverHour]);
  const prepared = useContext(PreparingPage);
  const following = active && follows && !prepared && !!track && !!params;
  const hour = follows || !track || !params ? null : hourAt(runtime.clock.getHour());
  const pinned = active && !follows && !prepared && hour !== null;
  const paramsKey = params ? JSON.stringify(params) : null;
  const [node, setNode] = useState<Element | null>(null);
  const root = useCallback((next: Element | null) => { if (next) setNode(next); }, []);
  const pipeline = usePipeline(runtime.clock, runtime.client);
  const bounds = useMemo(() => track ? [track.branch.firstHour, track.branch.lastHour] as const : null,
    [track?.branch.firstHour, track?.branch.lastHour]);
  const widgetKey = useWidgetKey();
  const [owner] = useState(() => pinnedOwners--);
  const pipe = useFollowedWidget(pipeline, (following || pinned) && track ? headKey(widgetKey, track.axisOffset, hoverHour) : null,
    { offset: track?.axisOffset ?? 0, pin: pinned ? hour : null, bounds, widget: widgetKey, key: `${widget.kind}|${product}|${paramsKey}` });
  const query = useMemo(() => following ? { ...params, panel: product, branchId: track!.branch.id } : null,
    [following, product, paramsKey, track?.branch.id]);
  const follow = useMemo(() => track ? { source: track.source, branch: track.branch.id, offset: track.axisOffset } : null, [track?.source, track?.branch.id, track?.axisOffset]);
  const sub = useFollowedSub(pipeline, pipe, follow, query, 'panel', product);
  const view = useInstalled(pipeline, pipe);
  const model = installedModel(view, sub);
  const panel = installedPanel(model);
  const pushed = following && view?.mode === 'picture' && panel !== undefined;
  const fromPanel = pushed ? panelReply(panel, validate) : {};
  const named = following && (view?.mode === 'failed' || view?.mode === 'refused') ? view.reason ?? 'The followed stream failed' : undefined;
  const followFailure = useMemo(() => named === undefined ? undefined : asFailure(new Error(named)), [named]);
  const live = useMemo(() => pipeline && pipe !== null && sub !== undefined ? installedLive(pipeline, pipe, sub) : null, [pipeline, pipe, sub]);
  const outside = following ? view?.mode === 'outside' : !follows && !!track && !!params && hour === null;
  const asked = useFollowedProduct<T>(runtime.client, hour === null ? null : { sourceId: track!.source.id, revision: track!.source.revision,
    product, params: { ...params, branchId: track!.branch.id, hour } }, [], active, validate, track?.source);
  const askedAt = asked.data === undefined ? null : momentHour(asked.moment) ?? hour;
  const held = useRef<Retained<T> | null>(null);
  const subject = headSubject(track, product, params);
  held.current = following ? null : retain(held.current, runtime.client, subject, asked.data, askedAt, { exact: hour });
  const shown = held.current ?? undefined;
  const data = outside ? undefined : following ? fromPanel.data : shown?.data;
  const error: Failure | undefined = outside ? undefined : following ? followFailure ?? fromPanel.error : asked.error;
  const askedLive = useMemo(() => repliedCells(following ? undefined : data), [following, data]);
  const shownLive = following && live ? live : askedLive;
  const at = outside ? null : following ? (pushed ? view!.painted ?? view!.h ?? null : null) : shown?.hour ?? null;
  const latest = useRef(paint);
  latest.current = paint;
  useLayoutEffect(() => { if (data !== undefined) latest.current?.(data); }, [data]);
  const exactRead = track && params ? (at: number) => runtime.client.product({ sourceId: track.source.id, revision: track.source.revision, product,
    params: { ...params, branchId: track.branch.id, hour: at } }, new AbortController().signal).then(reply => reply.data) : undefined;
  const exactNow = useRef(exactRead);
  exactNow.current = exactRead;
  const auditable = !!exactRead;
  useSurfaceLabel(node, (following || pinned) && auditable, pipe);
  useLayoutEffect(() => {
    if (!pipeline || pipe === null || !pinned) return;
    pipeline.answer(pipe, owner, data === undefined || at === null ? null : { hour: at, rows: () => auditable ? panelAuditRows(product, data, exactLive(data)) : undefined });
    return () => pipeline.answer(pipe, owner, null);
  }, [pipeline, pipe, owner, pinned, data, at, product, auditable]);
  useLayoutEffect(() => {
    if (!perfLog || pipe === null || !pinned || !auditable) return;
    perfLog.reader(pipe, owner, async hour => {
      const read = exactNow.current;
      if (!read) return null;
      const raw = await read(hour);
      return { values: new Map(panelAuditRows(product, raw, exactLive(raw)).map(row => [row.key, row.value])) };
    });
  }, [pipe, owner, pinned, product, auditable]);
  useLayoutEffect(() => {
    if (!perfLog || !pipeline || pipe === null || sub === undefined || !auditable) return;
    let texted = false;
    pipeline.audit(pipe, sub, fields => {
      const texts = fields[`texts:${sub}`] as ReadonlyMap<string, string> | null | undefined, panel = installedPanel(fields[`model:${sub}`] as ModelField | undefined);
      texted = texts !== undefined;
      if (texts) return [...texts].map(([key, value]) => ({ row: key, key, value }));
      return panel?.data === undefined ? undefined : panelAuditRows(product, panel.data, fields[`live:${sub}`] as ReadonlyMap<string, unknown> | null | undefined);
    });
    perfLog.reader(pipe, sub, async hour => {
      const read = exactNow.current;
      if (!read) return null;
      const raw = await read(hour), live = exactLive(raw);
      const rows = texted ? [...panelTexts(product, raw, live, hour) ?? []].map(([key, value]) => ({ row: key, key, value })) : panelAuditRows(product, raw, live);
      return { values: new Map(rows.map(row => [row.key, row.value])) };
    });
    return () => pipeline.audit(pipe, sub, null);
  }, [pipeline, pipe, sub, product, auditable]);
  useRecordedRows(product, pushed && data !== undefined && at !== null && exactRead ? {
    hour: at, rows: () => recordedPanelRows(product, data), exact: async () => recordedPanelRows(product, await exactRead(at)),
  } : undefined, pipe ?? undefined);
  const shownThrough = () => following ? (pipe === null || !pipeline ? null : pipeline.view(pipe)?.through ?? null) : data === undefined ? null : at;
  return {
    data, error, retry: asked.retry, root, hour, live: shownLive, outside,
    structure: outside || !pushed ? undefined : model?.structure,
    exactRead,
    workerFollow: pushed ? sub : undefined,
    followedWidget: following ? pipe ?? undefined : undefined,
    placement: pipe ?? undefined,
    pipeline,
    at, atNow: () => following ? datedStore(pipeline, pipe).read() : at, shownAt: shownThrough,
    current: () => data,
    inspected: () => {
      if (!pushed || !pipeline || pipe === null || sub === undefined) return undefined;
      const shown = pipeline.view(pipe), texts = shown?.fields?.[`texts:${sub}`] as ReadonlyMap<string, string> | null | undefined, hour = shown?.painted ?? shown?.h;
      return shown?.mode === 'picture' && texts && hour !== undefined ? { texts, hour, data: panelReply(installedPanel(installedModel(shown, sub)), validate).data } : undefined;
    },
    read: outside ? 'off' : following ? 'followed' : hour === null ? 'off' : 'asked',
  };
}
