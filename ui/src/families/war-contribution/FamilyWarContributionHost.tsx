import { useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useContextMenu, type MenuItem } from '../../components/ContextMenu';
import { useViewportHeight } from '../../charts/useChartTokens';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { FailureOrigin } from '../../dashboard/follow/page.ts';
import type { WidgetSpec } from '../../dashboard/model';
import { PreparingPage } from '../../dashboard/preparing.ts';
import { useWidgetSkeleton } from '../../dashboard/widgetFrame.tsx';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { holdElement } from '../../draw/bitmapSurface.ts';
import type { WarContributionsReply, WarTimeline } from '../../generated/dashboard';
import type { FamilySourceSubscription } from '../contract.ts';
import {
  bindFamilyPointer,
  handleFamilyKeyDown,
  mountFamilyCanvas,
  renderFamilyBodyState,
  reportFamilyShownHour,
  syncListboxMirror,
  useFamilyAnnouncer,
  useFamilyHold,
  useFamilyPlacement,
  useFamilyTheme,
  useFamilyTooltip,
  type FamilyNavTuple,
  type FamilyPointerHit,
  type FamilyTooltipField,
  type ListboxMirrorItem,
  type MountedFamilyCanvas,
} from '../host/index.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import type { EntityKind } from '../plan.ts';
import type { FamilyWarContributionBinding } from './laneView.ts';
import type { WCMeasureKey, WCShowMode, WCViewMode } from './model.ts';
import './warContributionFamily.css';

function parseWCHit(hitKey: string | null): FamilyPointerHit | null {
  if (!hitKey) return null;
  if (hitKey.startsWith('sort:') || hitKey.startsWith('measure:')) {
    return { key: hitKey, toggle: false };
  }
  if (hitKey.startsWith('row:')) {
    const key = hitKey.slice(4);
    return key ? { key, toggle: false } : null;
  }
  return null;
}

export interface FamilyWarContributionHostProps {
  readonly view: WCViewMode;
  readonly show: WCShowMode;
  readonly war: WarTimeline | null;
  readonly widget: WidgetSpec;
  readonly runtime: WidgetRuntime;
  readonly active: boolean;
  readonly subscriptions: readonly FamilySourceSubscription[];
  readonly staticReply?: WarContributionsReply | null;
  readonly offset: number;
  readonly pin: number | null;
  readonly bounds?: readonly [number, number] | null;
  readonly identitySnapshot?: IdentitySnapshot;
  readonly onIdentityDemands?: (demands: readonly (readonly [EntityKind, string, number])[]) => void;
  readonly contextMenuItems?: (key: string | null, name: string | null, hour: number) => MenuItem[];
  readonly onDiagnostics?: (warnings: string[]) => void;
  readonly extraWarnings?: readonly string[];
  readonly blockedState?: ReactNode;
  readonly loadingLabel?: string;
  readonly onHeaderMeta?: (meta: { warTitle: string; leftLeadTag: string | null; rightLeadTag: string | null; leftLeadName: string; rightLeadName: string }) => void;
}

export default function FamilyWarContributionHost({
  view,
  show,
  war,
  widget,
  runtime,
  active,
  subscriptions,
  staticReply = null,
  offset,
  pin,
  bounds,
  identitySnapshot,
  onIdentityDemands,
  contextMenuItems,
  onDiagnostics,
  extraWarnings,
  blockedState = null,
  loadingLabel,
  onHeaderMeta,
}: FamilyWarContributionHostProps) {
  const prepared = useContext(PreparingPage);
  useWidgetSkeleton('table');
  const viewportHeight = useViewportHeight();
  const contextMenu = useContextMenu('War contribution actions');
  const contextMenuRef = useRef(contextMenu);
  contextMenuRef.current = contextMenu;

  const frameRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const accessibleRef = useRef<HTMLDivElement>(null);

  const [sortKey, setSortKey] = useState<WCMeasureKey>('participation');
  const [selectedMeasure, setSelectedMeasure] = useState<WCMeasureKey>('equipmentDamage');
  const viewportRef = useRef({ width: 0, height: 0, scrollTop: 0, scrollLeftLeft: 0, scrollLeftRight: 0, maxScrollLeft: 0 });
  const focusedKeyRef = useRef<string | null>(null);
  const focusInsideRef = useRef(false);
  const focusAnchorRef = useRef({ x: 0, y: 0 });
  const inspectedKeyRef = useRef<string | null>(null);
  const lastPointerRef = useRef<{ x: number; y: number } | null>(null);
  const dimRef = useRef(false);
  const lastMirrorKeyRef = useRef('');
  const lastExtentHeightRef = useRef(-1);
  const navRef = useRef<readonly FamilyNavTuple[]>([]);
  const lastHourRef = useRef(staticReply?.hour ?? 0);

  const [meta, setMeta] = useState<{
    mode: string;
    reason: string | null;
    failureOrigin: FailureOrigin;
    empty: string | null;
    emptyKind: 'empty' | 'unavailable';
    warnings: readonly string[];
  }>({
    mode: staticReply ? 'picture' : 'reading',
    reason: null,
    failureOrigin: 'local',
    empty: null,
    emptyKind: 'empty',
    warnings: [],
  });
  const metaSigRef = useRef('');
  const headerSigRef = useRef('');

  const { theme } = useFamilyTheme();
  const { announce, hasFrame, liveRegionRef } = useFamilyAnnouncer('war-contribution');

  const enabled = active && !prepared && !blockedState && (subscriptions.length > 0 || staticReply !== null);
  const placement = useFamilyPlacement({
    family: 'war-contribution',
    declarationKind: 'war-contribution',
    widgetId: widget.id,
    widgetKind: widget.kind,
    subject: 'war contributions',
    widgetTrackIds: widget.trackIds,
    runtime,
    enabled,
    auditable: false,
    subscriptions,
    offset,
    pin: pin ?? (subscriptions.length === 0 && staticReply ? staticReply.hour : null),
    bounds,
    frameRef,
    surfaceLabel: 'war-contribution',
  });
  const { pipeline, pipe, sourceIdsRef, subsReady, tooltipOwner, dataContextKey, retryStream, retryLocal, instanceId } = placement;

  const tooltips = useFamilyTooltip(tooltipOwner, dataContextKey);
  useFamilyHold(frameRef, false, tooltips);

  const reportShown = useShownHour();
  const reportShownRef = useRef(reportShown);
  reportShownRef.current = reportShown;

  const surfaceRef = useRef<MountedFamilyCanvas | null>(null);
  const latestRef = useRef({
    view,
    show,
    sortKey,
    selectedMeasure,
    war,
    staticReply,
    extraWarnings,
    theme,
    identitySnapshot,
    onIdentityDemands,
    contextMenuItems,
    viewportHeight,
    onHeaderMeta,
  });
  latestRef.current = {
    view,
    show,
    sortKey,
    selectedMeasure,
    war,
    staticReply,
    extraWarnings,
    theme,
    identitySnapshot,
    onIdentityDemands,
    contextMenuItems,
    viewportHeight,
    onHeaderMeta,
  };

  const bind = useCallback(() => {
    const mounted = surfaceRef.current;
    if (!mounted || !pipeline || pipe === null) return;
    const { width, height, scrollTop, scrollLeftLeft, scrollLeftRight } = viewportRef.current;
    const w = Math.max(1, width || 480);
    const h = Math.max(1, height || 240);
    const cur = latestRef.current;
    const bindingModel: FamilyWarContributionBinding = {
      family: 'war-contribution',
      view: cur.view,
      show: cur.show,
      sortKey: cur.sortKey,
      selectedMeasure: cur.selectedMeasure,
      war: cur.war,
      sources: sourceIdsRef.current,
      ...(cur.staticReply ? { staticReply: cur.staticReply } : {}),
      ...(cur.extraWarnings ? { extraWarnings: cur.extraWarnings } : {}),
      width: w,
      height: h,
      scrollTop,
      scrollLeftLeft,
      scrollLeftRight,
      theme: cur.theme,
      ...(cur.identitySnapshot ? { identitySnapshot: cur.identitySnapshot } : {}),
      focusedKey: focusInsideRef.current ? focusedKeyRef.current : null,
    };
    mounted.canvas.style.width = `${w}px`;
    mounted.canvas.style.height = `${h}px`;
    pipeline.present(pipe, 'war-contribution');
    const dated = Object.keys(sourceIdsRef.current).length === 0 && cur.staticReply
      ? { h: cur.staticReply.hour, c: cur.staticReply.hour }
      : undefined;
    pipeline.bind(
      pipe,
      [mounted.surface],
      {
        surfaces: [{ width: w, height: h, dpr: typeof devicePixelRatio === 'number' ? devicePixelRatio : 1 }],
        ...(dated ? { dated } : {}),
      },
      {},
      bindingModel,
    );
  }, [pipeline, pipe, sourceIdsRef]);

  const bindRef = useRef(bind);
  bindRef.current = bind;

  const optionIdPrefix = useMemo(() => `wc-${instanceId}`, [instanceId]);

  useLayoutEffect(() => {
    const node = bodyRef.current;
    if (!node || !pipeline || pipe === null) return;
    lastExtentHeightRef.current = -1;
    lastMirrorKeyRef.current = '';

    const mounted = mountFamilyCanvas(node, pipeline, pipe, 'war-contribution', 'wc-canvas', 'wc-extent');
    surfaceRef.current = mounted;

    const install = () => {
      const shown = pipeline.view(pipe);
      const frameNode = frameRef.current;
      dimRef.current = !!shown?.dim;
      holdElement(frameNode, !!shown?.dim);
      const visible = shown?.mode === 'picture';

      const mode = shown?.mode ?? 'reading';
      const reason = shown?.reason ?? null;
      const failureOrigin: FailureOrigin = shown?.failureOrigin ?? 'local';
      const shownHour = reportFamilyShownHour(reportShownRef.current, visible, shown);
      if (shownHour !== null) lastHourRef.current = shownHour;

      const rawMeta = shown?.fields?.meta as {
        empty?: string | null;
        emptyKind?: 'empty' | 'unavailable';
        warnings?: readonly string[];
        warTitle?: string;
        leftLeadTag?: string | null;
        rightLeadTag?: string | null;
        leftLeadName?: string;
        rightLeadName?: string;
        contentHeight?: number;
        maxScrollLeft?: number;
      } | undefined;

      if (rawMeta) {
        viewportRef.current.maxScrollLeft = rawMeta.maxScrollLeft ?? 0;
        const hSig = JSON.stringify([rawMeta.warTitle, rawMeta.leftLeadTag, rawMeta.rightLeadTag, rawMeta.leftLeadName, rawMeta.rightLeadName]);
        if (hSig !== headerSigRef.current && latestRef.current.onHeaderMeta) {
          headerSigRef.current = hSig;
          latestRef.current.onHeaderMeta({
            warTitle: rawMeta.warTitle ?? '',
            leftLeadTag: rawMeta.leftLeadTag ?? null,
            rightLeadTag: rawMeta.rightLeadTag ?? null,
            leftLeadName: rawMeta.leftLeadName ?? '',
            rightLeadName: rawMeta.rightLeadName ?? '',
          });
        }
      }

      const sig = JSON.stringify([mode, reason, failureOrigin, rawMeta?.empty ?? null, rawMeta?.emptyKind ?? 'empty', rawMeta?.warnings ?? []]);
      if (sig !== metaSigRef.current) {
        metaSigRef.current = sig;
        setMeta({
          mode,
          reason,
          failureOrigin,
          empty: rawMeta?.empty ?? null,
          emptyKind: rawMeta?.emptyKind ?? 'empty',
          warnings: rawMeta?.warnings ?? [],
        });
      }

      const demandsField = shown?.fields?.identityDemands as readonly (readonly [EntityKind, string, number])[] | undefined;
      if (demandsField && latestRef.current.onIdentityDemands) {
        latestRef.current.onIdentityDemands(demandsField);
      }

      if (!visible || !shown?.fields) {
        syncListboxMirror(accessibleRef.current, null, { optionIdPrefix, total: 0, focusedKey: null, lastMirrorKeyRef });
        tooltips.hideIfUnpinned();
        return;
      }

      const extH = Math.max(viewportRef.current.height, rawMeta?.contentHeight ?? viewportRef.current.height);
      if (lastExtentHeightRef.current !== extH) {
        lastExtentHeightRef.current = extH;
        mounted.extent.style.height = `${extH}px`;
      }

      const navField = shown.fields.nav as readonly FamilyNavTuple[] | undefined;
      if (navField) navRef.current = navField;
      const mirror = shown.fields.mirror as readonly ListboxMirrorItem[] | undefined;
      if (mirror) {
        syncListboxMirror(accessibleRef.current, mirror, {
          optionIdPrefix,
          total: mirror.length,
          focusedKey: focusInsideRef.current ? focusedKeyRef.current : null,
          lastMirrorKeyRef,
        });
      }

      const activeInspected = inspectedKeyRef.current;
      const activeFocusInside = focusInsideRef.current;
      if (!activeInspected && !activeFocusInside) {
        tooltips.hideIfUnpinned();
        return;
      }

      const hoverTipField = activeInspected ? (shown.fields.tooltip as FamilyTooltipField | undefined) : undefined;
      const focusTipField = activeFocusInside ? (shown.fields.focusTooltip as FamilyTooltipField | undefined) : undefined;
      tooltips.syncFromInstall({
        dim: !!shown.dim,
        held: false,
        inspectedKey: activeInspected,
        pointerPos: lastPointerRef.current,
        hoverTip: hoverTipField?.text ?? null,
        focusInside: activeFocusInside,
        focusedKey: focusedKeyRef.current,
        focusAnchor: focusAnchorRef.current,
        focusTip: focusTipField?.text ?? null,
        viewportHeight: latestRef.current.viewportHeight,
      });
    };

    install();
    const stopInstall = pipeline.onInstall(pipe, install);

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
      accessibleRef,
      parseHit: parseWCHit,
      inspectToken: key => key.startsWith('measure:') ? key : `row:${key}`,
      isBlocked: () => dimRef.current,
      getViewportHeight: () => latestRef.current.viewportHeight,
      onActivate: key => {
        if (key.startsWith('sort:')) {
          setSortKey(key.slice(5) as WCMeasureKey);
          announce('Sorted war contribution table.');
          return;
        }
        if (key.startsWith('measure:')) {
          setSelectedMeasure(key.slice(8) as WCMeasureKey);
          announce('Selected war contribution measure.');
        }
      },
      onContextMenu: (key, clientX, clientY) => {
        const builder = latestRef.current.contextMenuItems;
        if (!builder) return false;
        const entry = key ? navRef.current.find(item => item[0] === key) : undefined;
        const items = builder(key, key ? (entry?.[1] ?? key) : null, lastHourRef.current);
        if (!items.length) return false;
        contextMenuRef.current.open(clientX, clientY, items);
        return true;
      },
    });

    const onWheel = (event: WheelEvent) => {
      if (latestRef.current.view !== 'sides') return;
      const deltaX = Math.abs(event.deltaX) > Math.abs(event.deltaY)
        ? event.deltaX
        : (event.shiftKey ? event.deltaY : 0);
      if (!deltaX || viewportRef.current.maxScrollLeft <= 0) return;
      event.preventDefault();
      const box = node.getBoundingClientRect();
      const onRight = event.clientX - box.left > box.width / 2;
      const max = viewportRef.current.maxScrollLeft;
      if (onRight) {
        viewportRef.current.scrollLeftRight = Math.max(0, Math.min(max, viewportRef.current.scrollLeftRight + deltaX));
      } else {
        viewportRef.current.scrollLeftLeft = Math.max(0, Math.min(max, viewportRef.current.scrollLeftLeft + deltaX));
      }
      bindRef.current();
    };

    const onScroll = () => {
      viewportRef.current.scrollTop = node.scrollTop;
      bindRef.current();
    };

    const resizeObserver = new ResizeObserver(entries => {
      const rect = entries[0]?.contentRect;
      const w = Math.round(rect ? rect.width : node.clientWidth);
      const h = Math.round(rect ? rect.height : node.clientHeight);
      if (w === viewportRef.current.width && h === viewportRef.current.height) return;
      viewportRef.current.width = w;
      viewportRef.current.height = h;
      bindRef.current();
    });
    resizeObserver.observe(node);
    viewportRef.current.width = node.clientWidth;
    viewportRef.current.height = node.clientHeight;
    node.addEventListener('scroll', onScroll, { passive: true });
    node.addEventListener('wheel', onWheel, { passive: false });
    bindRef.current();

    return () => {
      stopInstall();
      stopPointer();
      resizeObserver.disconnect();
      node.removeEventListener('scroll', onScroll);
      node.removeEventListener('wheel', onWheel);
      mounted.unmount();
      surfaceRef.current = null;
      holdElement(frameRef.current, false);
    };
  }, [pipeline, pipe, tooltips, optionIdPrefix, announce]);

  useLayoutEffect(() => {
    if (pipe !== null && enabled) bind();
  }, [pipe, enabled, subsReady, view, show, sortKey, selectedMeasure, war, staticReply, theme, identitySnapshot, bind]);

  const warningsKey = JSON.stringify([...meta.warnings, ...(extraWarnings ?? [])]);
  useEffect(() => {
    if (!onDiagnostics) return;
    onDiagnostics(JSON.parse(warningsKey) as string[]);
    return () => onDiagnostics([]);
  }, [warningsKey, onDiagnostics]);

  const onKeyDown = useCallback((event: React.KeyboardEvent<HTMLDivElement>) => {
    handleFamilyKeyDown(event, {
      nav: navRef.current,
      focusedKey: focusedKeyRef.current,
      pageStep: 8,
      blocked: dimRef.current,
      isPinnedTooltip: () => tooltips.isPinned(),
      onFocusKey: key => {
        focusedKeyRef.current = key;
        bindRef.current();
      },
      onActivate: () => {},
      onTogglePinTooltip: key => {
        if (!key || !pipeline || pipe === null) return;
        const tipField = pipeline.view(pipe)?.fields?.focusTooltip as FamilyTooltipField | undefined;
        tooltips.pinFocused(tipField?.text ?? null, lastHourRef.current, focusAnchorRef.current, viewportHeight, announce);
      },
      onUnpinTooltip: () => tooltips.unpin(announce),
    });
  }, [tooltips, pipeline, pipe, viewportHeight, announce]);

  if (blockedState) {
    return <div ref={frameRef} className="wc-frame">{blockedState}</div>;
  }

  const bodyState = renderFamilyBodyState({
    mode: meta.mode,
    reason: meta.reason,
    failureOrigin: meta.failureOrigin,
    empty: meta.empty,
    emptyKind: meta.emptyKind,
    subject: 'war contributions',
    loadingLabel,
    pipeline,
    pipe,
    onRetryStream: retryStream,
    onRetryLocal: retryLocal,
  });

  return (
    <div ref={frameRef} className="wc-frame">
      <div className="wc-body-wrap">
        <div
          ref={bodyRef}
          className="wc-body"
          tabIndex={0}
          onFocus={() => {
            focusInsideRef.current = true;
            if (!focusedKeyRef.current && navRef.current.length) focusedKeyRef.current = navRef.current[0]![0];
            bindRef.current();
          }}
          onBlur={() => {
            focusInsideRef.current = false;
            if (!inspectedKeyRef.current) tooltips.hideIfUnpinned();
            bindRef.current();
          }}
          onKeyDown={onKeyDown}
        >
          <div ref={accessibleRef} className="wc-accessibility" role="listbox" aria-label="War contribution" />
        </div>
        {bodyState && <div className="wc-state-overlay">{bodyState}</div>}
      </div>
      {!hasFrame && <div ref={liveRegionRef} className="wc-live-region" aria-live="polite" />}
      {contextMenu.menu}
    </div>
  );
}
