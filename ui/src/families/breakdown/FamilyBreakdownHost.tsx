import { useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useContextMenu, type MenuItem } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { useViewportHeight } from '../../charts/useChartTokens';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { FailureOrigin } from '../../dashboard/follow/page.ts';
import type { WidgetSpec } from '../../dashboard/model';
import type { OptionValue } from '../../dashboard/options';
import { PreparingPage } from '../../dashboard/preparing.ts';
import { useWidgetSkeleton } from '../../dashboard/widgetFrame.tsx';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { holdElement } from '../../draw/bitmapSurface.ts';
import { perfLog } from '../../shell/perf.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { BoundedReadView, FamilySourceSubscription } from '../contract.ts';
import {
  attachAuditWatch,
  bindFamilyPointer,
  handleFamilyKeyDown,
  mountFamilyCanvas,
  renderFamilyBodyState,
  reportFamilyShownHour,
  syncListboxMirror,
  useFamilyAnnouncer,
  useFamilyAudit,
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
import type { CountryLooksMap } from '../looks.ts';
import type { EntityKind } from '../plan.ts';
import type { FamilyBreakdownBinding } from './laneView.ts';
import { deriveBreakdownAudit, type BreakdownEmptyAction, type BreakdownProjection } from './model.ts';
import type { BreakdownViewMode } from './options.ts';
import './breakdownFamily.css';

const BREAKDOWN_AUDIT = -2;

interface BreakdownAuditSnapshot<TOptions> {
  readonly declaration: BreakdownProjection<TOptions>;
  readonly options: TOptions;
  readonly drillPath: readonly string[];
  readonly exactReadView: (hour: number) => Promise<BoundedReadView | null>;
}

async function verifyBreakdownAuditSnapshot<TOptions>(hour: number, snap: BreakdownAuditSnapshot<TOptions>) {
  const exact = await snap.exactReadView(hour);
  if (!exact) return null;
  const model = snap.declaration.project(exact, snap.options);
  const rows = deriveBreakdownAudit(model, snap.drillPath);
  return { values: new Map(rows.map(row => [row.key, row.value ?? ''])), ordered: true };
}

function parseBreakdownHit(hitKey: string | null): FamilyPointerHit | null {
  if (!hitKey) return null;
  if (hitKey.startsWith('back:')) return { key: '__back__', toggle: false };
  const toggle = hitKey.startsWith('toggle:');
  if (!toggle && !hitKey.startsWith('row:')) return null;
  const key = hitKey.slice(hitKey.indexOf(':') + 1);
  return key ? { key, toggle } : null;
}

export interface FamilyBreakdownHostProps<TOptions> {
  readonly declaration: BreakdownProjection<TOptions>;
  readonly options: TOptions;
  readonly view: BreakdownViewMode;
  readonly namesInRing: boolean;
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
    readonly endHour?: number;
  } | null;
  readonly exactReadView?: (hour: number) => Promise<BoundedReadView | null>;
  readonly offset: number;
  readonly pin: number | null;
  readonly bounds?: readonly [number, number] | null;
  readonly countryColours?: ReadonlyMap<string, string>;
  readonly countryLooks?: CountryLooksMap;
  readonly identitySnapshot?: IdentitySnapshot;
  readonly onIdentityDemands?: (demands: readonly (readonly [EntityKind, string, number])[]) => void;
  readonly selectedKey?: string | null;
  readonly onActivate?: (key: string, meta?: unknown, hour?: number) => void;
  readonly onSetOption?: (key: string, value: OptionValue) => void;
  readonly contextMenuItems?: (key: string | null, name: string | null, hour: number) => MenuItem[];
  readonly onDiagnostics?: (warnings: string[]) => void;
  readonly extraWarnings?: readonly string[];
  readonly blockedState?: ReactNode;
  readonly held?: boolean;
  readonly loadingLabel?: string;
  readonly className?: string;
}

interface BreakdownMetaState {
  mode: string;
  reason: string | null;
  failureOrigin: FailureOrigin;
  empty: string | null;
  emptyDetail: string | null;
  emptyKind: 'empty' | 'unavailable';
  emptyAction: BreakdownEmptyAction | null;
  warnings: readonly string[];
  footnote: string | null;
}

export default function FamilyBreakdownHost<TOptions>({
  declaration,
  options,
  view,
  namesInRing,
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
  identitySnapshot,
  onIdentityDemands,
  selectedKey = null,
  onActivate,
  onSetOption,
  contextMenuItems,
  onDiagnostics,
  extraWarnings,
  blockedState = null,
  held = false,
  loadingLabel,
  className,
}: FamilyBreakdownHostProps<TOptions>) {
  const prepared = useContext(PreparingPage);
  useWidgetSkeleton('sunburst');
  const viewportHeight = useViewportHeight();
  const contextMenu = useContextMenu(`${label} actions`);
  const contextMenuRef = useRef(contextMenu);
  contextMenuRef.current = contextMenu;

  const frameRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const accessibleRef = useRef<HTMLDivElement>(null);

  const viewportRef = useRef({ width: 0, height: 0, scrollTop: 0 });
  const [drillPath, setDrillPath] = useState<readonly string[]>([]);
  const drillPathRef = useRef(drillPath);
  drillPathRef.current = drillPath;
  const [userExpanded, setUserExpanded] = useState<Record<string, boolean>>({});
  const userExpandedRef = useRef(userExpanded);
  userExpandedRef.current = userExpanded;
  const [ringOpenKey, setRingOpenKey] = useState<string | null | undefined>(undefined);
  const ringOpenKeyRef = useRef(ringOpenKey);
  ringOpenKeyRef.current = ringOpenKey;

  const focusedKeyRef = useRef<string | null>(null);
  const focusInsideRef = useRef(false);
  const focusAnchorRef = useRef({ x: 0, y: 0 });
  const inspectedKeyRef = useRef<string | null>(null);
  const lastPointerRef = useRef<{ x: number; y: number } | null>(null);
  const heldRef = useRef(held);
  heldRef.current = held;
  const dimRef = useRef(false);
  const lastMirrorKeyRef = useRef('');
  const lastExtentHeightRef = useRef(-1);
  const navRef = useRef<readonly FamilyNavTuple[]>([]);
  const lastHourRef = useRef(staticSources?.hour ?? 0);
  const typeaheadRef = useRef<{ query: string; until: number }>({ query: '', until: 0 });

  const [meta, setMeta] = useState<BreakdownMetaState>(() => ({
    mode: staticSources ? 'picture' : 'reading',
    reason: null,
    failureOrigin: 'local',
    empty: null,
    emptyDetail: null,
    emptyKind: 'empty',
    emptyAction: null,
    warnings: [],
    footnote: null,
  }));
  const metaSigRef = useRef('');

  const { theme } = useFamilyTheme(countryColours, countryLooks);
  const { announce, hasFrame, liveRegionRef } = useFamilyAnnouncer('breakdown');

  const enabled = active && !prepared && !blockedState && (subscriptions.length > 0 || staticSources !== null || allowEmptyFollow);
  const auditable = !!exactReadView;

  const placement = useFamilyPlacement({
    family: 'breakdown',
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
    pin: pin ?? (subscriptions.length === 0 && staticSources && staticSources.endHour === undefined ? staticSources.hour : null),
    bounds,
    frameRef,
    surfaceLabel: 'breakdown',
  });
  const { pipeline, pipe, sourceIdsRef, subsReady, tooltipOwner, dataContextKey, retryStream, retryLocal, instanceId } = placement;

  const tooltips = useFamilyTooltip(tooltipOwner, dataContextKey);
  useFamilyHold(frameRef, held, tooltips);

  const audit = useFamilyAudit<BreakdownAuditSnapshot<TOptions>>({
    pipeline,
    pipe,
    ownerId: BREAKDOWN_AUDIT,
    auditable,
  });

  const reportShown = useShownHour();
  const reportShownRef = useRef(reportShown);
  reportShownRef.current = reportShown;

  const surfaceRef = useRef<MountedFamilyCanvas | null>(null);
  const latestRef = useRef({
    declaration,
    options,
    view,
    namesInRing,
    theme,
    staticSources,
    exactReadView,
    identitySnapshot,
    onIdentityDemands,
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
    view,
    namesInRing,
    theme,
    staticSources,
    exactReadView,
    identitySnapshot,
    onIdentityDemands,
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
    const { width, height, scrollTop } = viewportRef.current;
    const w = Math.max(1, width || 320);
    const h = Math.max(1, height || 220);
    const cur = latestRef.current;
    const staticPanels = cur.staticSources?.panels;
    const staticAnswers = cur.staticSources?.answers;
    let auditId: number | undefined;
    if (perfLog && cur.captureActive && cur.exactReadView) {
      auditId = audit.capture({
        declaration: cur.declaration,
        options: cur.options,
        drillPath: drillPathRef.current,
        exactReadView: cur.exactReadView,
      }, verifyBreakdownAuditSnapshot);
    }
    const bindingModel: FamilyBreakdownBinding<TOptions> = {
      family: 'breakdown',
      kind: cur.declaration.kind,
      view: cur.view,
      namesInRing: cur.namesInRing,
      options: cur.options,
      sources: sourceIdsRef.current,
      ...(staticPanels && Object.keys(staticPanels).length ? { staticPanels } : {}),
      ...(staticAnswers && Object.keys(staticAnswers).length ? { staticAnswers } : {}),
      width: w,
      height: h,
      scrollTop,
      theme: cur.theme,
      ...(cur.identitySnapshot ? { identitySnapshot: cur.identitySnapshot } : {}),
      drillPath: drillPathRef.current,
      userExpanded: userExpandedRef.current,
      ringOpenKey: ringOpenKeyRef.current,
      selectedKey: cur.selectedKey,
      focusedKey: focusInsideRef.current ? focusedKeyRef.current : null,
      ...(perfLog && cur.captureActive && auditId !== undefined ? { audit: true, auditId } : {}),
    };
    mounted.canvas.style.width = `${w}px`;
    mounted.canvas.style.height = `${h}px`;
    pipeline.present(pipe, 'breakdown');
    const dated = Object.keys(sourceIdsRef.current).length === 0
      ? (cur.staticSources
        ? { h: cur.staticSources.hour, c: cur.staticSources.endHour ?? cur.staticSources.hour }
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

  const togglePart = useCallback((key: string) => {
    const curView = latestRef.current.view;
    if (curView === 'ring') {
      setRingOpenKey(prev => (prev === key ? '' : key));
      announce('Toggled category details.');
      return;
    }
    const entry = navRef.current.find(item => item[0] === key);
    const nextOpen = entry ? entry[3] === 0 : true;
    setUserExpanded(prev => ({ ...prev, [key]: nextOpen }));
    if (entry) announce(`${entry[1]} ${nextOpen ? 'expanded' : 'collapsed'}.`);
  }, [announce]);
  const togglePartRef = useRef(togglePart);
  togglePartRef.current = togglePart;

  const activateHit = useCallback((key: string, isToggle: boolean) => {
    if (key === '__back__') {
      setDrillPath(prev => prev.slice(0, -1));
      announce('Returned to parent breakdown.');
      return;
    }
    if (key === '__partial__') return;
    if (isToggle) {
      togglePartRef.current(key);
      return;
    }
    const entry = navRef.current.find(item => item[0] === key);
    if (latestRef.current.view === 'ring' || latestRef.current.view === 'mosaic') {
      if (entry && entry[3] >= 0) {
        if (latestRef.current.view === 'ring') {
          setRingOpenKey(key);
        } else {
          setDrillPath(prev => [...prev, key]);
        }
        return;
      }
    } else if (entry && entry[3] >= 0) {
      togglePartRef.current(key);
      return;
    }
    latestRef.current.onActivate?.(key, entry?.[5], lastHourRef.current);
  }, [announce]);
  const activateHitRef = useRef(activateHit);
  activateHitRef.current = activateHit;

  const optionIdPrefix = useMemo(() => `bf-${instanceId}`, [instanceId]);

  useLayoutEffect(() => {
    const node = bodyRef.current;
    if (!node || !pipeline || pipe === null) return;
    lastExtentHeightRef.current = -1;
    lastMirrorKeyRef.current = '';

    const mounted = mountFamilyCanvas(node, pipeline, pipe, 'breakdown', 'bf-canvas', 'bf-extent');
    surfaceRef.current = mounted;

    const auditWatch = attachAuditWatch(pipeline, pipe, {
      isAuditable: () => !!latestRef.current.exactReadView,
      onCaptureChange: activeAudit => { latestRef.current.captureActive = activeAudit; },
      bindNow: () => bindRef.current(),
    });

    const install = () => {
      const shown = pipeline.view(pipe);
      const frameNode = frameRef.current;
      dimRef.current = !!shown?.dim;
      holdElement(frameNode, !!shown?.dim);
      const visible = shown?.mode === 'picture';
      node.toggleAttribute('data-breakdown-visible', visible);
      node.toggleAttribute('data-breakdown-pending', !visible);

      const mode = shown?.mode ?? 'reading';
      const reason = shown?.reason ?? null;
      const failureOrigin: FailureOrigin = shown?.failureOrigin ?? 'local';
      const shownHour = reportFamilyShownHour(reportShownRef.current, visible, shown);
      if (shownHour !== null) lastHourRef.current = shownHour;

      auditWatch.settleIfReady(visible, mode, shown?.installedBinding, shown?.fields);

      const rawMeta = shown?.fields?.meta as {
        empty?: string | null;
        emptyDetail?: string | null;
        emptyKind?: 'empty' | 'unavailable';
        emptyAction?: BreakdownEmptyAction | null;
        warnings?: readonly string[];
        footnote?: string | null;
        contentHeight?: number;
        bodyTop?: number;
      } | undefined;

      const sig = JSON.stringify([
        mode,
        reason,
        failureOrigin,
        rawMeta?.empty ?? null,
        rawMeta?.emptyDetail ?? null,
        rawMeta?.emptyKind ?? 'empty',
        rawMeta?.emptyAction ?? null,
        rawMeta?.warnings ?? [],
        rawMeta?.footnote ?? null,
      ]);
      if (sig !== metaSigRef.current) {
        metaSigRef.current = sig;
        setMeta({
          mode,
          reason,
          failureOrigin,
          empty: rawMeta?.empty ?? null,
          emptyDetail: rawMeta?.emptyDetail ?? null,
          emptyKind: rawMeta?.emptyKind ?? 'empty',
          emptyAction: rawMeta?.emptyAction ?? null,
          warnings: rawMeta?.warnings ?? [],
          footnote: rawMeta?.footnote ?? null,
        });
      }

      const demandsField = shown?.fields?.identityDemands as readonly (readonly [EntityKind, string, number])[] | undefined;
      if (demandsField && latestRef.current.onIdentityDemands) {
        latestRef.current.onIdentityDemands(demandsField);
      }

      if (!visible || !shown?.fields) {
        syncListboxMirror(accessibleRef.current, null, {
          optionIdPrefix,
          total: 0,
          focusedKey: null,
          lastMirrorKeyRef,
        });
        tooltips.hideIfUnpinned();
        return;
      }

      const extHeight = Math.max(
        viewportRef.current.height,
        (rawMeta?.bodyTop ?? 0) + (rawMeta?.contentHeight ?? viewportRef.current.height),
      );
      if (lastExtentHeightRef.current !== extHeight) {
        lastExtentHeightRef.current = extHeight;
        mounted.extent.style.height = `${extHeight}px`;
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
      accessibleRef,
      parseHit: parseBreakdownHit,
      isBlocked: () => heldRef.current || dimRef.current,
      getViewportHeight: () => latestRef.current.viewportHeight,
      onActivate: (key, toggle) => activateHitRef.current(key, toggle),
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
      viewportRef.current.scrollTop = node.scrollTop;
      bindRef.current();
    };

    const resizeObserver = new ResizeObserver(entries => {
      const rect = entries[0]?.contentRect;
      const w = Math.round(rect ? rect.width : node.clientWidth);
      const h = Math.round(rect ? rect.height : node.clientHeight);
      const box = frameRef.current?.getBoundingClientRect();
      if (box) focusAnchorRef.current = { x: box.left + Math.min(200, box.width / 2), y: box.top + 48 };
      if (w === viewportRef.current.width && h === viewportRef.current.height) return;
      viewportRef.current.width = w;
      viewportRef.current.height = h;
      bindRef.current();
    });
    resizeObserver.observe(node);
    viewportRef.current.width = node.clientWidth;
    viewportRef.current.height = node.clientHeight;
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
      holdElement(frameRef.current, false);
    };
  }, [pipeline, pipe, tooltips, optionIdPrefix]);

  useLayoutEffect(() => {
    if (pipe !== null && enabled) bind();
  }, [
    pipe,
    enabled,
    subsReady,
    declaration,
    options,
    view,
    namesInRing,
    theme,
    staticSources,
    identitySnapshot,
    drillPath,
    userExpanded,
    ringOpenKey,
    selectedKey,
    audit.captureActive,
    bind,
  ]);

  const warningsKey = JSON.stringify([...(meta.warnings ?? []), ...(extraWarnings ?? [])]);
  useEffect(() => {
    if (!onDiagnostics) return;
    const list = JSON.parse(warningsKey) as string[];
    onDiagnostics(list);
    return () => onDiagnostics([]);
  }, [warningsKey, onDiagnostics]);

  const onKeyDown = useCallback((event: React.KeyboardEvent<HTMLDivElement>) => {
    handleFamilyKeyDown(event, {
      nav: navRef.current,
      focusedKey: focusedKeyRef.current,
      pageStep: 8,
      blocked: heldRef.current || dimRef.current,
      typeaheadRef,
      isPinnedTooltip: () => tooltips.isPinned(),
      onFocusKey: key => {
        focusedKeyRef.current = key;
        bindRef.current();
      },
      onActivate: entry => activateHitRef.current(entry[0], false),
      onToggleTree: key => togglePartRef.current(key),
      onEscape: () => {
        if (drillPathRef.current.length > 0) {
          setDrillPath(prev => prev.slice(0, -1));
          announce('Returned to parent breakdown.');
        }
      },
      onTogglePinTooltip: key => {
        if (!key || !pipeline || pipe === null) return;
        const tipField = pipeline.view(pipe)?.fields?.focusTooltip as FamilyTooltipField | undefined;
        tooltips.pinFocused(tipField?.text ?? null, lastHourRef.current, focusAnchorRef.current, viewportHeight, announce);
      },
      onUnpinTooltip: () => tooltips.unpin(announce),
      onContextMenu: entry => {
        const builder = latestRef.current.contextMenuItems;
        if (!builder) return false;
        const items = builder(entry?.[0] ?? null, entry?.[1] ?? null, lastHourRef.current);
        if (!items.length) return false;
        const box = frameRef.current?.getBoundingClientRect();
        contextMenuRef.current.open(box ? box.left + 40 : 40, box ? box.top + 48 : 48, items);
        return true;
      },
    });
  }, [tooltips, pipeline, pipe, viewportHeight, announce]);

  if (blockedState) {
    return <div ref={frameRef} className={className ? `bf-frame ${className}` : 'bf-frame'}>{blockedState}</div>;
  }

  const bodyState = renderFamilyBodyState({
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
    onRetryLocal: retryLocal,
  });

  const actionButton = meta.empty && meta.emptyAction && onSetOption
    ? (
      <div className="bf-empty-wrap">
        <WidgetState
          kind={meta.emptyKind}
          message={meta.empty}
          detail={meta.emptyDetail ?? undefined}
        />
        <button
          type="button"
          className="button small"
          onClick={() => onSetOption(meta.emptyAction!.option, meta.emptyAction!.value)}
        >
          {meta.emptyAction.label}
        </button>
      </div>
    )
    : null;

  return (
    <div ref={frameRef} className={className ? `bf-frame ${className}` : 'bf-frame'}>
      <div className="bf-body-wrap">
        <div
          ref={bodyRef}
          className="bf-body"
          data-view={view}
          tabIndex={0}
          onFocus={() => {
            focusInsideRef.current = true;
            if (!focusedKeyRef.current && navRef.current.length) {
              focusedKeyRef.current = navRef.current[0]![0];
            }
            bindRef.current();
          }}
          onBlur={() => {
            focusInsideRef.current = false;
            if (!inspectedKeyRef.current) tooltips.hideIfUnpinned();
            bindRef.current();
          }}
          onKeyDown={onKeyDown}
        >
          <div
            ref={accessibleRef}
            className="bf-accessibility"
            role="listbox"
            aria-label={label}
          />
        </div>
        {(actionButton ?? bodyState) && (
          <div className="bf-state-overlay">{actionButton ?? bodyState}</div>
        )}
      </div>
      {!hasFrame && <div ref={liveRegionRef} className="bf-live-region" aria-live="polite" />}
      {contextMenu.menu}
    </div>
  );
}
