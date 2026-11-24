import { useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
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
import { auditIdOf } from '../audit.ts';
import { displayedHour } from '../boundedView.ts';
import type {
  BoundedReadView,
  EntityKind,
  FamilySourceSubscription,
  InstalledAction,
  InstalledActionTarget,
  InstalledPicture,
} from '../contract.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import {
  attachAuditWatch,
  bindFamilyPointer,
  createPlanAuditVerifier,
  handleFamilyKeyDown,
  mountFamilyCanvas,
  renderFamilyBodyState,
  reportFamilyShownHour,
  scrollRangeIntoView,
  useFamilyAnnouncer,
  useFamilyAudit,
  useFamilyHold,
  useFamilyPauseFlush,
  useFamilyPlacement,
  useFamilyTheme,
  useFamilyTooltip,
  type FamilyNavTuple,
  type FamilyTooltipField,
  type MountedFamilyCanvas,
  type PlanAuditSnapshot,
} from '../host/index.ts';
import type { CountryLooksMap } from '../looks.ts';
import type { FamilyTimelineBinding } from './laneView.ts';
import { deriveTimelineAudit, type TimelineModel, type TimelineProjection } from './model.ts';
import { decodeTimelineHit } from './painter.ts';
import './timelineFamily.css';

const TIMELINE_AUDIT = -3;

export interface FamilyTimelineHostProps<TOptions> {
  readonly declaration: TimelineProjection<TOptions>;
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
  readonly onViewportSize?: (width: number, height: number, dpr: number) => void;
  readonly hoverPanels?: Readonly<Record<string, { readonly hour: number; readonly data: unknown }>>;
  readonly exactReadView?: (hour: number) => Promise<BoundedReadView | null>;
  readonly offset: number;
  readonly pin: number | null;
  readonly bounds?: readonly [number, number] | null;
  readonly countryColours?: ReadonlyMap<string, string>;
  readonly countryLooks?: CountryLooksMap;
  readonly identitySnapshot?: IdentitySnapshot | null;
  readonly onIdentityDemands?: (demands: readonly (readonly [EntityKind, string, number])[]) => void;
  readonly selectedKey?: string | null;
  readonly onActivate?: (key: string, meta?: unknown, hour?: number) => void;
  readonly onHoverHour?: (hour: number | null) => void;
  readonly contextMenuItems?: (key: string | null, name: string | null, hour: number) => MenuItem[];
  readonly headerSlot?: HTMLElement | null;
  readonly onDiagnostics?: (warnings: string[]) => void;
  readonly extraWarnings?: readonly string[];
  readonly blockedState?: ReactNode;
  readonly held?: boolean;
  readonly loadingLabel?: string;
  readonly className?: string;
}

interface TimelineMetaState {
  mode: string;
  reason: string | null;
  failureOrigin: FailureOrigin;
  headerSummary: string | null;
  empty: string | null;
  emptyDetail: string | null;
  emptyKind: 'empty' | 'unavailable';
  warnings: readonly string[];
  notes: readonly string[];
  totalHeight: number;
}

export default function FamilyTimelineHost<TOptions>({
  declaration,
  options,
  label,
  subject,
  widget,
  runtime,
  active,
  subscriptions,
  allowEmptyFollow = true,
  owner,
  staticSources = null,
  onViewportSize,
  hoverPanels,
  exactReadView,
  offset,
  pin,
  bounds,
  countryColours,
  countryLooks,
  identitySnapshot = null,
  onIdentityDemands,
  selectedKey = null,
  onActivate,
  onHoverHour,
  contextMenuItems,
  headerSlot = null,
  onDiagnostics,
  extraWarnings,
  blockedState = null,
  held = false,
  loadingLabel,
  className,
}: FamilyTimelineHostProps<TOptions>) {
  const prepared = useContext(PreparingPage);
  useWidgetSkeleton('chart');
  const viewportHeight = useViewportHeight();
  const contextMenu = useContextMenu(`${label} actions`);
  const contextMenuRef = useRef(contextMenu);
  contextMenuRef.current = contextMenu;
  const frameRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const accessibleRef = useRef<HTMLDivElement>(null);

  const viewportRef = useRef({ width: 0, height: 0, scrollTop: 0 });
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const expandedRef = useRef(expanded);
  expandedRef.current = expanded;

  const focusedKeyRef = useRef<string | null>(null);
  const focusInsideRef = useRef(false);
  const focusAnchorRef = useRef({ x: 0, y: 0 });
  const inspectedKeyRef = useRef<string | null>(null);
  const lastPointerRef = useRef<{ x: number; y: number } | null>(null);
  const heldRef = useRef(held);
  heldRef.current = held;
  const dimRef = useRef(false);
  const pauseFlushRef = useRef(0);
  const navRef = useRef<readonly FamilyNavTuple[]>([]);
  const installedPictureRef = useRef<InstalledPicture | null>(null);
  const lastHourRef = useRef(staticSources?.hour ?? bounds?.[1] ?? 0);
  const typeaheadRef = useRef<{ query: string; until: number }>({ query: '', until: 0 });
  const lastExtentHeightRef = useRef(-1);

  const [meta, setMeta] = useState<TimelineMetaState>(() => ({
    mode: staticSources ? 'picture' : 'reading',
    reason: null,
    failureOrigin: 'local',
    headerSummary: null,
    empty: null,
    emptyDetail: null,
    emptyKind: 'empty',
    warnings: [],
    notes: [],
    totalHeight: 0,
  }));
  const metaSigRef = useRef('');

  const { theme } = useFamilyTheme(countryColours, countryLooks);
  const { announce, hasFrame, liveRegionRef } = useFamilyAnnouncer('timeline');

  const enabled = active && !prepared && !blockedState && (subscriptions.length > 0 || staticSources !== null || allowEmptyFollow);
  const auditable = !!exactReadView;

  const placement = useFamilyPlacement({
    family: 'timeline',
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
    pin,
    bounds,
    frameRef,
    surfaceLabel: 'timeline',
  });
  const { pipeline, pipe, sourceIdsRef, subsReady, tooltipOwner, dataContextKey, retryStream, retryLocal } = placement;

  const tooltips = useFamilyTooltip(tooltipOwner, dataContextKey);
  useFamilyHold(frameRef, held, tooltips);

  const auditVerifier = useMemo(
    () => createPlanAuditVerifier<TimelineModel, { readonly options: TOptions; readonly expanded: Readonly<Record<string, boolean>> }>(),
    [],
  );
  const audit = useFamilyAudit<PlanAuditSnapshot<TimelineModel, { readonly options: TOptions; readonly expanded: Readonly<Record<string, boolean>> }>>({
    pipeline,
    pipe,
    ownerId: TIMELINE_AUDIT,
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
    onViewportSize,
    hoverPanels,
    exactReadView,
    selectedKey,
    onActivate,
    onHoverHour,
    contextMenuItems,
    viewportHeight,
    offset,
    bounds,
    allowEmptyFollow,
    captureActive: audit.captureActive,
    identitySnapshot,
    onIdentityDemands,
  });
  latestRef.current = {
    declaration,
    options,
    theme,
    staticSources,
    onViewportSize,
    hoverPanels,
    exactReadView,
    selectedKey,
    onActivate,
    onHoverHour,
    contextMenuItems,
    viewportHeight,
    offset,
    bounds,
    allowEmptyFollow,
    captureActive: audit.captureActive,
    identitySnapshot,
    onIdentityDemands,
  };

  const bind = useCallback(() => {
    const mounted = surfaceRef.current;
    if (!mounted || !pipeline || pipe === null) return;
    const { width, height, scrollTop } = viewportRef.current;
    const w = Math.max(1, width || 320);
    const h = Math.max(1, height || 200);
    const cur = latestRef.current;
    const staticPanels = cur.staticSources?.panels;
    const staticAnswers = cur.staticSources?.answers;

    let auditId: number | undefined;
    if (perfLog && cur.captureActive && cur.exactReadView) {
      const expandedSnap = expandedRef.current;
      const decl = cur.declaration;
      auditId = audit.capture({
        projection: {
          kind: decl.kind,
          family: 'timeline',
          project: (readView, wrapped) => decl.project(readView, wrapped.options, wrapped.expanded),
        },
        options: { options: cur.options, expanded: expandedSnap },
        exactReadView: cur.exactReadView,
        deriveRows: model => deriveTimelineAudit(model, expandedSnap),
      }, auditVerifier);
    }

    const bindingModel: FamilyTimelineBinding<TOptions> = {
      family: 'timeline',
      kind: cur.declaration.kind,
      options: cur.options,
      sources: sourceIdsRef.current,
      ...(staticPanels && Object.keys(staticPanels).length ? { staticPanels } : {}),
      ...(staticAnswers && Object.keys(staticAnswers).length ? { staticAnswers } : {}),
      ...(cur.hoverPanels && Object.keys(cur.hoverPanels).length ? { hoverPanels: cur.hoverPanels } : {}),
      ...(cur.identitySnapshot ? { identitySnapshot: cur.identitySnapshot } : {}),
      width: w,
      height: h,
      scrollTop,
      theme: cur.theme,
      selectedKey: cur.selectedKey,
      focusedKey: focusInsideRef.current ? focusedKeyRef.current : null,
      expanded: expandedRef.current,
      pauseFlush: pauseFlushRef.current,
      ...(perfLog && cur.captureActive && auditId !== undefined ? { audit: true, auditId } : {}),
    };

    mounted.canvas.style.width = `${w}px`;
    mounted.canvas.style.height = `${h}px`;
    pipeline.present(pipe, 'timeline');
    const dated = Object.keys(sourceIdsRef.current).length === 0
      ? (cur.staticSources
        ? (cur.staticSources.provenInterval
          ? { h: cur.staticSources.provenInterval[0], c: cur.staticSources.provenInterval[1] }
          : { h: cur.staticSources.hour, c: cur.staticSources.hour })
        : cur.bounds
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
  }, [pipeline, pipe, audit.capture, auditVerifier, sourceIdsRef]);

  const bindRef = useRef(bind);
  bindRef.current = bind;

  const toggleKey = useCallback((key: string) => {
    const entry = navRef.current.find(item => item[0] === key);
    if (!entry || entry[3] < 0) return;
    const nextOpen = entry[3] === 0;
    setExpanded(prev => ({ ...prev, [key]: nextOpen }));
    announce(`${entry[1]} ${nextOpen ? 'expanded' : 'collapsed'}.`);
  }, [announce]);
  const toggleKeyRef = useRef(toggleKey);
  toggleKeyRef.current = toggleKey;

  useLayoutEffect(() => {
    const node = bodyRef.current;
    if (!node || !pipeline || pipe === null) return;
    lastExtentHeightRef.current = -1;

    const mounted = mountFamilyCanvas(node, pipeline, pipe, 'timeline', 'tl-canvas', 'tl-extent');
    surfaceRef.current = mounted;

    const auditWatch = attachAuditWatch(pipeline, pipe, {
      isAuditable: () => !!latestRef.current.exactReadView,
      onCaptureChange: activeAudit => { latestRef.current.captureActive = activeAudit; },
      bindNow: () => bindRef.current(),
    });

    const activateHit = (key: string, isToggle: boolean, action?: InstalledAction) => {
      const installed = installedPictureRef.current;
      if (!installed) return;
      if (action && (action.installedId !== installed.id || action.owner !== installed.owner)) return;
      const target = installed.actionKeys.get(key);
      if (isToggle || target?.toggle) {
        toggleKeyRef.current(key);
        return;
      }
      latestRef.current.onActivate?.(target?.subject ?? key, target?.meta, target?.hour ?? installed.hour);
    };

    const install = () => {
      const shown = pipeline.view(pipe);
      const frameNode = frameRef.current;
      dimRef.current = !!shown?.dim;
      holdElement(frameNode, !!shown?.dim);
      const visible = shown?.mode === 'picture';
      node.toggleAttribute('data-timeline-visible', visible);

      const mode = shown?.mode ?? 'reading';
      const reason = shown?.reason ?? null;
      const failureOrigin: FailureOrigin = shown?.failureOrigin ?? 'local';
      const shownHour = reportFamilyShownHour(reportShownRef.current, visible, shown);
      if (shownHour !== null) lastHourRef.current = shownHour;

      auditWatch.settleIfReady(visible, mode, shown?.installedBinding, shown?.fields);

      const rawMeta = shown?.fields?.meta as {
        headerSummary?: string | null;
        empty?: string | null;
        emptyDetail?: string | null;
        emptyKind?: 'empty' | 'unavailable';
        warnings?: readonly string[];
        notes?: readonly string[];
        totalHeight?: number;
      } | undefined;

      const nextMetaSig = JSON.stringify([
        mode,
        reason,
        failureOrigin,
        rawMeta?.headerSummary ?? null,
        rawMeta?.empty ?? null,
        rawMeta?.emptyDetail ?? null,
        rawMeta?.emptyKind ?? 'empty',
        rawMeta?.warnings ?? [],
        rawMeta?.notes ?? [],
      ]);
      if (nextMetaSig !== metaSigRef.current) {
        metaSigRef.current = nextMetaSig;
        setMeta({
          mode,
          reason,
          failureOrigin,
          headerSummary: rawMeta?.headerSummary ?? null,
          empty: rawMeta?.empty ?? null,
          emptyDetail: rawMeta?.emptyDetail ?? null,
          emptyKind: rawMeta?.emptyKind ?? 'empty',
          warnings: rawMeta?.warnings ?? [],
          notes: rawMeta?.notes ?? [],
          totalHeight: rawMeta?.totalHeight ?? 0,
        });
      }

      const demandsField = shown?.fields?.identityDemands as readonly (readonly [EntityKind, string, number])[] | undefined;
      if (demandsField && latestRef.current.onIdentityDemands) {
        latestRef.current.onIdentityDemands(demandsField);
      }

      if (!visible || !shown?.fields || shown.id === undefined) {
        installedPictureRef.current = null;
        tooltips.hideIfUnpinned();
        return;
      }

      const extH = Math.max(viewportRef.current.height, rawMeta?.totalHeight ?? 0);
      if (lastExtentHeightRef.current !== extH) {
        lastExtentHeightRef.current = extH;
        mounted.extent.style.height = `${extH}px`;
      }

      const navField = shown.fields.nav as readonly FamilyNavTuple[] | undefined;
      if (navField) navRef.current = navField;
      const activeNav = navField ?? navRef.current;
      const installedHour = displayedHour(shownHour ?? shown.painted ?? shown.h ?? lastHourRef.current);
      const actionKeys = new Map<string, InstalledActionTarget>();
      for (const entry of activeNav) {
        actionKeys.set(entry[0], {
          subject: entry[0],
          hour: installedHour,
          ...(entry[3] >= 0 ? { toggle: true } : {}),
          label: entry[1],
          meta: entry[5],
        });
      }
      const installedViewport = shown.viewport ?? {
        width: viewportRef.current.width,
        height: viewportRef.current.height,
        dpr: typeof devicePixelRatio === 'number' ? devicePixelRatio : 1,
        scrollLeft: 0,
        scrollTop: viewportRef.current.scrollTop,
      };
      const installedAuditId = auditIdOf(shown.fields.audit);
      installedPictureRef.current = {
        id: shown.id,
        owner: shown.installedOwner ?? shown.owner ?? '',
        generation: {
          seek: shown.gen?.seek ?? 0,
          reset: shown.gen?.reset ?? 0,
          binding: shown.installedBinding ?? shown.binding ?? 0,
        },
        factRevisions: {},
        validInterval: shown.validInterval ?? [installedHour, installedHour],
        through: shown.through ?? installedHour,
        hour: installedHour,
        viewport: installedViewport,
        optionsRevision: '',
        themeRevision: '',
        actionKeys,
        accessibleDescription: null,
        ...(installedAuditId !== undefined ? { auditId: installedAuditId } : {}),
      };

      const hoverTipField = shown.fields.tooltip as FamilyTooltipField | undefined;
      const focusTipField = shown.fields.focusTooltip as FamilyTooltipField | undefined;
      tooltips.syncFromInstall({
        dim: !!shown.dim,
        held: heldRef.current,
        inspectedKey: inspectedKeyRef.current,
        pointerPos: lastPointerRef.current,
        hoverTip: hoverTipField?.text ?? null,
        focusInside: focusInsideRef.current,
        focusedKey: focusedKeyRef.current,
        focusAnchor: focusAnchorRef.current,
        focusTip: focusTipField?.text ?? null,
        viewportHeight: latestRef.current.viewportHeight,
      });
    };

    const stopInstall = pipeline.onInstall(pipe, install);

    const updateSize = () => {
      const box = node.getBoundingClientRect();
      const nextW = Math.round(box.width);
      const nextH = Math.round(box.height);
      const frameBox = frameRef.current?.getBoundingClientRect();
      if (frameBox) focusAnchorRef.current = { x: frameBox.left + Math.min(200, frameBox.width / 2), y: frameBox.top + 48 };
      if (nextW === viewportRef.current.width && nextH === viewportRef.current.height) return;
      viewportRef.current.width = nextW;
      viewportRef.current.height = nextH;
      latestRef.current.onViewportSize?.(nextW, nextH, typeof devicePixelRatio === 'number' ? devicePixelRatio : 1);
      bindRef.current();
    };
    updateSize();
    const ro = typeof ResizeObserver === 'function' ? new ResizeObserver(updateSize) : null;
    ro?.observe(node);

    const onScroll = () => {
      const nextTop = node.scrollTop;
      if (nextTop === viewportRef.current.scrollTop) return;
      viewportRef.current.scrollTop = nextTop;
      bindRef.current();
    };
    node.addEventListener('scroll', onScroll, { passive: true });

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
      isBlocked: () => dimRef.current || heldRef.current || !installedPictureRef.current,
      getInstalled: () => installedPictureRef.current,
      getViewportHeight: () => latestRef.current.viewportHeight,
      parseHit: raw => {
        const parsed = decodeTimelineHit(raw);
        latestRef.current.onHoverHour?.(parsed?.hour ?? null);
        if (!parsed) return null;
        return { key: parsed.key, toggle: parsed.kind === 'toggle' };
      },
      inspectToken: key => `row:${key}`,
      onActivate: activateHit,
      onContextMenu: (key, x, y) => {
        const installed = installedPictureRef.current;
        const build = latestRef.current.contextMenuItems;
        if (!installed || !build) return false;
        const target = key ? installed.actionKeys.get(key) : undefined;
        const items = build(key, key ? (target?.label ?? key) : null, target?.hour ?? installed.hour);
        if (!items.length) return false;
        contextMenuRef.current.open(x, y, items);
        return true;
      },
    });

    bindRef.current();
    install();

    return () => {
      stopPointer();
      node.removeEventListener('scroll', onScroll);
      ro?.disconnect();
      stopInstall();
      auditWatch.dispose();
      mounted.unmount();
      surfaceRef.current = null;
      reportFamilyShownHour(reportShownRef.current, false, null);
    };
  }, [pipeline, pipe, tooltips]);

  useEffect(() => {
    bind();
  }, [bind, declaration, options, theme, staticSources, hoverPanels, identitySnapshot, expanded, selectedKey, subsReady, audit.captureActive]);

  useFamilyPauseFlush(runtime.clock, () => {
    pauseFlushRef.current++;
    bindRef.current();
  });

  const combinedWarnings = useMemo(() => {
    const all = [...meta.warnings, ...(extraWarnings ?? [])];
    return all.length ? all : [];
  }, [meta.warnings, extraWarnings]);

  useEffect(() => {
    onDiagnostics?.(combinedWarnings);
  }, [combinedWarnings, onDiagnostics]);

  const onKeyDown = useCallback((event: React.KeyboardEvent<HTMLDivElement>) => {
    handleFamilyKeyDown(event, {
      nav: navRef.current,
      focusedKey: focusedKeyRef.current,
      typeaheadRef,
      pageStep: Math.max(1, Math.floor((viewportRef.current.height || 200) / 36)),
      blocked: dimRef.current || heldRef.current || !installedPictureRef.current,
      isPinnedTooltip: () => tooltips.isPinned(),
      onFocusKey: key => {
        focusedKeyRef.current = key;
        const entry = navRef.current.find(item => item[0] === key);
        if (entry) {
          scrollRangeIntoView(bodyRef.current, viewportRef.current, entry[2] * 36, 36);
        }
        bindRef.current();
      },
      onToggleTree: key => toggleKeyRef.current(key),
      onActivate: entry => {
        const installed = installedPictureRef.current;
        if (!installed) return;
        const target = installed.actionKeys.get(entry[0]);
        latestRef.current.onActivate?.(target?.subject ?? entry[0], target?.meta ?? entry[5], target?.hour ?? installed.hour);
      },
      onContextMenu: entry => {
        const installed = installedPictureRef.current;
        const build = latestRef.current.contextMenuItems;
        if (!installed || !build) return false;
        const key = entry?.[0] ?? null;
        const target = key ? installed.actionKeys.get(key) : undefined;
        const items = build(key, key ? (target?.label ?? entry?.[1] ?? key) : null, target?.hour ?? installed.hour);
        if (!items.length) return false;
        const pos = focusAnchorRef.current;
        contextMenuRef.current.open(pos.x, pos.y, items);
        return true;
      },
      onUnpinTooltip: () => tooltips.unpin(announce),
      onTogglePinTooltip: () => {
        if (tooltips.isPinned()) {
          tooltips.unpin(announce);
          return;
        }
        const focusTipField = pipeline && pipe !== null ? pipeline.view(pipe)?.fields?.focusTooltip as FamilyTooltipField | undefined : undefined;
        const tip = focusTipField?.text ?? null;
        if (tip) {
          const box = frameRef.current?.getBoundingClientRect();
          const pos = box ? { x: box.left + Math.min(200, box.width / 2), y: box.top + 48 } : focusAnchorRef.current;
          tooltips.pinFocused(tip, lastHourRef.current, pos, latestRef.current.viewportHeight, announce);
        }
      },
    });
  }, [announce, pipeline, pipe, tooltips]);

  const bodyState = blockedState ?? renderFamilyBodyState({
    mode: meta.mode,
    reason: meta.reason,
    failureOrigin: meta.failureOrigin,
    empty: meta.empty,
    emptyDetail: meta.emptyDetail,
    emptyKind: meta.emptyKind,
    subject,
    loadingLabel: loadingLabel ?? `Loading ${subject}…`,
    pipeline,
    pipe,
    onRetryStream: retryStream,
    onRetryLocal: () => retryLocal(),
  });

  const headerPortal = headerSlot && meta.headerSummary
    ? createPortal(<span className="tl-header-summary">{meta.headerSummary}</span>, headerSlot)
    : null;

  return (
    <div ref={frameRef} className={`tl-frame ${className ?? ''}`.trim()} role="region" aria-label={label}>
      {headerPortal}
      {bodyState}
      <div
        ref={bodyRef}
        className="tl-body"
        style={bodyState ? { display: 'none' } : undefined}
        tabIndex={0}
        onFocus={() => {
          focusInsideRef.current = true;
          if (!focusedKeyRef.current && navRef.current.length > 0) {
            focusedKeyRef.current = navRef.current[0]![0];
          }
          bindRef.current();
        }}
        onBlur={() => {
          focusInsideRef.current = false;
          tooltips.hideIfUnpinned();
          bindRef.current();
        }}
        onKeyDown={onKeyDown}
      />
      {meta.notes.length > 0 ? <WidgetCaption>{meta.notes.join(' ')}</WidgetCaption> : null}
      <div ref={accessibleRef} className="tl-sr-only" aria-hidden="true" />
      {!hasFrame ? <div ref={liveRegionRef} className="tl-sr-only" aria-live="polite" /> : null}
      {contextMenu.menu}
    </div>
  );
}
