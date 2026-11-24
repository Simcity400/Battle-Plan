import { useCallback, useContext, useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { useContextMenu, type MenuItem } from '../../components/ContextMenu';
import { useViewportHeight } from '../../charts/useChartTokens';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { FailureOrigin } from '../../dashboard/follow/page.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { WidgetSpec } from '../../dashboard/model';
import { PreparingPage } from '../../dashboard/preparing.ts';
import { useWidgetSkeleton, WidgetCaption } from '../../dashboard/widgetFrame.tsx';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { holdElement } from '../../draw/bitmapSurface.ts';
import { perfLog } from '../../shell/perf.ts';
import type { BoundedReadView, EntityKind, FamilySourceSubscription } from '../contract.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import {
  attachAuditWatch,
  bindFamilyPointer,
  handleFamilyKeyDown,
  mountFamilyCanvas,
  renderFamilyBodyState,
  reportFamilyShownHour,
  scrollRangeIntoView,
  syncListboxMirror,
  useFamilyAnnouncer,
  useFamilyAudit,
  useFamilyHold,
  useFamilyPauseFlush,
  useFamilyPlacement,
  useFamilyTheme,
  useFamilyTooltip,
  type FamilyNavTuple,
  type FamilyTooltipField,
  type ListboxMirrorItem,
  type MountedFamilyCanvas,
} from '../host/index.ts';
import type { CountryLooksMap } from '../looks.ts';
import type { FamilyMeterBinding, MeterArtResolutions } from './laneView.ts';
import { deriveMeterAudit, type MeterProjection } from './model.ts';
import './meterFamily.css';

const METER_AUDIT = -2;

interface AuditSnapshot<TOptions> {
  readonly declaration: MeterProjection<TOptions>;
  readonly options: TOptions;
  readonly exactReadView: (hour: number) => Promise<BoundedReadView | null>;
}

async function verifyMeterAuditSnapshot<TOptions>(hour: number, snap: AuditSnapshot<TOptions>) {
  const exact = await snap.exactReadView(hour);
  if (!exact) return null;
  const model = snap.declaration.project(exact, snap.options);
  const rows = deriveMeterAudit(model);
  return { values: new Map(rows.map(row => [row.key, row.value ?? ''])), ordered: true };
}

export interface FamilyMeterHostProps<TOptions> {
  readonly declaration: MeterProjection<TOptions>;
  readonly options: TOptions;
  readonly label: string;
  readonly subject: string;
  readonly widget: WidgetSpec;
  readonly runtime: WidgetRuntime;
  readonly active: boolean;
  readonly subscriptions: readonly FamilySourceSubscription[];
  readonly allowEmptyFollow?: boolean;
  readonly owner?: string;
  readonly staticSources?: {
    readonly panels?: Readonly<Record<string, unknown>>;
    readonly answers?: Readonly<Record<string, FollowedAnswer>>;
    readonly hour: number;
    readonly provenInterval?: readonly [number, number] | null;
  } | null;
  readonly exactReadView?: (hour: number) => Promise<BoundedReadView | null>;
  readonly offset: number;
  readonly pin: number | null;
  readonly bounds?: readonly [number, number] | null;
  readonly countryColours?: ReadonlyMap<string, string>;
  readonly countryLooks?: CountryLooksMap;
  readonly identitySnapshot?: IdentitySnapshot | null;
  readonly onIdentityDemands?: (demands: readonly (readonly [EntityKind, string, number])[]) => void;
  readonly onViewportSize?: (width: number, height: number) => void;
  readonly artResolutions?: MeterArtResolutions;
  readonly selectedKey?: string | null;
  readonly onActivate?: (key: string, meta?: unknown, hour?: number) => void;
  readonly contextMenuItems?: (key: string | null, name: string | null, hour: number) => MenuItem[];
  readonly headerSlot?: HTMLElement | null;
  readonly onDiagnostics?: (warnings: string[]) => void;
  readonly extraWarnings?: readonly string[];
  readonly blockedState?: ReactNode;
  readonly held?: boolean;
  readonly loadingLabel?: string;
  readonly className?: string;
}

interface MeterMetaState {
  mode: string;
  reason: string | null;
  failureOrigin: FailureOrigin;
  empty: string | null;
  emptyDetail: string | null;
  emptyKind: 'empty' | 'unavailable';
  summary: string | null;
  notes: readonly string[];
  warnings: readonly string[];
}

interface MeterExtentState {
  readonly width: number;
  readonly height: number;
  readonly bands: Readonly<Record<string, readonly [number, number]>>;
}

export default function FamilyMeterHost<TOptions>({
  declaration,
  options,
  label,
  subject,
  widget,
  runtime,
  active,
  subscriptions,
  allowEmptyFollow = false,
  owner,
  staticSources = null,
  exactReadView,
  offset,
  pin,
  bounds,
  countryColours,
  countryLooks,
  identitySnapshot = null,
  onIdentityDemands,
  onViewportSize,
  artResolutions,
  selectedKey = null,
  onActivate,
  contextMenuItems,
  headerSlot = null,
  onDiagnostics,
  extraWarnings,
  blockedState = null,
  held = false,
  loadingLabel,
  className,
}: FamilyMeterHostProps<TOptions>) {
  const prepared = useContext(PreparingPage);
  useWidgetSkeleton('list');
  const listboxPrefix = useId();
  const viewportHeight = useViewportHeight();
  const contextMenu = useContextMenu(`${label} actions`);
  const contextMenuRef = useRef(contextMenu);
  contextMenuRef.current = contextMenu;
  const frameRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const listboxRef = useRef<HTMLDivElement>(null);

  const viewportRef = useRef({ width: 0, height: 0, scrollLeft: 0, scrollTop: 0 });
  const bandsRef = useRef<Readonly<Record<string, readonly [number, number]>>>({});
  const focusedKeyRef = useRef<string | null>(null);
  const focusInsideRef = useRef(false);
  const focusAnchorRef = useRef({ x: 0, y: 0 });
  const inspectedKeyRef = useRef<string | null>(null);
  const lastPointerRef = useRef<{ x: number; y: number } | null>(null);
  const heldRef = useRef(held);
  heldRef.current = held;
  const dimRef = useRef(false);
  const pauseFlushRef = useRef(0);
  const lastAuditRef = useRef<{
    id: number;
    declaration: MeterProjection<TOptions>;
    options: TOptions;
    exactReadView: (hour: number) => Promise<BoundedReadView | null>;
  } | null>(null);
  const lastMirrorKeyRef = useRef('');
  const lastMirrorObjRef = useRef<readonly ListboxMirrorItem[] | null>(null);
  const lastExtentRef = useRef({ width: -1, height: -1 });
  const navRef = useRef<readonly FamilyNavTuple[]>([]);
  const lastHourRef = useRef(staticSources?.hour ?? 0);
  const typeaheadRef = useRef<{ query: string; until: number }>({ query: '', until: 0 });

  const [meta, setMeta] = useState<MeterMetaState>(() => ({
    mode: staticSources ? 'picture' : 'reading',
    reason: null,
    failureOrigin: 'local',
    empty: null,
    emptyDetail: null,
    emptyKind: 'empty',
    summary: null,
    notes: [],
    warnings: [],
  }));
  const metaSigRef = useRef('');
  const lastMetaKeyRef = useRef<{ mode: string; reason: string | null; failureOrigin: FailureOrigin; rawMeta: unknown } | null>(null);

  const { theme } = useFamilyTheme(countryColours, countryLooks);
  const { announce, hasFrame, liveRegionRef } = useFamilyAnnouncer('meters');

  const enabled = active && !prepared && !blockedState && (subscriptions.length > 0 || staticSources !== null || allowEmptyFollow);
  const auditable = !!exactReadView;
  const placement = useFamilyPlacement({
    family: 'meters',
    declarationKind: declaration.kind,
    widgetId: widget.id,
    widgetKind: widget.kind,
    owner,
    subject,
    widgetTrackIds: widget.trackIds,
    runtime,
    enabled,
    auditable,
    subscriptions,
    offset,
    pin: pin ?? (subscriptions.length === 0 && staticSources ? staticSources.hour : null),
    bounds,
    frameRef,
    surfaceLabel: 'meters',
  });
  const { pipeline, pipe, sourceIdsRef, subsReady, tooltipOwner, dataContextKey, retryStream, retryLocal } = placement;

  const tooltips = useFamilyTooltip(tooltipOwner, dataContextKey);
  useFamilyHold(frameRef, held, tooltips);

  const audit = useFamilyAudit<AuditSnapshot<TOptions>>({
    pipeline,
    pipe,
    ownerId: METER_AUDIT,
    auditable,
  });

  const reportShown = useShownHour();
  const reportShownRef = useRef(reportShown);
  reportShownRef.current = reportShown;

  const surfaceRef = useRef<MountedFamilyCanvas | null>(null);

  const latestRef = useRef({
    declaration,
    options,
    theme,
    staticSources,
    exactReadView,
    identitySnapshot,
    onIdentityDemands,
    onViewportSize,
    artResolutions,
    selectedKey,
    onActivate,
    contextMenuItems,
    viewportHeight,
    bounds,
    allowEmptyFollow,
    captureActive: audit.captureActive,
  });
  latestRef.current = {
    declaration,
    options,
    theme,
    staticSources,
    exactReadView,
    identitySnapshot,
    onIdentityDemands,
    onViewportSize,
    artResolutions,
    selectedKey,
    onActivate,
    contextMenuItems,
    viewportHeight,
    bounds,
    allowEmptyFollow,
    captureActive: audit.captureActive,
  };

  const bind = useCallback(() => {
    const mounted = surfaceRef.current;
    if (!mounted || !pipeline || pipe === null) return;
    const { width, height, scrollLeft, scrollTop } = viewportRef.current;
    const w = Math.max(1, width || 320);
    const h = Math.max(1, height || 200);
    const cur = latestRef.current;
    const staticPanels = cur.staticSources?.panels;
    const staticAnswers = cur.staticSources?.answers;

    let auditId: number | undefined;
    if (perfLog && cur.captureActive && cur.exactReadView) {
      const prev = lastAuditRef.current;
      if (prev && prev.declaration === cur.declaration && prev.options === cur.options && prev.exactReadView === cur.exactReadView) {
        auditId = prev.id;
      } else {
        auditId = audit.capture({
          declaration: cur.declaration,
          options: cur.options,
          exactReadView: cur.exactReadView,
        }, verifyMeterAuditSnapshot);
        lastAuditRef.current = {
          id: auditId,
          declaration: cur.declaration,
          options: cur.options,
          exactReadView: cur.exactReadView,
        };
      }
    }

    const bindingModel: FamilyMeterBinding<TOptions> = {
      family: 'meters',
      kind: cur.declaration.kind,
      options: cur.options,
      sources: sourceIdsRef.current,
      ...(staticPanels && Object.keys(staticPanels).length ? { staticPanels } : {}),
      ...(staticAnswers && Object.keys(staticAnswers).length ? { staticAnswers } : {}),
      ...(cur.identitySnapshot ? { identitySnapshot: cur.identitySnapshot } : {}),
      ...(cur.artResolutions ? { artResolutions: cur.artResolutions } : {}),
      width: w,
      height: h,
      scrollLeft,
      scrollTop,
      theme: cur.theme,
      selectedKey: cur.selectedKey,
      focusedKey: focusInsideRef.current ? focusedKeyRef.current : null,
      pauseFlush: pauseFlushRef.current,
      ...(perfLog && cur.captureActive && auditId !== undefined ? { audit: true, auditId } : {}),
    };

    mounted.canvas.style.width = `${w}px`;
    mounted.canvas.style.height = `${h}px`;
    pipeline.present(pipe, 'meters');
    const dated = Object.keys(sourceIdsRef.current).length === 0
      ? (cur.staticSources
        ? (cur.staticSources.provenInterval
          ? { h: cur.staticSources.provenInterval[0], c: cur.staticSources.provenInterval[1] }
          : { h: cur.staticSources.hour, c: cur.staticSources.hour })
        : cur.allowEmptyFollow && cur.bounds
          ? { h: cur.bounds[0], c: cur.bounds[1] }
          : undefined)
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
  }, [pipeline, pipe, audit.capture, sourceIdsRef]);

  const bindRef = useRef(bind);
  bindRef.current = bind;

  const paintMirror = useCallback((items: readonly ListboxMirrorItem[] | null) => {
    syncListboxMirror(listboxRef.current, items, {
      optionIdPrefix: listboxPrefix,
      total: navRef.current.length,
      focusedKey: focusedKeyRef.current,
      lastMirrorKeyRef,
    });
  }, [listboxPrefix]);
  const paintMirrorRef = useRef(paintMirror);
  paintMirrorRef.current = paintMirror;

  const scrollKeyIntoView = useCallback((key: string) => {
    const band = bandsRef.current[key];
    if (!band) return;
    if (scrollRangeIntoView(bodyRef.current, viewportRef.current, band[0], band[1])) {
      bindRef.current();
    }
  }, []);
  const scrollKeyIntoViewRef = useRef(scrollKeyIntoView);
  scrollKeyIntoViewRef.current = scrollKeyIntoView;

  const refreshFocusAnchor = useCallback(() => {
    const box = frameRef.current?.getBoundingClientRect();
    if (box) focusAnchorRef.current = { x: box.left + Math.min(200, box.width / 2), y: box.top + 48 };
  }, []);
  const refreshFocusAnchorRef = useRef(refreshFocusAnchor);
  refreshFocusAnchorRef.current = refreshFocusAnchor;

  const focusRowByKey = useCallback((key: string) => {
    focusedKeyRef.current = key;
    scrollKeyIntoView(key);
    paintMirrorRef.current(lastMirrorObjRef.current);
    bindRef.current();
  }, [scrollKeyIntoView]);

  useLayoutEffect(() => {
    const node = bodyRef.current;
    if (!node || !pipeline || pipe === null) return;
    lastExtentRef.current = { width: -1, height: -1 };
    lastMetaKeyRef.current = null;
    lastMirrorObjRef.current = null;

    const mounted = mountFamilyCanvas(node, pipeline, pipe, 'meters', 'mf-canvas', 'mf-extent');
    surfaceRef.current = mounted;

    const auditWatch = attachAuditWatch(pipeline, pipe, {
      isAuditable: () => !!latestRef.current.exactReadView,
      onCaptureChange: activeAudit => { latestRef.current.captureActive = activeAudit; },
      bindNow: () => bindRef.current(),
    });

    const activateHit = (rowKey: string) => {
      const entry = navRef.current.find(item => item[0] === rowKey);
      latestRef.current.onActivate?.(rowKey, entry?.[5], lastHourRef.current);
    };

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

      auditWatch.settleIfReady(visible, mode, shown?.installedBinding, shown?.fields);

      const rawMeta = shown?.fields?.meta as Omit<MeterMetaState, 'mode' | 'reason' | 'failureOrigin'> | undefined;
      const prevMetaKey = lastMetaKeyRef.current;
      if (
        !prevMetaKey
        || prevMetaKey.mode !== mode
        || prevMetaKey.reason !== reason
        || prevMetaKey.failureOrigin !== failureOrigin
        || prevMetaKey.rawMeta !== rawMeta
      ) {
        lastMetaKeyRef.current = { mode, reason, failureOrigin, rawMeta };
        const nextMetaSig = JSON.stringify([
          mode,
          reason,
          failureOrigin,
          rawMeta?.empty ?? null,
          rawMeta?.emptyDetail ?? null,
          rawMeta?.emptyKind ?? 'empty',
          rawMeta?.summary ?? null,
          rawMeta?.notes ?? [],
          rawMeta?.warnings ?? [],
        ]);
        if (nextMetaSig !== metaSigRef.current) {
          metaSigRef.current = nextMetaSig;
          setMeta({
            mode,
            reason,
            failureOrigin,
            empty: rawMeta?.empty ?? null,
            emptyDetail: rawMeta?.emptyDetail ?? null,
            emptyKind: rawMeta?.emptyKind ?? 'empty',
            summary: rawMeta?.summary ?? null,
            notes: rawMeta?.notes ?? [],
            warnings: rawMeta?.warnings ?? [],
          });
        }
      }

      const demandsField = shown?.fields?.identityDemands as readonly (readonly [EntityKind, string, number])[] | undefined;
      if (demandsField && latestRef.current.onIdentityDemands) {
        latestRef.current.onIdentityDemands(demandsField);
      }

      if (!visible || !shown?.fields) {
        lastMirrorObjRef.current = null;
        paintMirrorRef.current(null);
        tooltips.hideIfUnpinned();
        return;
      }

      const navField = shown.fields.nav as readonly FamilyNavTuple[] | undefined;
      if (navField) navRef.current = navField;

      const extent = shown.fields.extent as MeterExtentState | undefined;
      if (extent) {
        bandsRef.current = extent.bands;
        const extW = Math.max(viewportRef.current.width, extent.width);
        const extH = Math.max(viewportRef.current.height, extent.height);
        if (lastExtentRef.current.width !== extW) {
          lastExtentRef.current.width = extW;
          mounted.extent.style.width = `${extW}px`;
        }
        if (lastExtentRef.current.height !== extH) {
          lastExtentRef.current.height = extH;
          mounted.extent.style.height = `${extH}px`;
        }
      }

      if (focusInsideRef.current && focusedKeyRef.current) {
        const entry = navRef.current.find(item => item[0] === focusedKeyRef.current);
        if (!entry) {
          const fallbackKey = navRef.current[0]?.[0] ?? null;
          if (fallbackKey !== focusedKeyRef.current) {
            focusedKeyRef.current = fallbackKey;
            queueMicrotask(() => bindRef.current());
          }
        }
      }

      const mirror = shown.fields.mirror as readonly ListboxMirrorItem[] | undefined;
      if (mirror && mirror !== lastMirrorObjRef.current) {
        lastMirrorObjRef.current = mirror;
        paintMirrorRef.current(mirror);
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
        held: heldRef.current,
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
      accessibleRef: listboxRef,
      isBlocked: () => heldRef.current || dimRef.current,
      getViewportHeight: () => latestRef.current.viewportHeight,
      onActivate: rowKey => activateHit(rowKey),
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

    const onScroll = () => {
      viewportRef.current.scrollLeft = node.scrollLeft;
      viewportRef.current.scrollTop = node.scrollTop;
      bindRef.current();
    };

    const resizeObserver = new ResizeObserver(entries => {
      const rect = entries[0]?.contentRect;
      const rawW = rect ? rect.width : node.clientWidth;
      const rawH = rect ? rect.height : node.clientHeight;
      latestRef.current.onViewportSize?.(rawW, rawH);
      const w = Math.round(rawW);
      const h = Math.round(rawH);
      refreshFocusAnchorRef.current();
      if (w === viewportRef.current.width && h === viewportRef.current.height) return;
      viewportRef.current.width = w;
      viewportRef.current.height = h;
      bindRef.current();
    });
    resizeObserver.observe(node);
    viewportRef.current.width = node.clientWidth;
    viewportRef.current.height = node.clientHeight;
    latestRef.current.onViewportSize?.(node.clientWidth, node.clientHeight);
    refreshFocusAnchorRef.current();

    node.addEventListener('scroll', onScroll, { passive: true });
    bindRef.current();

    return () => {
      auditWatch.dispose();
      stopInstall();
      stopPointer();
      resizeObserver.disconnect();
      node.removeEventListener('scroll', onScroll);
      mounted.unmount();
      surfaceRef.current = null;
      lastExtentRef.current = { width: -1, height: -1 };
      holdElement(frameRef.current, false);
    };
  }, [pipeline, pipe, tooltips]);

  useLayoutEffect(() => {
    if (pipe !== null && enabled) bind();
  }, [pipe, enabled, bind, options, theme, staticSources, identitySnapshot, artResolutions, selectedKey, subsReady, auditable, exactReadView, audit.captureActive]);

  useFamilyPauseFlush(runtime.clock, () => {
    if (!pipeline || pipe === null) return;
    pauseFlushRef.current++;
    bindRef.current();
    const view = pipeline.view(pipe);
    const mirror = view?.fields?.mirror as readonly ListboxMirrorItem[] | undefined;
    if (mirror) paintMirror(mirror);
  });

  useEffect(() => {
    if (!onDiagnostics) return;
    const all = [...(extraWarnings ?? []), ...meta.warnings];
    onDiagnostics(all);
    return () => onDiagnostics([]);
  }, [onDiagnostics, extraWarnings, meta.warnings]);

  const handleKeyDown = useCallback((event: React.KeyboardEvent) => {
    const pageStep = Math.max(1, Math.floor((viewportRef.current.height || 200) / 32) - 1);
    handleFamilyKeyDown(event, {
      nav: navRef.current,
      focusedKey: focusedKeyRef.current,
      pageStep,
      blocked: heldRef.current || dimRef.current,
      typeaheadRef,
      isPinnedTooltip: () => tooltips.isPinned(),
      onFocusKey: focusRowByKey,
      onActivate: entry => latestRef.current.onActivate?.(entry[0], entry[5], lastHourRef.current),
      onContextMenu: entry => {
        const builder = latestRef.current.contextMenuItems;
        if (!builder) return false;
        const items = builder(entry ? entry[0] : null, entry ? entry[1] : null, lastHourRef.current);
        if (!items.length) return false;
        refreshFocusAnchor();
        contextMenuRef.current.open(focusAnchorRef.current.x, focusAnchorRef.current.y, items);
        return true;
      },
      onTogglePinTooltip: currentKey => {
        if (!pipeline || pipe === null || heldRef.current || dimRef.current || !currentKey) return;
        const focusTipField = pipeline.view(pipe)?.fields?.focusTooltip as FamilyTooltipField | undefined;
        if (focusTipField?.text && focusTipField.text.rowKey === currentKey) {
          refreshFocusAnchor();
          tooltips.pinFocused(focusTipField.text, lastHourRef.current, focusAnchorRef.current, latestRef.current.viewportHeight, announce);
        }
      },
      onUnpinTooltip: () => tooltips.unpin(announce),
    });
  }, [announce, focusRowByKey, pipe, pipeline, refreshFocusAnchor, tooltips]);

  const onRetryLocal = useCallback(() => retryLocal(() => bindRef.current()), [retryLocal]);

  if (blockedState) return <>{blockedState}</>;

  const summaryNode = headerSlot && meta.mode === 'picture' && !meta.empty && meta.summary
    ? createPortal(<span>{meta.summary}</span>, headerSlot)
    : null;

  const bodyStateNode = renderFamilyBodyState({
    mode: meta.mode,
    reason: meta.reason,
    failureOrigin: meta.failureOrigin,
    empty: meta.empty,
    emptyDetail: meta.emptyDetail,
    emptyKind: meta.emptyKind,
    subject,
    loadingLabel,
    pipeline,
    pipe,
    onRetryStream: retryStream,
    onRetryLocal,
  });

  return <div ref={frameRef} className={`mf-frame${className ? ` ${className}` : ''}`}>
    {summaryNode}
    {meta.notes.length > 0 && !bodyStateNode && <WidgetCaption>{meta.notes.join(' ')}</WidgetCaption>}
    <div className="mf-body-wrap">
      <div
        ref={bodyRef}
        className="mf-body"
        data-clickable={onActivate ? 'true' : undefined}
      />
      {bodyStateNode && <div className="mf-empty-body">{bodyStateNode}</div>}
    </div>
    <div
      ref={listboxRef}
      className="mf-accessibility"
      role="listbox"
      tabIndex={0}
      aria-label={label}
      onFocus={() => {
        focusInsideRef.current = true;
        refreshFocusAnchor();
        if (!focusedKeyRef.current && navRef.current.length) {
          focusedKeyRef.current = navRef.current[0]![0];
        }
        paintMirrorRef.current(lastMirrorObjRef.current);
        bindRef.current();
      }}
      onBlur={event => {
        if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
        focusInsideRef.current = false;
        if (!inspectedKeyRef.current) tooltips.hideIfUnpinned();
        bindRef.current();
      }}
      onKeyDown={handleKeyDown}
    />
    {!hasFrame && <div ref={liveRegionRef} className="mf-live-region" role="status" aria-live="polite" aria-atomic="true" />}
    {contextMenu.menu}
  </div>;
}
