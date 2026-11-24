import { useCallback, useContext, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { useContextMenu, type MenuItem } from '../../components/ContextMenu';
import { useViewportHeight } from '../../charts/useChartTokens';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { FailureOrigin } from '../../dashboard/follow/page.ts';
import type { WidgetSpec } from '../../dashboard/model';
import { PreparingPage } from '../../dashboard/preparing.ts';
import { useWidgetSkeleton, WidgetCaption } from '../../dashboard/widgetFrame.tsx';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { holdElement } from '../../draw/bitmapSurface.ts';
import { perfLog } from '../../shell/perf.ts';
import type { BoundedReadView, EntityKind, FamilySourceSubscription, MirrorModel } from '../contract.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
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
  useFamilyPauseFlush,
  useFamilyPlacement,
  useFamilyTheme,
  useFamilyTooltip,
  type FamilyNavTuple,
  type FamilyTooltipField,
  type MountedFamilyCanvas,
} from '../host/index.ts';
import type { StaticBundle } from '../host/useFamilyPlan.ts';
import type { CountryLooksMap } from '../looks.ts';
import type { FamilyBarsBinding } from './laneView.ts';
import { deriveBarsAudit, type BarsProjection } from './model.ts';
import './barsFamily.css';

const BARS_AUDIT = -2;

interface BarsAuditSnapshot<TOptions> {
  readonly declaration: BarsProjection<TOptions>;
  readonly options: TOptions;
  readonly exactReadView: (hour: number) => Promise<BoundedReadView | null>;
}

async function verifyBarsAuditSnapshot<TOptions>(hour: number, snap: BarsAuditSnapshot<TOptions>) {
  const readView = await snap.exactReadView(hour);
  if (!readView) return null;
  const model = snap.declaration.project(readView, snap.options);
  const rows = deriveBarsAudit(model);
  return { values: new Map(rows.map(row => [row.key, row.value ?? ''])), ordered: true };
}

interface BarsMetaState {
  readonly mode: string;
  readonly reason: string | null;
  readonly failureOrigin: FailureOrigin;
  readonly empty: string | null;
  readonly emptyDetail: string | null;
  readonly emptyKind: 'empty' | 'unavailable';
  readonly summaryText: string | null;
  readonly notes: readonly string[];
  readonly warnings: readonly string[];
  readonly count: number;
  readonly totalHeight: number;
  readonly bodyHeight: number;
}

export interface FamilyBarsHostProps<TOptions> {
  readonly declaration: BarsProjection<TOptions>;
  readonly options: TOptions;
  readonly label: string;
  readonly subject: string;
  readonly widget: WidgetSpec;
  readonly runtime: WidgetRuntime;
  readonly active: boolean;
  readonly subscriptions: readonly FamilySourceSubscription[];
  readonly allowEmptyFollow?: boolean;
  readonly owner?: string;
  readonly staticBundle?: StaticBundle | null;
  readonly exactReadView?: (hour: number) => Promise<BoundedReadView | null>;
  readonly offset: number;
  readonly pin: number | null;
  readonly bounds?: readonly [number, number] | null;
  readonly countryColours?: ReadonlyMap<string, string>;
  readonly countryLooks?: CountryLooksMap;
  readonly identitySnapshot?: IdentitySnapshot | null;
  readonly onIdentityDemands?: (demands: readonly (readonly [EntityKind, string, number])[]) => void;
  readonly onViewportSize?: (width: number, height: number) => void;
  readonly selectedKey?: string | null;
  readonly onActivate?: (key: string, meta?: unknown, hour?: number) => void;
  readonly onAddRegion?: () => void;
  readonly onRemoveRegion?: (key: string) => void;
  readonly contextMenuItems?: (key: string | null, name: string | null, hour: number) => MenuItem[];
  readonly headerSlot?: HTMLElement | null;
  readonly onDiagnostics?: (warnings: string[]) => void;
  readonly extraWarnings?: readonly string[];
  readonly blockedState?: ReactNode;
  readonly held?: boolean;
  readonly loadingLabel?: string;
  readonly className?: string;
}

export default function FamilyBarsHost<TOptions>({
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
  staticBundle = null,
  exactReadView,
  offset,
  pin,
  bounds = null,
  countryColours,
  countryLooks,
  identitySnapshot = null,
  onIdentityDemands,
  onViewportSize,
  selectedKey = null,
  onActivate,
  onAddRegion,
  onRemoveRegion,
  contextMenuItems,
  headerSlot = null,
  onDiagnostics,
  extraWarnings,
  blockedState = null,
  held = false,
  loadingLabel,
  className = '',
}: FamilyBarsHostProps<TOptions>) {
  const prepared = useContext(PreparingPage);
  useWidgetSkeleton('chart');
  const viewportHeight = useViewportHeight();
  const contextMenu = useContextMenu(`${label} actions`);
  const contextMenuRef = useRef(contextMenu);
  contextMenuRef.current = contextMenu;

  const frameRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const mirrorRef = useRef<HTMLDivElement>(null);
  const surfaceRef = useRef<MountedFamilyCanvas | null>(null);

  const viewportRef = useRef({ width: 0, height: 0, scrollTop: 0 });
  const focusedKeyRef = useRef<string | null>(null);
  const focusInsideRef = useRef(false);
  const focusAnchorRef = useRef({ x: 0, y: 0 });
  const inspectedKeyRef = useRef<string | null>(null);
  const lastPointerRef = useRef<{ x: number; y: number } | null>(null);
  const heldRef = useRef(held);
  heldRef.current = held;
  const dimRef = useRef(false);
  const pauseFlushRef = useRef(0);
  const lastHourRef = useRef(staticBundle?.hour ?? 0);
  const lastMirrorKeyRef = useRef('');
  const navRef = useRef<readonly FamilyNavTuple[]>([]);
  const typeaheadRef = useRef({ query: '', until: 0 });
  const metaSigRef = useRef('');

  const [meta, setMeta] = useState<BarsMetaState>(() => ({
    mode: staticBundle ? 'picture' : 'reading',
    reason: null,
    failureOrigin: 'local',
    empty: null,
    emptyDetail: null,
    emptyKind: 'empty',
    summaryText: null,
    notes: [],
    warnings: [],
    count: 0,
    totalHeight: 0,
    bodyHeight: 0,
  }));

  const { theme, reducedMotion } = useFamilyTheme(countryColours, countryLooks);
  const { announce, hasFrame, liveRegionRef } = useFamilyAnnouncer('bars');

  const enabled = active && !prepared && !blockedState && (subscriptions.length > 0 || staticBundle !== null || allowEmptyFollow);
  const auditable = !!exactReadView;
  const placement = useFamilyPlacement({
    family: 'bars',
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
    pin: pin ?? (subscriptions.length === 0 && staticBundle && !staticBundle.follows ? staticBundle.hour : null),
    bounds,
    frameRef,
    surfaceLabel: 'bars',
  });
  const { pipeline, pipe, sourceIdsRef, subsReady, instanceId, tooltipOwner, dataContextKey, retryStream, retryLocal } = placement;

  const tooltips = useFamilyTooltip(tooltipOwner, dataContextKey);
  useFamilyHold(frameRef, held, tooltips);

  const audit = useFamilyAudit<BarsAuditSnapshot<TOptions>>({
    pipeline,
    pipe,
    ownerId: BARS_AUDIT,
    auditable,
  });

  const reportShown = useShownHour();
  const reportShownRef = useRef(reportShown);
  reportShownRef.current = reportShown;

  const latestRef = useRef({
    declaration,
    options,
    theme,
    reducedMotion,
    staticBundle,
    exactReadView,
    identitySnapshot,
    onIdentityDemands,
    onViewportSize,
    selectedKey,
    onActivate,
    onAddRegion,
    onRemoveRegion,
    contextMenuItems,
    viewportHeight,
    captureActive: audit.captureActive,
  });
  latestRef.current = {
    declaration,
    options,
    theme,
    reducedMotion,
    staticBundle,
    exactReadView,
    identitySnapshot,
    onIdentityDemands,
    onViewportSize,
    selectedKey,
    onActivate,
    onAddRegion,
    onRemoveRegion,
    contextMenuItems,
    viewportHeight,
    captureActive: audit.captureActive,
  };

  useEffect(() => {
    const all = [...meta.warnings, ...(extraWarnings ?? [])];
    onDiagnostics?.(all);
  }, [meta.warnings, extraWarnings, onDiagnostics]);

  const bind = useCallback(() => {
    const mounted = surfaceRef.current;
    if (!pipeline || pipe === null || !mounted) return;
    const latest = latestRef.current;
    const w = Math.max(1, viewportRef.current.width);
    const h = Math.max(1, viewportRef.current.height);
    if (!latest.theme) return;

    let auditId: number | undefined;
    if (perfLog && latest.captureActive && latest.exactReadView) {
      auditId = audit.capture(
        { declaration: latest.declaration, options: latest.options, exactReadView: latest.exactReadView },
        verifyBarsAuditSnapshot,
      );
    }

    const bindingModel: FamilyBarsBinding<TOptions> = {
      family: 'bars',
      kind: latest.declaration.kind,
      options: latest.options,
      sources: { ...sourceIdsRef.current },
      ...(latest.staticBundle ? { staticPanels: latest.staticBundle.panels, staticAnswers: latest.staticBundle.answers } : {}),
      width: w,
      height: h,
      scrollTop: viewportRef.current.scrollTop,
      theme: latest.theme,
      ...(latest.identitySnapshot ? { identitySnapshot: latest.identitySnapshot } : {}),
      selectedKey: latest.selectedKey,
      focusedKey: focusedKeyRef.current,
      reducedMotion: latest.reducedMotion,
      audit: auditId !== undefined,
      ...(auditId !== undefined ? { auditId } : {}),
      pauseFlush: pauseFlushRef.current,
    };

    const dated = latest.staticBundle
      ? (Object.keys(latest.staticBundle.panels).length > 0 || Object.keys(latest.staticBundle.answers ?? {}).length > 0
          ? (latest.staticBundle.provenInterval
              ? { h: latest.staticBundle.provenInterval[0], c: latest.staticBundle.provenInterval[1] }
              : { h: latest.staticBundle.hour, c: latest.staticBundle.hour })
          : undefined)
      : undefined;

    pipeline.bind(
      pipe,
      [mounted.surface],
      {
        surfaces: [{ width: w, height: h, dpr: typeof devicePixelRatio === 'number' ? devicePixelRatio : 1 }],
        ...(dated !== undefined ? { dated } : {}),
      },
      {},
      bindingModel,
    );
  }, [pipeline, pipe, audit.capture, sourceIdsRef]);

  const bindRef = useRef(bind);
  bindRef.current = bind;

  useFamilyPauseFlush(runtime.clock, () => {
    pauseFlushRef.current++;
    bindRef.current();
  });

  useLayoutEffect(() => {
    const node = bodyRef.current;
    if (!node || !pipeline || pipe === null) return;
    const mounted = mountFamilyCanvas(node, pipeline, pipe, 'bars', 'bars-family-canvas', 'bars-family-extent');
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
      const mode = shown?.mode ?? 'reading';
      const reason = shown?.reason ?? null;
      const failureOrigin: FailureOrigin = shown?.failureOrigin ?? 'local';
      const shownHour = reportFamilyShownHour(reportShownRef.current, visible, shown);
      if (shownHour !== null) lastHourRef.current = shownHour;

      auditWatch.settleIfReady(visible, mode, shown?.installedBinding, shown?.fields);

      const rawMeta = shown?.fields?.meta as Partial<BarsMetaState> | undefined;
      const nextMeta: BarsMetaState = {
        mode,
        reason,
        failureOrigin,
        empty: rawMeta?.empty ?? null,
        emptyDetail: rawMeta?.emptyDetail ?? null,
        emptyKind: rawMeta?.emptyKind ?? 'empty',
        summaryText: rawMeta?.summaryText ?? null,
        notes: rawMeta?.notes ?? [],
        warnings: rawMeta?.warnings ?? [],
        count: rawMeta?.count ?? 0,
        totalHeight: rawMeta?.totalHeight ?? 0,
        bodyHeight: rawMeta?.bodyHeight ?? 0,
      };
      const nextSig = JSON.stringify(nextMeta);
      if (nextSig !== metaSigRef.current) {
        metaSigRef.current = nextSig;
        setMeta(nextMeta);
      }
      const demandsField = shown?.fields?.identityDemands as readonly (readonly [EntityKind, string, number])[] | undefined;
      if (demandsField && latestRef.current.onIdentityDemands) {
        latestRef.current.onIdentityDemands(demandsField);
      }

      if (!visible || !shown?.fields) {
        tooltips.hideIfUnpinned();
        return;
      }

      mounted.extent.style.height = `${Math.max(viewportRef.current.height, nextMeta.totalHeight)}px`;

      const nextNav = (shown.fields.nav as readonly FamilyNavTuple[] | undefined) ?? [];
      navRef.current = nextNav;

      const nextMirror = shown.fields.mirror as MirrorModel | undefined;
      if (nextMirror && mirrorRef.current) {
        syncListboxMirror(
          mirrorRef.current,
          nextMirror.rows.map(row => ({
            key: row.key,
            spoken: [row.name, ...row.cells.map(c => c.spoken)].filter(Boolean).join(', '),
            index: row.rowIndex,
            selected: latestRef.current.selectedKey === row.key,
          })),
          {
            optionIdPrefix: `bars-${instanceId}`,
            total: nextMirror.total,
            focusedKey: focusedKeyRef.current,
            lastMirrorKeyRef,
          },
        );
      }

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
      accessibleRef: mirrorRef,
      isBlocked: () => heldRef.current || dimRef.current,
      getViewportHeight: () => latestRef.current.viewportHeight,
      parseHit: hitKey => {
        if (!hitKey) return null;
        if (hitKey.startsWith('add:')) return { key: '__add-region', toggle: false };
        if (hitKey.startsWith('remove:')) return { key: hitKey.slice(7), toggle: true };
        if (hitKey.startsWith('right:')) return { key: hitKey.slice(6), toggle: false };
        if (hitKey.startsWith('row:')) return { key: hitKey.slice(4), toggle: false };
        return null;
      },
      onActivate: (rowKey, isToggle) => {
        if (rowKey === '__add-region') {
          latestRef.current.onAddRegion?.();
          return;
        }
        if (isToggle) {
          latestRef.current.onRemoveRegion?.(rowKey);
          return;
        }
        const entry = navRef.current.find(item => item[0] === rowKey);
        latestRef.current.onActivate?.(rowKey, entry?.[5], lastHourRef.current);
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

    const onScroll = () => {
      const st = Math.floor(node.scrollTop);
      if (st === viewportRef.current.scrollTop) return;
      viewportRef.current.scrollTop = st;
      bindRef.current();
    };
    node.addEventListener('scroll', onScroll, { passive: true });

    const observer = typeof ResizeObserver === 'function' ? new ResizeObserver(entries => {
      const rect = entries[0]?.contentRect;
      const rawW = rect ? rect.width : node.clientWidth;
      const rawH = rect ? rect.height : node.clientHeight;
      latestRef.current.onViewportSize?.(rawW, rawH);
      const w = Math.round(rawW);
      const h = Math.round(rawH);
      const box = frameRef.current?.getBoundingClientRect();
      if (box) focusAnchorRef.current = { x: box.left + Math.min(200, box.width / 2), y: box.top + 36 };
      if (w === viewportRef.current.width && h === viewportRef.current.height) return;
      viewportRef.current.width = w;
      viewportRef.current.height = h;
      bindRef.current();
    }) : null;
    observer?.observe(node);
    viewportRef.current.width = node.clientWidth;
    viewportRef.current.height = node.clientHeight;
    latestRef.current.onViewportSize?.(node.clientWidth, node.clientHeight);
    bindRef.current();

    return () => {
      observer?.disconnect();
      node.removeEventListener('scroll', onScroll);
      stopPointer();
      stopInstall();
      auditWatch.dispose();
      mounted.unmount();
      surfaceRef.current = null;
      reportFamilyShownHour(reportShownRef.current, false, null);
    };
  }, [pipeline, pipe, instanceId, tooltips]);

  useEffect(() => {
    bindRef.current();
  }, [declaration, options, theme, reducedMotion, staticBundle, identitySnapshot, selectedKey, subsReady, audit.captureActive]);

  const onKeyDown = useCallback((event: React.KeyboardEvent<HTMLDivElement>) => {
    handleFamilyKeyDown(event, {
      nav: navRef.current,
      focusedKey: focusedKeyRef.current,
      pageStep: Math.max(1, Math.floor(viewportRef.current.height / 28)),
      blocked: heldRef.current || dimRef.current,
      typeaheadRef,
      isPinnedTooltip: () => tooltips.isPinned(),
      onFocusKey: key => {
        focusedKeyRef.current = key;
        bindRef.current();
      },
      onActivate: entry => {
        if (entry[0] === '__add-region') {
          latestRef.current.onAddRegion?.();
          return;
        }
        latestRef.current.onActivate?.(entry[0], entry[5], lastHourRef.current);
      },
      onUnpinTooltip: () => tooltips.unpin(announce),
    });
  }, [tooltips, announce]);

  const bodyState = blockedState ?? renderFamilyBodyState({
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
    onRetryLocal: () => retryLocal(),
  });

  return (
    <div ref={frameRef} className={`bars-family ${className}`.trim()}>
      {headerSlot && meta.summaryText ? createPortal(<span className="bars-family-summary">{meta.summaryText}</span>, headerSlot) : null}
      {bodyState}
      <div
        ref={bodyRef}
        className="bars-family-viewport"
        style={bodyState ? { display: 'none' } : undefined}
        tabIndex={0}
        role="listbox"
        aria-label={label}
        onFocus={() => { focusInsideRef.current = true; }}
        onBlur={() => {
          focusInsideRef.current = false;
          tooltips.hideIfUnpinned();
        }}
        onKeyDown={onKeyDown}
      >
        <div ref={mirrorRef} className="bars-family-mirror" />
      </div>
      {meta.notes.length > 0 ? <WidgetCaption>{meta.notes.join(' ')}</WidgetCaption> : null}
      {!hasFrame ? <div ref={liveRegionRef} className="bars-family-mirror" aria-live="polite" /> : null}
      {contextMenu.menu}
    </div>
  );
}
