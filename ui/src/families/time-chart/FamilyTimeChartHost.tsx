import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { useContextMenu } from '../../components/ContextMenu.tsx';
import { jumpItem } from '../../charts/jump.ts';
import { useViewportHeight } from '../../charts/useChartTokens.ts';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { FailureOrigin } from '../../dashboard/follow/page.ts';
import { holdElement } from '../../draw/bitmapSurface.ts';
import type { EntityKind, FamilySourceSubscription } from '../contract.ts';
import { renderFamilyBodyState } from '../host/FamilyBodyState.tsx';
import { bindFamilyPointer } from '../host/pointer.ts';
import {
  mountFamilyCanvas,
  reportFamilyShownHour,
  useFamilyPlacement,
  useFamilyTooltip,
  type MountedFamilyCanvas,
} from '../host/surface.ts';
import type { FamilyTooltipField } from '../host/tooltips.ts';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { parseChartInspect } from './hitTest.ts';
import type { TimeChartBinding } from './declaration.ts';

interface ChartHostMeta {
  readonly mode: string;
  readonly reason: string | null;
  readonly failureOrigin: FailureOrigin;
  readonly empty: string | null;
  readonly emptyDetail: string | null;
  readonly emptyKind: 'empty' | 'unavailable';
  readonly note: string | null;
  readonly warnings: readonly string[];
}

export interface FamilyTimeChartHostProps<TOptions> {
  readonly declarationKind: string;
  readonly label: string;
  readonly subject: string;
  readonly widgetId: string;
  readonly widgetKind: string;
  readonly trackId: string;
  readonly runtime: WidgetRuntime;
  readonly active: boolean;
  readonly subscriptions: readonly FamilySourceSubscription[];
  readonly offset: number;
  readonly pin: number | null;
  readonly bounds: readonly [number, number] | null;
  readonly buildBinding: (width: number, height: number, sourceIds: Readonly<Record<string, number>>) => TimeChartBinding<TOptions> | null;
  readonly bindingVersion: unknown;
  readonly blockedState?: ReactNode;
  readonly extraWarnings?: readonly string[];
  readonly onViewportWidth?: (width: number) => void;
  readonly onViewportSize?: (width: number, height: number) => void;
  readonly onHoverHour?: (hour: number | null) => void;
  readonly onIdentityDemands?: (demands: readonly (readonly [EntityKind, string, number])[]) => void;
  readonly onArtDemands?: (demands: readonly { readonly key: string; readonly frame: number }[]) => void;
  readonly onActivate?: (inspectKey: string) => void;
  readonly onDiagnostics?: (warnings: string[]) => void;
  readonly loadingLabel?: string;
}

const NO_WARNINGS: string[] = [];

export default function FamilyTimeChartHost<TOptions>({
  declarationKind,
  label,
  subject,
  widgetId,
  widgetKind,
  trackId,
  runtime,
  active,
  subscriptions,
  offset,
  pin,
  bounds,
  buildBinding,
  bindingVersion,
  blockedState,
  extraWarnings,
  onViewportWidth,
  onViewportSize,
  onHoverHour,
  onIdentityDemands,
  onArtDemands,
  onActivate,
  onDiagnostics,
  loadingLabel,
}: FamilyTimeChartHostProps<TOptions>) {
  const viewportHeight = useViewportHeight();
  const contextMenu = useContextMenu(`${label} actions`);
  const frameRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const surfaceRef = useRef<MountedFamilyCanvas | null>(null);
  const sizeRef = useRef({ width: 0, height: 0 });
  const inspectedKeyRef = useRef<string | null>(null);
  const lastHoverHourRef = useRef<number | null>(null);
  const lastPointerRef = useRef<{ x: number; y: number } | null>(null);
  const focusInsideRef = useRef(false);
  const focusedKeyRef = useRef<string | null>(null);
  const focusAnchorRef = useRef({ x: 0, y: 0 });
  const dimRef = useRef(false);
  const reportShown = useShownHour();
  const reportShownRef = useRef(reportShown);
  reportShownRef.current = reportShown;

  const [meta, setMeta] = useState<ChartHostMeta>({
    mode: 'reading',
    reason: null,
    failureOrigin: 'local',
    empty: null,
    emptyDetail: null,
    emptyKind: 'empty',
    note: null,
    warnings: NO_WARNINGS,
  });
  const metaSigRef = useRef('');

  const enabled = active && bounds !== null && !blockedState;
  const placement = useFamilyPlacement({
    family: 'time-chart',
    declarationKind,
    widgetId,
    widgetKind,
    runtime,
    enabled,
    auditable: false,
    subscriptions,
    offset,
    pin,
    bounds,
    frameRef,
  });

  const tooltips = useFamilyTooltip(placement.tooltipOwner, placement.dataContextKey);

  const latestRef = useRef({
    buildBinding,
    onViewportWidth,
    onViewportSize,
    onHoverHour,
    onIdentityDemands,
    onArtDemands,
    onActivate,
    viewportHeight,
    contextMenu,
    runtime,
    trackId,
    offset,
    bounds,
  });
  latestRef.current = {
    buildBinding,
    onViewportWidth,
    onViewportSize,
    onHoverHour,
    onIdentityDemands,
    onArtDemands,
    onActivate,
    viewportHeight,
    contextMenu,
    runtime,
    trackId,
    offset,
    bounds,
  };

  useEffect(() => {
    if (!onDiagnostics) return;
    const all = [...(extraWarnings ?? []), ...meta.warnings];
    onDiagnostics(all.length ? all : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [meta.warnings, extraWarnings, onDiagnostics]);

  const bindNow = useCallback(() => {
    const { pipeline, pipe, sourceIdsRef } = placement;
    const mounted = surfaceRef.current;
    if (!pipeline || pipe === null || !mounted) return;
    const { width, height } = sizeRef.current;
    if (width <= 0 || height <= 0) return;
    const binding = latestRef.current.buildBinding(width, height, sourceIdsRef.current);
    if (!binding) return;
    const dpr = typeof devicePixelRatio === 'number' ? devicePixelRatio : 1;
    mounted.canvas.style.width = `${width}px`;
    mounted.canvas.style.height = `${height}px`;
    pipeline.present(pipe, 'time-chart');
    const dated = Object.keys(sourceIdsRef.current).length === 0 && latestRef.current.bounds
      ? { h: latestRef.current.bounds[0], c: latestRef.current.bounds[1] }
      : undefined;
    pipeline.bind(
      pipe,
      [mounted.surface],
      { surfaces: [{ width, height, dpr }], ...(dated ? { dated } : {}) },
      {},
      binding,
    );
  }, [placement]);

  const bindRef = useRef(bindNow);
  bindRef.current = bindNow;

  useLayoutEffect(() => {
    const node = bodyRef.current;
    const { pipeline, pipe } = placement;
    if (!node || !pipeline || pipe === null) return;

    const mounted = mountFamilyCanvas(node, pipeline, pipe, 'time-chart', 'tc-family-canvas', 'tc-family-extent');
    mounted.extent.style.width = '100%';
    mounted.extent.style.height = '100%';
    mounted.canvas.style.display = 'block';
    surfaceRef.current = mounted;

    const publishHoverHour = (key: string | null) => {
      const parsed = parseChartInspect(key);
      const next = parsed && 'hour' in parsed ? parsed.hour : null;
      if (next !== lastHoverHourRef.current) {
        lastHoverHourRef.current = next;
        latestRef.current.onHoverHour?.(next);
      }
    };

    const install = () => {
      const shown = pipeline.view(pipe);
      dimRef.current = !!shown?.dim;
      holdElement(frameRef.current, !!shown?.dim);
      const visible = shown?.mode === 'picture';
      reportFamilyShownHour(reportShownRef.current, visible, shown);

      const rawMeta = shown?.fields?.meta as {
        empty?: string | null;
        emptyDetail?: string | null;
        emptyKind?: 'empty' | 'unavailable';
        note?: string | null;
        warnings?: readonly string[];
      } | undefined;
      const nextMeta: ChartHostMeta = {
        mode: shown?.mode ?? 'reading',
        reason: shown?.reason ?? null,
        failureOrigin: shown?.failureOrigin ?? 'local',
        empty: rawMeta?.empty ?? null,
        emptyDetail: rawMeta?.emptyDetail ?? null,
        emptyKind: rawMeta?.emptyKind ?? 'empty',
        note: rawMeta?.note ?? null,
        warnings: rawMeta?.warnings ?? NO_WARNINGS,
      };
      const sig = JSON.stringify([nextMeta.mode, nextMeta.reason, nextMeta.empty, nextMeta.emptyDetail, nextMeta.note, nextMeta.warnings]);
      if (sig !== metaSigRef.current) {
        metaSigRef.current = sig;
        setMeta(nextMeta);
      }

      const idDemands = shown?.fields?.identityDemands as readonly (readonly [EntityKind, string, number])[] | undefined;
      if (idDemands && latestRef.current.onIdentityDemands) {
        latestRef.current.onIdentityDemands(idDemands);
      }
      const artDemands = shown?.fields?.artDemands as readonly { readonly key: string; readonly frame: number }[] | undefined;
      if (artDemands && latestRef.current.onArtDemands) {
        latestRef.current.onArtDemands(artDemands);
      }

      if (!visible || !shown?.fields) {
        tooltips.hideIfUnpinned();
        return;
      }

      const hoverField = shown.fields.tooltip as FamilyTooltipField | undefined;
      tooltips.syncFromInstall({
        dim: dimRef.current,
        held: false,
        inspectedKey: inspectedKeyRef.current,
        pointerPos: lastPointerRef.current,
        hoverTip: hoverField?.text ?? null,
        focusInside: focusInsideRef.current,
        focusedKey: focusedKeyRef.current,
        focusAnchor: focusAnchorRef.current,
        focusTip: null,
        viewportHeight: latestRef.current.viewportHeight,
      });
    };

    const stopInstall = pipeline.onInstall(pipe, install);
    install();

    const stopPointer = bindFamilyPointer({
      bodyNode: node,
      menuHost: frameRef.current ?? node,
      pipeline,
      pipe,
      tooltips,
      inspectedKeyRef,
      lastPointerRef,
      focusInsideRef,
      focusedKeyRef,
      focusAnchorRef,
      accessibleRef: frameRef,
      isBlocked: () => dimRef.current,
      getViewportHeight: () => latestRef.current.viewportHeight,
      parseHit: hitKey => {
        publishHoverHour(hitKey);
        return hitKey ? { key: hitKey, toggle: false } : null;
      },
      inspectToken: key => key,
      onActivate: key => {
        latestRef.current.onActivate?.(key);
      },
      onContextMenu: (key, clientX, clientY) => {
        const parsed = parseChartInspect(key);
        const hour = parsed && 'hour' in parsed ? parsed.hour : null;
        if (hour === null) return false;
        latestRef.current.contextMenu.open(clientX, clientY, [
          jumpItem(latestRef.current.runtime, latestRef.current.trackId, hour),
        ]);
        return true;
      },
    });

    const measure = () => {
      const rect = node.getBoundingClientRect();
      latestRef.current.onViewportSize?.(rect.width, rect.height);
      const w = Math.max(0, Math.floor(rect.width));
      const h = Math.max(0, Math.floor(rect.height));
      if (w !== sizeRef.current.width || h !== sizeRef.current.height) {
        sizeRef.current = { width: w, height: h };
        if (w > 0) latestRef.current.onViewportWidth?.(w);
        bindRef.current();
      }
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);

    return () => {
      observer.disconnect();
      stopPointer();
      stopInstall();
      publishHoverHour(null);
      tooltips.clearPinAndHide();
      surfaceRef.current = null;
      mounted.unmount();
    };
  }, [placement.pipeline, placement.pipe, tooltips]);

  useLayoutEffect(() => {
    bindNow();
  }, [bindNow, placement.subsReady, bindingVersion]);

  const bodyState = blockedState ?? renderFamilyBodyState({
    mode: meta.mode,
    reason: meta.reason,
    failureOrigin: meta.failureOrigin,
    empty: meta.empty,
    emptyDetail: meta.emptyDetail,
    emptyKind: meta.emptyKind,
    subject,
    loadingLabel: loadingLabel ?? `Loading ${subject}…`,
    pipeline: placement.pipeline,
    pipe: placement.pipe,
    onRetryStream: placement.retryStream,
    onRetryLocal: () => placement.retryLocal(),
  });

  return (
    <div
      ref={frameRef}
      className="tc-family-frame"
      style={{ position: 'relative', width: '100%', height: '100%', display: 'flex', flexDirection: 'column', minHeight: 0 }}
    >
      {meta.note && !blockedState && (
        <div className="tc-family-note" style={{ fontSize: 12, color: 'var(--muted)', padding: '2px 12px 0' }}>
          {meta.note}
        </div>
      )}
      <div
        ref={bodyRef}
        className="tc-family-body"
        style={{ position: 'relative', flex: '1 1 auto', minHeight: 0, width: '100%', height: '100%', overflow: 'hidden' }}
      />
      {bodyState && (
        <div style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', justifyContent: 'center', background: 'var(--surface)' }}>
          {bodyState}
        </div>
      )}
    </div>
  );
}
