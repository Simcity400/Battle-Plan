import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { canonical } from '../../dashboard/follow/canonical.ts';
import { restartPipeline, type Pipeline } from '../../dashboard/follow/pipeline.ts';
import { useElement, useFollowedWidget, usePipeline, useSurfaceLabel } from '../../dashboard/follow/useFollowed.ts';
import { useWidgetKey } from '../../dashboard/follow/useFollow.ts';
import { useWidgetFrame } from '../../dashboard/widgetFrame.tsx';
import { bitmapSurface, holdElement } from '../../draw/bitmapSurface.ts';
import { perfLog } from '../../shell/perf.ts';
import { AuditSnapshotStore, auditRowsOf, isFamilyAuditing, subscribeFamilyAuditing } from '../audit.ts';
import type { FamilySourceSubscription } from '../contract.ts';
import { createBoundAuditReader, type BoundAuditEntry, type FamilyAuditVerifier } from './auditBinding.ts';
import { FamilyTooltipController } from './tooltips.ts';
import { watchDevicePixelRatio, windowRatio } from '../../charts/surface.ts';

const subscribeAuditing = (listener: () => void) => subscribeFamilyAuditing(() => { listener(); });

const STATIC_HOLD = {};
let nextInstanceSeq = 1;

const alignedCanvases = new Map<HTMLCanvasElement, { x: number; y: number }>();
const alignedParents = new Map<Element, number>();
let alignmentFrame = 0;
let alignmentWatch: { resize: ResizeObserver; moved: MutationObserver; stopRatio: () => void } | null = null;

function alignCanvases() {
  alignmentFrame = 0;
  const ratio = windowRatio();
  const offsets = [...alignedCanvases].map(([canvas, previous]) => {
    const box = canvas.getBoundingClientRect(), zoom = canvas.currentCSSZoom || 1;
    const unit = 64 * ratio * zoom;
    const snap = (at: number, offset: number) => {
      const base = at - offset * zoom;
      return Math.round((Math.round(base * ratio) / ratio - base) / zoom * unit) / unit;
    };
    return { canvas, x: snap(box.left, previous.x), y: snap(box.top, previous.y), unit };
  });
  for (const { canvas, x, y, unit } of offsets) {
    alignedCanvases.set(canvas, { x, y });
    const left = `${x + 1e-4 / unit}px`, top = `${y + 1e-4 / unit}px`;
    if (canvas.style.left !== left) canvas.style.marginLeft = canvas.style.left = left;
    if (canvas.style.top !== top) canvas.style.marginTop = canvas.style.top = top;
  }
}

function scheduleAlignment() {
  if (!alignmentFrame) alignmentFrame = requestAnimationFrame(alignCanvases);
}

function alignCanvas(canvas: HTMLCanvasElement): () => void {
  if (!alignmentWatch) {
    alignmentWatch = { resize: new ResizeObserver(scheduleAlignment), moved: new MutationObserver(scheduleAlignment), stopRatio: watchDevicePixelRatio(scheduleAlignment) };
    document.addEventListener('scroll', scheduleAlignment, true);
    document.addEventListener('transitionend', scheduleAlignment, true);
  }
  alignedCanvases.set(canvas, { x: parseFloat(canvas.style.marginLeft) || 0, y: parseFloat(canvas.style.marginTop) || 0 });
  const parents: Element[] = [];
  for (let parent = canvas.parentElement?.parentElement; parent; parent = parent.parentElement) {
    parents.push(parent);
    alignedParents.set(parent, (alignedParents.get(parent) ?? 0) + 1);
    alignmentWatch.resize.observe(parent);
    alignmentWatch.moved.observe(parent, { attributes: true, attributeFilter: ['style', 'class'] });
  }
  scheduleAlignment();
  return () => {
    alignedCanvases.delete(canvas);
    for (const parent of parents) {
      const count = alignedParents.get(parent)! - 1;
      if (count) alignedParents.set(parent, count);
      else { alignedParents.delete(parent); alignmentWatch!.resize.unobserve(parent); }
    }
    alignmentWatch!.moved.disconnect();
    for (const parent of alignedParents.keys()) alignmentWatch!.moved.observe(parent, { attributes: true, attributeFilter: ['style', 'class'] });
    if (alignedCanvases.size) return;
    alignmentWatch!.resize.disconnect();
    alignmentWatch!.stopRatio();
    alignmentWatch = null;
    document.removeEventListener('scroll', scheduleAlignment, true);
    document.removeEventListener('transitionend', scheduleAlignment, true);
    cancelAnimationFrame(alignmentFrame);
    alignmentFrame = 0;
  };
}

export function useFamilyInstanceId(): number {
  const idRef = useRef<number | null>(null);
  if (idRef.current === null) idRef.current = nextInstanceSeq++;
  return idRef.current;
}

export interface FamilyPlacementOptions {
  readonly family: string;
  readonly declarationKind: string;
  readonly widgetId: string;
  readonly widgetKind: string;
  readonly owner?: string;
  readonly subject?: string;
  readonly widgetTrackIds?: readonly string[];
  readonly dataContext?: string;
  readonly runtime: WidgetRuntime;
  readonly enabled: boolean;
  readonly auditable: boolean;
  readonly subscriptions: readonly FamilySourceSubscription[];
  readonly offset: number;
  readonly pin: number | null;
  readonly bounds?: readonly [number, number] | null;
  readonly frameRef: { readonly current: HTMLElement | null };
  readonly surfaceLabel?: string;
}

export interface FamilyPlacementHandle {
  readonly pipeline: Pipeline | null;
  readonly pipe: number | null;
  readonly sourceIdsRef: { readonly current: Readonly<Record<string, number>> };
  readonly subsReady: number;
  readonly instanceId: number;
  readonly tooltipOwner: string;
  readonly dataContextKey: string;
  readonly retryStream: () => void;
  readonly retryLocal: (afterRetry?: () => void) => void;
}

export function useFamilyPlacement(options: FamilyPlacementOptions): FamilyPlacementHandle {
  const {
    family,
    declarationKind,
    widgetId,
    widgetKind,
    owner,
    subject = '',
    widgetTrackIds,
    dataContext = '',
    runtime,
    enabled,
    auditable,
    subscriptions,
    offset,
    pin,
    bounds,
    frameRef,
    surfaceLabel = family,
  } = options;

  const instanceId = useFamilyInstanceId();
  const pipeline = usePipeline(runtime.clock, runtime.client);
  const widgetKey = useWidgetKey();
  const pipeKey = enabled && pipeline ? `${widgetKey}|${offset}|${declarationKind}|${instanceId}` : null;
  const pipe = useFollowedWidget(pipeline, pipeKey, {
    offset,
    pin,
    ...(owner !== undefined ? { owner } : {}),
    bounds,
    widget: widgetKey,
    kind: `${widgetKind}:${declarationKind}`,
    key: `${widgetId}|${declarationKind}|${instanceId}`,
  });

  const frameElement = useElement(frameRef);
  useSurfaceLabel(frameElement, enabled && auditable, pipe, surfaceLabel);

  const subsKey = useMemo(
    () => JSON.stringify(subscriptions.map(s => [s.name, s.source.id, s.source.revision, s.branch, s.offset, s.channel, s.product ?? '', canonical(s.query)])),
    [subscriptions],
  );
  const trackContext = widgetTrackIds && widgetTrackIds.length > 0
    ? widgetTrackIds.map(id => {
        const track = runtime.tracks.find(item => item.id === id);
        return track ? [track.source.id, track.source.revision, track.branch.id] : [id];
      })
    : runtime.tracks.map(track => [track.source.id, track.source.revision, track.branch.id]);
  const subsContext = subscriptions.map(s => [s.source.id, s.source.revision, s.branch]);
  const dataContextKey = JSON.stringify([declarationKind, owner ?? '', subject, trackContext, subsContext, dataContext]);

  const subsRef = useRef(subscriptions);
  subsRef.current = subscriptions;
  const sourceIdsRef = useRef<Record<string, number>>({});
  const [subsReady, setSubsReady] = useState(0);

  useLayoutEffect(() => {
    if (!pipeline || pipe === null || !enabled || !subsRef.current.length) {
      sourceIdsRef.current = {};
      return;
    }
    const tokens: object[] = [];
    const ids: Record<string, number> = {};
    for (const sub of subsRef.current) {
      const token = {};
      tokens.push(token);
      const id = pipeline.subscribe(pipe, token, sub.source, sub.branch, sub.offset, sub.query, sub.channel, sub.product);
      if (id !== undefined) ids[sub.name] = id;
    }
    sourceIdsRef.current = ids;
    setSubsReady(n => n + 1);
    return () => {
      sourceIdsRef.current = {};
      for (const token of tokens) pipeline.unsubscribe(pipe, token);
    };
  }, [pipeline, pipe, enabled, subsKey]);

  const tooltipOwner = `family-${family}:${widgetId}:${instanceId}`;

  const retryStream = useCallback(() => {
    if (pipeline) restartPipeline(pipeline);
  }, [pipeline]);

  const retryLocal = useCallback((afterRetry?: () => void) => {
    if (pipeline && pipe !== null) {
      pipeline.retryWidget(pipe);
      afterRetry?.();
    }
  }, [pipeline, pipe]);

  return { pipeline, pipe, sourceIdsRef, subsReady, instanceId, tooltipOwner, dataContextKey, retryStream, retryLocal };
}

export function useFamilyTooltip(tooltipOwner: string, dataContextKey: string): FamilyTooltipController {
  const controller = useMemo(() => new FamilyTooltipController(tooltipOwner), [tooltipOwner]);
  const prevContextRef = useRef(dataContextKey);

  useEffect(() => {
    if (prevContextRef.current !== dataContextKey) {
      prevContextRef.current = dataContextKey;
      controller.clearPinAndHide();
    }
  }, [controller, dataContextKey]);

  useEffect(() => () => controller.dispose(), [controller]);

  return controller;
}

export interface MountedFamilyCanvas {
  readonly canvas: HTMLCanvasElement;
  readonly extent: HTMLDivElement;
  readonly surface: ReturnType<typeof bitmapSurface>;
  readonly unmount: () => void;
}

export function mountFamilyCanvas(
  bodyNode: HTMLElement,
  pipeline: Pipeline,
  pipe: number,
  surfaceKind: string,
  canvasClassName: string,
  extentClassName: string,
): MountedFamilyCanvas {
  const extent = document.createElement('div');
  const drawn = pipeline.held(pipe, surfaceKind, () => {
    const canvas = document.createElement('canvas');
    canvas.className = canvasClassName;
    canvas.style.display = 'block';
    canvas.setAttribute('aria-hidden', 'true');
    canvas.dataset.surface = surfaceKind;
    canvas.dataset.placement = String(pipe);
    return { canvas, surface: bitmapSurface(canvas) };
  })!;
  extent.className = extentClassName;
  extent.append(drawn.canvas);
  bodyNode.append(extent);
  const stopAlign = alignCanvas(drawn.canvas);
  return {
    canvas: drawn.canvas,
    extent,
    surface: drawn.surface,
    unmount: () => {
      stopAlign();
      extent.remove();
    },
  };
}

export interface FamilyAuditOptions {
  readonly pipeline: Pipeline | null;
  readonly pipe: number | null;
  readonly ownerId: number;
  readonly auditable: boolean;
  readonly maxEntries?: number;
}

export interface FamilyAuditHandle<TSnapshot> {
  readonly capture: (snapshot: TSnapshot, verify: FamilyAuditVerifier<TSnapshot>) => number;
  readonly update: (id: number, patch: (current: TSnapshot) => TSnapshot) => void;
  readonly captureActive: boolean;
}

export function useFamilyAudit<TSnapshot>(options: FamilyAuditOptions): FamilyAuditHandle<TSnapshot> {
  const { pipeline, pipe, ownerId, auditable, maxEntries = 512 } = options;
  const captureActive = useSyncExternalStore(subscribeAuditing, isFamilyAuditing, isFamilyAuditing);
  const storeRef = useRef<AuditSnapshotStore<BoundAuditEntry<TSnapshot>> | null>(null);
  if (!storeRef.current) storeRef.current = new AuditSnapshotStore<BoundAuditEntry<TSnapshot>>(maxEntries);

  useEffect(() => () => {
    storeRef.current?.clear();
  }, []);

  useLayoutEffect(() => {
    if (!perfLog || !pipeline || pipe === null || !auditable) return;
    const store = storeRef.current!;
    pipeline.audit(pipe, ownerId, fields => auditRowsOf(fields.audit));
    const stopReader = perfLog.reader(
      pipe,
      ownerId,
      createBoundAuditReader(store),
      pipeline.generation,
      auditId => store.pin(auditId),
    );
    return () => {
      stopReader();
      pipeline.audit(pipe, ownerId, null);
    };
  }, [pipeline, pipe, ownerId, auditable]);

  const capture = useCallback((snapshot: TSnapshot, verify: FamilyAuditVerifier<TSnapshot>) => {
    return storeRef.current!.create({ snapshot, verify });
  }, []);

  const update = useCallback((id: number, patch: (current: TSnapshot) => TSnapshot) => {
    storeRef.current!.update(id, entry => ({ snapshot: patch(entry.snapshot), verify: entry.verify }));
  }, []);

  return { capture, update, captureActive };
}

export interface AuditWatchController {
  settleIfReady(visible: boolean, mode: string, installedBinding: number | undefined, fields: Readonly<Record<string, unknown>> | undefined): void;
  dispose(): void;
}

export function attachAuditWatch(
  pipeline: Pipeline,
  pipe: number,
  options: {
    readonly isAuditable: () => boolean;
    readonly onCaptureChange: (active: boolean) => void;
    readonly bindNow: () => void;
  },
): AuditWatchController {
  let pendingAck: { targetBinding: number; resolve: () => void } | null = null;
  const settle = () => {
    if (!pendingAck) return;
    const done = pendingAck;
    pendingAck = null;
    done.resolve();
  };

  const stop = subscribeFamilyAuditing(activeAudit => {
    options.onCaptureChange(activeAudit);
    if (!activeAudit) {
      settle();
      options.bindNow();
      return;
    }
    if (!perfLog || !options.isAuditable()) return;
    options.bindNow();
    const targetBinding = pipeline.binding(pipe);
    const currentView = pipeline.view(pipe);
    if (currentView?.mode !== 'picture') return;
    const installedAudit = currentView.fields ? auditRowsOf(currentView.fields.audit) : undefined;
    if ((currentView.installedBinding ?? 0) >= targetBinding && installedAudit !== undefined) return;
    return new Promise<void>(resolve => {
      settle();
      pendingAck = { targetBinding, resolve };
    });
  });

  return {
    settleIfReady(visible, mode, installedBinding, fields) {
      if (!pendingAck) return;
      const installedAudit = visible && fields ? auditRowsOf(fields.audit) : undefined;
      if (!visible || mode !== 'picture' || ((installedBinding ?? 0) >= pendingAck.targetBinding && installedAudit !== undefined)) {
        settle();
      }
    },
    dispose() {
      settle();
      stop();
    },
  };
}

export function useFamilyAnnouncer(prefix: string) {
  const frame = useWidgetFrame();
  const liveRegionRef = useRef<HTMLDivElement>(null);
  const seqRef = useRef(0);
  const lastKeyRef = useRef<string | null>(null);

  useEffect(() => () => {
    if (frame && lastKeyRef.current) frame.clearAnnouncement(lastKeyRef.current);
  }, [frame]);

  const announce = useCallback((message: string) => {
    if (frame) {
      const key = `${prefix}:${++seqRef.current}`;
      lastKeyRef.current = key;
      frame.announce(key, message);
      return;
    }
    if (liveRegionRef.current) liveRegionRef.current.textContent = message;
  }, [frame, prefix]);

  return { announce, hasFrame: !!frame, liveRegionRef };
}

export function useFamilyPauseFlush(
  clock: WidgetRuntime['clock'],
  onPause: () => void,
): void {
  const onPauseRef = useRef(onPause);
  onPauseRef.current = onPause;
  useEffect(() => {
    let wasPlaying = clock.getSnapshot().playing;
    return clock.subscribe(() => {
      const nowPlaying = clock.getSnapshot().playing;
      if (wasPlaying && !nowPlaying) onPauseRef.current();
      wasPlaying = nowPlaying;
    });
  }, [clock]);
}

export function useFamilyHold(
  frameRef: { readonly current: HTMLElement | null },
  held: boolean,
  tooltips: FamilyTooltipController,
): void {
  useLayoutEffect(() => {
    holdElement(frameRef.current, held, STATIC_HOLD);
    if (held) tooltips.hideIfUnpinned();
    return () => holdElement(frameRef.current, false, STATIC_HOLD);
  }, [frameRef, held, tooltips]);
}
