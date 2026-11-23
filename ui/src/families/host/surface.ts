import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { canonical } from '../../dashboard/follow/canonical.ts';
import { restartPipeline, type Pipeline } from '../../dashboard/follow/pipeline.ts';
import { useElement, useFollowedWidget, usePipeline, useSurfaceLabel } from '../../dashboard/follow/useFollowed.ts';
import { useWidgetKey } from '../../dashboard/follow/useFollow.ts';
import { useWidgetFrame } from '../../dashboard/widgetFrame.tsx';
import { bitmapSurface, holdElement } from '../../draw/bitmapSurface.ts';
import { perfLog, type ExactPicture } from '../../shell/perf.ts';
import { AuditSnapshotStore, auditRowsOf, subscribeFamilyAuditing, useFamilyAuditing } from '../audit.ts';
import type { FamilySourceSubscription } from '../contract.ts';
import type { FamilyTooltipController } from './tooltips.ts';
export { reportFamilyShownHour } from './shownHour.ts';

const STATIC_HOLD = {};
let nextInstanceSeq = 1;

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
  readonly retryStream: () => void;
  readonly retryLocal: (afterRetry?: () => void) => void;
}

export function useFamilyPlacement(options: FamilyPlacementOptions): FamilyPlacementHandle {
  const {
    family,
    declarationKind,
    widgetId,
    widgetKind,
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
    bounds,
    widget: widgetKey,
    key: `${widgetKind}|${declarationKind}`,
  });

  const frameElement = useElement(frameRef);
  useSurfaceLabel(frameElement, enabled && auditable, pipe, surfaceLabel);

  const subsKey = useMemo(
    () => JSON.stringify(subscriptions.map(s => [s.name, s.source.id, s.source.revision, s.branch, s.offset, s.channel, s.product ?? '', canonical(s.query)])),
    [subscriptions],
  );
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

  return { pipeline, pipe, sourceIdsRef, subsReady, instanceId, tooltipOwner, retryStream, retryLocal };
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
    canvas.setAttribute('aria-hidden', 'true');
    canvas.dataset.surface = surfaceKind;
    canvas.dataset.placement = String(pipe);
    return { canvas, surface: bitmapSurface(canvas) };
  })!;
  extent.className = extentClassName;
  extent.append(drawn.canvas);
  bodyNode.append(extent);
  return {
    canvas: drawn.canvas,
    extent,
    surface: drawn.surface,
    unmount: () => {
      extent.remove();
    },
  };
}

export interface FamilyAuditOptions<TSnapshot> {
  readonly pipeline: Pipeline | null;
  readonly pipe: number | null;
  readonly ownerId: number;
  readonly auditable: boolean;
  readonly maxEntries?: number;
  readonly exact: (hour: number, snap: TSnapshot, auditId: number | undefined) => ReturnType<ExactPicture>;
}

export interface FamilyAuditHandle<TSnapshot> {
  readonly store: AuditSnapshotStore<TSnapshot>;
  readonly captureActive: boolean;
}

export function useFamilyAudit<TSnapshot>(options: FamilyAuditOptions<TSnapshot>): FamilyAuditHandle<TSnapshot> {
  const { pipeline, pipe, ownerId, auditable, maxEntries = 512, exact } = options;
  const captureActive = useFamilyAuditing();
  const storeRef = useRef<AuditSnapshotStore<TSnapshot> | null>(null);
  if (!storeRef.current) storeRef.current = new AuditSnapshotStore<TSnapshot>(maxEntries);
  const exactRef = useRef(exact);
  exactRef.current = exact;

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
      async (hour, auditId) => {
        const snap = store.require(auditId);
        return exactRef.current(hour, snap, auditId);
      },
      pipeline.generation,
      auditId => store.pin(auditId),
    );
    return () => {
      stopReader();
      pipeline.audit(pipe, ownerId, null);
    };
  }, [pipeline, pipe, ownerId, auditable]);

  return { store: storeRef.current, captureActive };
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
