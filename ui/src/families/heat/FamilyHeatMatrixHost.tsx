import { useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { useContextMenu, type MenuItem } from '../../components/ContextMenu';
import { useViewportHeight } from '../../charts/useChartTokens';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { FailureOrigin } from '../../dashboard/follow/page.ts';
import type { WidgetSpec } from '../../dashboard/model';
import { PreparingPage } from '../../dashboard/preparing.ts';
import { useWidgetSkeleton, WidgetCaption } from '../../dashboard/widgetFrame.tsx';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { holdElement } from '../../draw/bitmapSurface.ts';
import { perfLog } from '../../shell/perf.ts';
import type { BoundedReadView, EntityKind, FamilySourceSubscription, MirrorModel } from '../contract.ts';
import {
  attachAuditWatch,
  bindFamilyPointer,
  createPlanAuditVerifier,
  mountFamilyCanvas,
  renderFamilyBodyState,
  reportFamilyShownHour,
  syncTableMirror,
  useFamilyAnnouncer,
  useFamilyAudit,
  useFamilyHold,
  useFamilyPauseFlush,
  useFamilyPlacement,
  useFamilyTheme,
  useFamilyTooltip,
  type FamilyTooltipField,
  type MountedFamilyCanvas,
  type PlanAuditSnapshot,
} from '../host/index.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import type { CountryLooksMap } from '../looks.ts';
import type { MatrixActivation } from './matrixDeclaration.ts';
import type { FamilyHeatMatrixBinding } from './matrixLaneView.ts';
import { matrixKeyStep, revealMatrixOffset } from './matrixLayout.ts';
import { deriveMatrixAudit, matrixCellKey, parseMatrixCellKey, type HeatMatrixModel, type HeatMatrixProjection, type MatrixAxisEntity } from './matrixModel.ts';
import './heatFamily.css';

const MATRIX_AUDIT = -2;
const verifyMatrixAudit = createPlanAuditVerifier<HeatMatrixModel, unknown>();

export interface FamilyHeatMatrixHostProps<TOptions> {
  readonly declaration: HeatMatrixProjection<TOptions>;
  readonly options: TOptions;
  readonly label: string;
  readonly subject: string;
  readonly widget: WidgetSpec;
  readonly runtime: WidgetRuntime;
  readonly active: boolean;
  readonly subscriptions: readonly FamilySourceSubscription[];
  readonly allowEmptyFollow?: boolean;
  readonly owner?: string;
  readonly staticSources?: { readonly panels?: Readonly<Record<string, unknown>>; readonly answers?: Readonly<Record<string, FollowedAnswer>>; readonly hour: number } | null;
  readonly exactReadView?: (hour: number) => Promise<BoundedReadView | null>;
  readonly offset: number;
  readonly pin: number | null;
  readonly bounds?: readonly [number, number] | null;
  readonly countryLooks?: CountryLooksMap;
  readonly identitySnapshot?: IdentitySnapshot | null;
  readonly onIdentityDemands?: (demands: readonly (readonly [EntityKind, string, number])[]) => void;
  readonly onActivate?: (activation: MatrixActivation) => void;
  readonly contextMenuItems?: (activation: MatrixActivation | null, hour: number) => MenuItem[];
  readonly onDiagnostics?: (warnings: string[]) => void;
  readonly extraWarnings?: readonly string[];
  readonly blockedState?: ReactNode;
  readonly held?: boolean;
  readonly loadingLabel?: string;
  readonly className?: string;
}

interface MatrixMetaState {
  mode: string;
  reason: string | null;
  failureOrigin: FailureOrigin;
  empty: string | null;
  emptyDetail: string | null;
  emptyKind: 'empty' | 'unavailable';
  notes: readonly string[];
  warnings: readonly string[];
}

interface MatrixExtentState {
  totalWidth: number;
  totalHeight: number;
  cellSize: number;
  gridLeft: number;
  gridTop: number;
  rows: readonly MatrixAxisEntity[];
  columns: readonly MatrixAxisEntity[];
}

function parseCellHit(hitKey: string | null): { readonly key: string; readonly toggle: boolean } | null {
  if (!hitKey || !hitKey.startsWith('cell:')) return null;
  const key = hitKey.slice(5);
  return key ? { key, toggle: false } : null;
}

export default function FamilyHeatMatrixHost<TOptions>({
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
  countryLooks,
  identitySnapshot = null,
  onIdentityDemands,
  onActivate,
  contextMenuItems,
  onDiagnostics,
  extraWarnings,
  blockedState = null,
  held = false,
  loadingLabel,
  className,
}: FamilyHeatMatrixHostProps<TOptions>) {
  const prepared = useContext(PreparingPage);
  useWidgetSkeleton('chart');
  const viewportHeight = useViewportHeight();
  const contextMenu = useContextMenu(`${label} actions`);
  const contextMenuRef = useRef(contextMenu);
  contextMenuRef.current = contextMenu;
  const frameRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const accessibleRef = useRef<HTMLTableElement>(null);
  const mirrorBodyRef = useRef<HTMLTableSectionElement>(null);

  const viewportRef = useRef({ width: 0, height: 0, scrollLeft: 0, scrollTop: 0 });
  const focusedKeyRef = useRef<string | null>(null);
  const focusInsideRef = useRef(false);
  const focusAnchorRef = useRef({ x: 0, y: 0 });
  const restoringFocusRef = useRef(false);
  const inspectedKeyRef = useRef<string | null>(null);
  const lastPointerRef = useRef<{ x: number; y: number } | null>(null);
  const heldRef = useRef(held);
  heldRef.current = held;
  const dimRef = useRef(false);
  const pauseFlushRef = useRef(0);
  const lastHourRef = useRef(staticSources?.hour ?? 0);
  const lastMirrorKeyRef = useRef('');
  const lastMirrorObjRef = useRef<MirrorModel | null>(null);
  const extentRef = useRef<MatrixExtentState>({
    totalWidth: 0,
    totalHeight: 0,
    cellSize: 28,
    gridLeft: 120,
    gridTop: 40,
    rows: [],
    columns: [],
  });
  const lastAuditRef = useRef<{ id: number; declaration: HeatMatrixProjection<TOptions>; options: TOptions; exactReadView: (hour: number) => Promise<BoundedReadView | null> } | null>(null);

  const [meta, setMeta] = useState<MatrixMetaState>(() => ({
    mode: staticSources ? 'picture' : 'reading',
    reason: null,
    failureOrigin: 'local',
    empty: null,
    emptyDetail: null,
    emptyKind: 'empty',
    notes: [],
    warnings: [],
  }));
  const metaSigRef = useRef('');

  const { theme } = useFamilyTheme(undefined, countryLooks);
  const { announce, hasFrame, liveRegionRef } = useFamilyAnnouncer('heat-matrix');

  const enabled = active && !prepared && !blockedState && (subscriptions.length > 0 || staticSources !== null || allowEmptyFollow);
  const auditable = !!exactReadView;
  const placement = useFamilyPlacement({
    family: 'heat-matrix',
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
    surfaceLabel: 'heat-matrix',
  });
  const { pipeline, pipe, sourceIdsRef, subsReady, tooltipOwner, dataContextKey, retryStream, retryLocal } = placement;

  const tooltips = useFamilyTooltip(tooltipOwner, dataContextKey);
  useFamilyHold(frameRef, held, tooltips);

  const audit = useFamilyAudit<PlanAuditSnapshot<HeatMatrixModel, unknown>>({
    pipeline,
    pipe,
    ownerId: MATRIX_AUDIT,
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
    const { width, height, scrollLeft: sl, scrollTop: st } = viewportRef.current;
    const w = Math.max(1, width || 320);
    const h = Math.max(1, height || 240);
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
          projection: cur.declaration as HeatMatrixProjection<unknown>,
          options: cur.options,
          exactReadView: cur.exactReadView,
          deriveRows: model => deriveMatrixAudit(model),
          ordered: false,
        }, verifyMatrixAudit);
        lastAuditRef.current = { id: auditId, declaration: cur.declaration, options: cur.options, exactReadView: cur.exactReadView };
      }
    }

    const bindingModel: FamilyHeatMatrixBinding<TOptions> = {
      family: 'heat-matrix',
      kind: cur.declaration.kind,
      options: cur.options,
      sources: sourceIdsRef.current,
      ...(staticPanels && Object.keys(staticPanels).length ? { staticPanels } : {}),
      ...(staticAnswers && Object.keys(staticAnswers).length ? { staticAnswers } : {}),
      width: w,
      height: h,
      scrollLeft: sl,
      scrollTop: st,
      theme: cur.theme,
      ...(cur.identitySnapshot ? { identitySnapshot: cur.identitySnapshot } : {}),
      focusedCell: focusInsideRef.current ? focusedKeyRef.current : null,
      pauseFlush: pauseFlushRef.current,
      ...(perfLog && cur.captureActive && auditId !== undefined ? { audit: true, auditId } : {}),
    };

    mounted.canvas.style.width = `${w}px`;
    mounted.canvas.style.height = `${h}px`;
    pipeline.present(pipe, 'heat-matrix');
    const dated = Object.keys(sourceIdsRef.current).length === 0
      ? (cur.staticSources
        ? { h: cur.staticSources.hour, c: cur.staticSources.hour }
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

  const buildActivation = useCallback((key: string | null): MatrixActivation | null => {
    const parsed = parseMatrixCellKey(key);
    if (!parsed) return null;
    const ext = extentRef.current;
    const row = ext.rows[parsed.row];
    const column = ext.columns[parsed.column];
    if (!row || !column) return null;
    return { row, column, rowIndex: parsed.row, columnIndex: parsed.column, hour: lastHourRef.current };
  }, []);

  useLayoutEffect(() => {
    const node = bodyRef.current;
    if (!node || !pipeline || pipe === null) return;
    lastMirrorObjRef.current = null;

    const mounted = mountFamilyCanvas(node, pipeline, pipe, 'heat-matrix', 'hf-canvas', 'hf-extent');
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
      node.toggleAttribute('data-heat-visible', visible);
      node.toggleAttribute('data-heat-pending', !visible);

      const mode = shown?.mode ?? 'reading';
      const reason = shown?.reason ?? null;
      const failureOrigin: FailureOrigin = shown?.failureOrigin ?? 'local';
      const shownHour = reportFamilyShownHour(reportShownRef.current, visible, shown);
      if (shownHour !== null) lastHourRef.current = shownHour;

      auditWatch.settleIfReady(visible, mode, shown?.installedBinding, shown?.fields);

      const rawMeta = shown?.fields?.meta as Omit<MatrixMetaState, 'mode' | 'reason' | 'failureOrigin'> | undefined;
      const nextMetaSig = JSON.stringify([mode, reason, failureOrigin, rawMeta?.empty ?? null, rawMeta?.emptyDetail ?? null, rawMeta?.emptyKind ?? 'empty', rawMeta?.notes ?? [], rawMeta?.warnings ?? []]);
      if (nextMetaSig !== metaSigRef.current) {
        metaSigRef.current = nextMetaSig;
        setMeta({
          mode,
          reason,
          failureOrigin,
          empty: rawMeta?.empty ?? null,
          emptyDetail: rawMeta?.emptyDetail ?? null,
          emptyKind: rawMeta?.emptyKind ?? 'empty',
          notes: rawMeta?.notes ?? [],
          warnings: rawMeta?.warnings ?? [],
        });
      }

      const demandsField = shown?.fields?.identityDemands as readonly (readonly [EntityKind, string, number])[] | undefined;
      if (demandsField && latestRef.current.onIdentityDemands) {
        latestRef.current.onIdentityDemands(demandsField);
      }

      if (!visible || !shown?.fields) {
        lastMirrorObjRef.current = null;
        syncTableMirror(mirrorBodyRef.current, null, {
          selectedKey: null,
          focusedKeyRef,
          focusInsideRef,
          restoringFocusRef,
          lastMirrorKeyRef,
        });
        tooltips.hideIfUnpinned();
        return;
      }

      const ext = shown.fields.extent as MatrixExtentState | undefined;
      if (ext) {
        extentRef.current = ext;
        mounted.extent.style.width = `${Math.max(viewportRef.current.width, ext.totalWidth)}px`;
        mounted.extent.style.height = `${Math.max(viewportRef.current.height, ext.totalHeight)}px`;
      }

      const mirror = shown.fields.mirror as MirrorModel | undefined;
      if (mirror && mirror !== lastMirrorObjRef.current) {
        lastMirrorObjRef.current = mirror;
        syncTableMirror(mirrorBodyRef.current, mirror, {
          selectedKey: null,
          focusedKeyRef,
          focusInsideRef,
          restoringFocusRef,
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
      isBlocked: () => heldRef.current || dimRef.current,
      getViewportHeight: () => latestRef.current.viewportHeight,
      parseHit: parseCellHit,
      inspectToken: key => `cell:${key}`,
      onActivate: key => {
        const activation = buildActivation(key);
        if (activation) latestRef.current.onActivate?.(activation);
      },
      onContextMenu: (key, clientX, clientY) => {
        const builder = latestRef.current.contextMenuItems;
        if (!builder) return false;
        const items = builder(buildActivation(key), lastHourRef.current);
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
  }, [pipeline, pipe, tooltips, buildActivation]);

  useLayoutEffect(() => {
    if (pipe !== null && enabled) bind();
  }, [pipe, enabled, bind, options, theme, staticSources, identitySnapshot, subsReady, auditable, exactReadView, audit.captureActive]);

  useFamilyPauseFlush(runtime.clock, () => {
    if (!pipeline || pipe === null) return;
    pauseFlushRef.current += 1;
    bindRef.current();
  });

  const combinedWarnings = useMemo(() => {
    const out = [...meta.warnings, ...(extraWarnings ?? [])];
    return out;
  }, [meta.warnings, extraWarnings]);

  useEffect(() => {
    onDiagnostics?.(combinedWarnings);
  }, [onDiagnostics, combinedWarnings]);

  const onKeyDown = useCallback((event: KeyboardEvent<HTMLDivElement>) => {
    const ext = extentRef.current;
    if (!ext.rows.length || !ext.columns.length) return;
    const current = parseMatrixCellKey(focusedKeyRef.current) ?? { row: 0, column: 0 };
    if (event.key === 'Enter') {
      event.preventDefault();
      const activation = buildActivation(matrixCellKey(current.row, current.column));
      if (activation) latestRef.current.onActivate?.(activation);
      return;
    }
    if (event.key === 'Escape') {
      if (tooltips.isPinned()) {
        event.preventDefault();
        tooltips.clearPinAndHide();
      }
      return;
    }
    const pageStep = Math.max(1, Math.floor(Math.max(1, viewportRef.current.height - ext.gridTop) / ext.cellSize));
    const next = matrixKeyStep(event.key, current, ext.rows.length, ext.columns.length, pageStep);
    if (!next) return;
    event.preventDefault();
    const key = matrixCellKey(next.row, next.column);
    focusedKeyRef.current = key;
    const node = bodyRef.current;
    if (node) {
      const nextLeft = revealMatrixOffset(next.column, ext.cellSize, node.scrollLeft, Math.max(1, viewportRef.current.width - ext.gridLeft));
      const nextTop = revealMatrixOffset(next.row, ext.cellSize, node.scrollTop, Math.max(1, viewportRef.current.height - ext.gridTop));
      if (nextLeft !== node.scrollLeft) {
        node.scrollLeft = nextLeft;
        viewportRef.current.scrollLeft = nextLeft;
      }
      if (nextTop !== node.scrollTop) {
        node.scrollTop = nextTop;
        viewportRef.current.scrollTop = nextTop;
      }
    }
    const rowLabel = ext.rows[next.row]?.label ?? '';
    const colLabel = ext.columns[next.column]?.label ?? '';
    announce(`${rowLabel} to ${colLabel}`);
    bindRef.current();
  }, [announce, buildActivation, tooltips]);

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

  return <div ref={frameRef} className={`hf-frame hf-matrix${className ? ` ${className}` : ''}`}>
    {meta.notes.length > 0 && <WidgetCaption>{meta.notes.join(' · ')}</WidgetCaption>}
    <div
      ref={bodyRef}
      className="hf-body"
      tabIndex={0}
      role="grid"
      aria-label={label}
      onFocus={() => {
        focusInsideRef.current = true;
        if (!focusedKeyRef.current && extentRef.current.rows.length && extentRef.current.columns.length) {
          focusedKeyRef.current = matrixCellKey(0, 0);
        }
        bindRef.current();
      }}
      onBlur={() => {
        focusInsideRef.current = false;
        tooltips.hideIfUnpinned();
        bindRef.current();
      }}
      onKeyDown={onKeyDown}
    >
      {bodyState && <div className="hf-overlay">{bodyState}</div>}
      <table ref={accessibleRef} className="hf-sr-only" aria-label={label}>
        <tbody ref={mirrorBodyRef} />
      </table>
    </div>
    {!hasFrame && <div ref={liveRegionRef} className="hf-sr-only" role="status" aria-live="polite" />}
    {contextMenu.menu}
  </div>;
}
