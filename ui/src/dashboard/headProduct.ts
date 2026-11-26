import { useCallback, useContext, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';
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
import { asFailure, RefusedError } from './queryError.ts';
import { restartPipeline, type Pipeline } from './follow/pipeline.ts';
import { coverageOf, useReadsRegion, type ReadCoverage } from './widgetTime.ts';
import { axisHour, regionFollows, type ParamsFor, type Region } from './regionTime.ts';
import type { PanelKey } from '../generated/dashboard';
import type { Gate } from './regionGate';
import { forwardRequest } from './request.ts';
import { holdElement } from '../draw/bitmapSurface.ts';

export const headKey = (widgetKey: string, offset: number, region: Pick<Region, 'action'>) =>
  `${widgetKey}|${offset}${region.action.kind === 'spec' ? '' : region.action.kind === 'at' ? '|detail' : '|follow'}`;

let pinnedOwners = -1000;

export interface HeadProductOptions<T> {
  paint?: (reply: T) => boolean | void;
  active: boolean;
  validate: (data: unknown) => T;
  structure?: (reply: T, hour: number) => string;
}

export interface HeadProduct<T> extends ProductState<T>, ReadCoverage {
  exactRead?: (hour: number) => Promise<unknown>;
  structure?: string;
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

export function useHeadProduct<T, const K extends PanelKey>(runtime: WidgetRuntime, gate: Gate, product: K, wanted: ParamsFor<K> | null,
  options: HeadProductOptions<T>): HeadProduct<T> {
  const { active, validate, paint } = options;
  const { region, track } = gate;
  const params = gate.open ? wanted : null;
  const follows = regionFollows(region);
  const prepared = useContext(PreparingPage);
  const following = active && follows && !prepared && !!track && !!params;
  const hour = follows || !track || !params ? null : gate.hourAt(axisHour(runtime.clock));
  const pinned = active && !follows && !prepared && hour !== null;
  const outside = gate.outside !== null;
  useReadsRegion(region, active);
  const paramsKey = params ? canonical(params) : null;
  const [node, setNode] = useState<Element | null>(null);
  const root = useCallback((next: Element | null) => { if (next) setNode(next); }, []);
  const pipeline = usePipeline(runtime.clock, runtime.client);
  const bounds = useMemo(() => track ? [track.branch.firstHour, track.branch.lastHour] as const : null,
    [track?.branch.firstHour, track?.branch.lastHour]);
  const widgetKey = useWidgetKey();
  const [owner] = useState(() => pinnedOwners--);
  const pipe = useFollowedWidget(pipeline, (following || pinned) && track ? headKey(widgetKey, track.axisOffset, region) : null,
    { offset: track?.axisOffset ?? 0, pin: pinned ? hour : null, bounds, widget: widgetKey, kind: region.widget.kind, key: `${region.widget.kind}|${product}|${paramsKey}` });
  const checked = useMemo(() => following ? gate.checkFollow({ ...params, panel: product, branchId: track!.branch.id }) : null,
    [following, product, paramsKey, track?.branch.id, gate.checkFollow]);
  const query = checked?.query ?? null;
  const follow = useMemo(() => track ? { source: track.source, branch: track.branch.id, offset: track.axisOffset } : null, [track?.source, track?.branch.id, track?.axisOffset]);
  const sub = useFollowedSub(pipeline, pipe, follow, query, 'panel', product);
  const view = useInstalled(pipeline, pipe);
  const model = installedModel(view, sub);
  const [holder] = useState(() => ({}));
  const dim = (following || pinned) && !!view?.dim;
  useLayoutEffect(() => {
    if (!node) return;
    holdElement(node, dim, holder);
    return () => holdElement(node, false, holder);
  }, [node, dim, holder]);
  const panel = installedPanel(model);
  const pushed = following && view?.mode === 'picture' && panel !== undefined;
  const fromPanel = pushed ? panelReply(panel, validate) : {};
  const refusedNamed = checked?.refused ?? (following && view?.mode === 'refused' ? view.reason ?? 'Not in this recording' : undefined);
  const failedNamed = following && view?.mode === 'failed' ? view.reason ?? 'The followed stream failed' : undefined;
  const subError = following && pipeline && sub !== undefined ? (pipeline.status(sub).error ?? (pipe !== null ? pipeline.widgetError(pipe) : undefined)) : undefined;
  const followFailure = useMemo(() => refusedNamed !== undefined ? new RefusedError(refusedNamed)
    : failedNamed !== undefined ? (subError ?? asFailure(new Error(failedNamed))) : undefined, [refusedNamed, failedNamed, subError]);
  const live = useMemo(() => pipeline && pipe !== null && sub !== undefined ? installedLive(pipeline, pipe, sub) : null, [pipeline, pipe, sub]);
  const asked = useFollowedProduct<T>(runtime.client, hour === null ? null : forwardRequest(track!.source, product, { ...params, branchId: track!.branch.id, hour } as unknown as ParamsFor<K>),
    [], active, validate, track?.source);
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
  const exactRead = track && params ? (asking: number) => {
    const recorded = gate.recorded(asking);
    if (recorded === null) return Promise.reject(new Error('This hour was not recorded.'));
    return runtime.client.product(forwardRequest(track.source, product, { ...params, branchId: track.branch.id, hour: recorded } as unknown as ParamsFor<K>),
      new AbortController().signal).then(reply => reply.data);
  } : undefined;
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
    if (!perfLog || !pipeline || pipe === null || !pinned || !auditable) return;
    return perfLog.reader(pipe, owner, async hour => {
      const read = exactNow.current;
      if (!read) return null;
      const raw = await read(hour);
      return { values: new Map(panelAuditRows(product, raw, exactLive(raw)).map(row => [row.key, row.value])) };
    }, pipeline.generation);
  }, [pipeline, pipe, owner, pinned, product, auditable]);
  useLayoutEffect(() => {
    if (!perfLog || !pipeline || pipe === null || sub === undefined || !auditable) return;
    let texted = false;
    pipeline.audit(pipe, sub, fields => {
      const texts = fields[`texts:${sub}`] as ReadonlyMap<string, string> | null | undefined, panel = installedPanel(fields[`model:${sub}`] as ModelField | undefined);
      texted = texts !== undefined;
      if (texts) return [...texts].map(([key, value]) => ({ row: key, key, value }));
      return panel?.data === undefined ? undefined : panelAuditRows(product, panel.data, fields[`live:${sub}`] as ReadonlyMap<string, unknown> | null | undefined);
    });
    const stopReader = perfLog.reader(pipe, sub, async hour => {
      const read = exactNow.current;
      if (!read) return null;
      const raw = await read(hour), live = exactLive(raw);
      const rows = texted ? [...panelTexts(product, raw, live, hour) ?? []].map(([key, value]) => ({ row: key, key, value })) : panelAuditRows(product, raw, live);
      return { values: new Map(rows.map(row => [row.key, row.value])) };
    }, pipeline.generation);
    return () => { stopReader(); pipeline.audit(pipe, sub, null); };
  }, [pipeline, pipe, sub, product, auditable]);
  useRecordedRows(product, pushed && data !== undefined && at !== null && exactRead ? {
    hour: at, rows: () => recordedPanelRows(product, data), exact: async () => recordedPanelRows(product, await exactRead(at)),
  } : undefined, pipe ?? undefined);
  const followRetryable = following && (failedNamed !== undefined || (fromPanel.error !== undefined && !(fromPanel.error instanceof RefusedError)));
  const retry = useCallback(() => {
    if (following) {
      if (followRetryable && pipeline) restartPipeline(pipeline);
      return;
    }
    asked.retry();
  }, [following, followRetryable, pipeline, asked.retry]);
  const shownThrough = () => outside ? null : following ? (pipe === null || !pipeline ? null : pipeline.view(pipe)?.through ?? null) : data === undefined ? null : at;
  return {
    data, error, retry, root, hour, live: shownLive, ...coverageOf(gate.outside),
    structure: outside || !pushed ? undefined : model?.structure,
    exactRead,
    workerFollow: pushed ? sub : undefined,
    followedWidget: following ? pipe ?? undefined : undefined,
    placement: pipe ?? undefined,
    pipeline,
    at, atNow: () => outside ? null : following ? datedStore(pipeline, pipe).read() : at, shownAt: shownThrough,
    current: () => data,
    inspected: () => {
      if (!pushed || !pipeline || pipe === null || sub === undefined) return undefined;
      const shown = pipeline.view(pipe), texts = shown?.fields?.[`texts:${sub}`] as ReadonlyMap<string, string> | null | undefined, hour = shown?.painted ?? shown?.h;
      return shown?.mode === 'picture' && !shown.dim && texts && hour !== undefined ? { texts, hour, data: panelReply(installedPanel(installedModel(shown, sub)), validate).data } : undefined;
    },
    read: outside ? 'off' : follows ? 'followed' : hour === null ? 'off' : 'asked',
  };
}
