import { useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type ComponentType, type FocusEvent as ReactFocusEvent, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { useContextMenu, type MenuItem } from '../../components/ContextMenu';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { FailureOrigin } from '../../dashboard/follow/page.ts';
import type { WidgetBase } from '../../dashboard/model';
import { PreparingPage } from '../../dashboard/preparing.ts';
import type { TiledLine } from '../../dashboard/tiles/view';
import { useWidgetSkeleton, WidgetCaption } from '../../dashboard/widgetFrame.tsx';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { holdElement } from '../../draw/bitmapSurface.ts';
import { perfLog } from '../../shell/perf.ts';
import type { ArtResolution } from '../art/types.ts';
import { auditIdOf } from '../audit.ts';
import { displayedHour } from '../boundedView.ts';
import type {
  BoundedReadView,
  EntityKind,
  FamilyProjection,
  FamilySourceSubscription,
  InstalledAction,
  InstalledPicture,
  InstalledTransform,
  MirrorModel,
  SourcePlan,
} from '../contract.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import type { CountryLooksMap } from '../looks.ts';
import type { PagedRows } from '../table/rowSource.ts';
import {
  familyBinding,
  familyInspectToken,
  lazyActionKeys,
  parseFamilyHit,
  requestedRowWindow,
  type FamilyArtDemand,
  type FamilyContextHit,
  type FamilyGestureContext,
  type FamilyHostAdapter,
  type FamilyMetaState,
  type FamilyRowWindow,
  type FamilyStaticProof,
  type FamilyStaticSources,
  type FamilyViewportChange,
  type FamilyViewportState,
  type ParsedFamilyHit,
} from './adapter.ts';
import { createPlanAuditVerifier, type FamilyAuditVerifier, type PlanAuditSnapshot } from './auditBinding.ts';
import { boundDated, ownProof, StaticSupply } from './dated.ts';
import { renderFamilyBodyState } from './FamilyBodyState.tsx';
import {
  handleFamilyGridKeyDown,
  handleFamilyKeyDown,
  handleFamilyPointsKeyDown,
  navPageSteps,
  usableRowHeight,
  type FamilyNavTuple,
} from './keyboard.ts';
import { NO_EXPANSION, type FamilyExpansion } from './expansion.ts';
import { BoundSelections, inputBlocked, installedRecordOf, recordInstalled, resolveHostInput, type HostCommand, type HostInput, type HostInputGate, type InstalledRecord } from './installed.ts';
import { mirrorRowSelector, mirrorToSync, syncListboxMirror, syncTableMirror, type ListboxMirrorItem } from './mirror.ts';
import { bindFamilyPointer } from './pointer.ts';
import { regionDropAt, regionRowOf, type RegionCommand, type RegionRowRef, type RegionRowsField } from '../regions/regionRows.ts';
import { computeScrollGeometry, physicalToLogicalTop, scrollRangeIntoView, type ScrollGeometry } from './scroll.ts';
import { reportFamilyShownHour } from './shownHour.ts';
import {
  attachAuditWatch,
  mountFamilyCanvas,
  useFamilyAnnouncer,
  useFamilyAudit,
  useFamilyHold,
  useFamilyPauseFlush,
  useFamilyPlacement,
  useFamilyTooltip,
  type MountedFamilyCanvas,
} from './surface.ts';
import { useFamilyTheme } from './theme.ts';
import type { FamilyTooltipField } from './tooltips.ts';
import './familyHost.css';

const DEFAULT_AUDIT_OWNER = -1;
const DEFAULT_POINTS_PAGE = 10;
const PLAN_VERIFIER = createPlanAuditVerifier<unknown, unknown>() as FamilyAuditVerifier<unknown>;
const EMPTY_NAV: readonly FamilyNavTuple[] = [];
const EMPTY_FIELDS: Readonly<Record<string, unknown>> = {};
const NO_STRINGS: readonly string[] = [];

interface RawMeta {
  readonly empty?: string | null;
  readonly emptyDetail?: string | null;
  readonly emptyKind?: 'empty' | 'unavailable';
  readonly summaryText?: string | null;
  readonly summaryPartial?: string | null;
  readonly notes?: readonly string[];
  readonly warnings?: readonly string[];
  readonly transform?: InstalledTransform;
}

function sameStrings(a: readonly string[], b: readonly string[]): boolean {
  if (a === b) return true;
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

function parseGridKey(key: string | null): { readonly row: number; readonly column: number } | null {
  if (!key) return null;
  const idx = key.indexOf(':');
  if (idx <= 0) return null;
  const row = Number(key.slice(0, idx));
  const column = Number(key.slice(idx + 1));
  return Number.isInteger(row) && Number.isInteger(column) && row >= 0 && column >= 0 ? { row, column } : null;
}

function formatGridKey(row: number, column: number): string {
  return `${row}:${column}`;
}

function revealGridAxis(index: number, cellSize: number, scroll: number, span: number): number {
  const start = index * cellSize;
  const end = start + cellSize;
  if (start < scroll) return start;
  if (end > scroll + span) return Math.max(0, end - span);
  return scroll;
}

function listboxItems(mirror: MirrorModel, selectedKey: string | null): readonly ListboxMirrorItem[] {
  return mirror.rows.map(row => ({
    key: row.key,
    spoken: [row.name, ...row.cells.map(c => c.spoken)].filter(Boolean).join(', '),
    index: row.rowIndex,
    selected: selectedKey === row.key,
  }));
}

interface HostInputAt extends HostInput {
  readonly at?: { readonly x: number; readonly y: number };
}

function currentDpr(): number {
  return typeof devicePixelRatio === 'number' ? devicePixelRatio : 1;
}

export interface FamilyChromeTargets {
  readonly body: HTMLElement | null;
  readonly bodyWrap: HTMLElement | null;
  readonly extent: HTMLElement | null;
  readonly mirrorHead: HTMLTableSectionElement | null;
}

export interface FamilyChromeProps<TSettings = unknown> {
  readonly state: unknown;
  readonly meta: FamilyMetaState;
  readonly settings: TSettings | undefined;
  readonly onSettings?: (next: TSettings) => void;
  readonly editable: boolean;
  readonly viewportWidth: number;
  readonly bodyBlocked: boolean;
  readonly announce: (message: string) => void;
  readonly targets: FamilyChromeTargets;
  readonly headerSlot: HTMLElement | null;
}

const NO_TARGETS: FamilyChromeTargets = { body: null, bodyWrap: null, extent: null, mirrorHead: null };

export interface FamilyHostComponentProps<TModel = unknown, TOptions = unknown, TBinding = unknown, TSettings = unknown> {
  readonly adapter: FamilyHostAdapter<TModel, TOptions, TBinding, TSettings>;
  readonly settings?: TSettings;
  readonly onSettings?: (next: TSettings) => void;
  readonly editableSettings?: boolean;
  readonly chrome?: ComponentType<FamilyChromeProps<TSettings>>;
  readonly onWindowChange?: (window: FamilyRowWindow, hour: number | null) => void;
  readonly declaration: FamilyProjection<TModel, TOptions>;
  readonly view: string;
  readonly options: TOptions;
  readonly label: string;
  readonly subject: string;
  readonly widget: WidgetBase;
  readonly runtime: WidgetRuntime;
  readonly active: boolean;
  readonly subscriptions: readonly FamilySourceSubscription[];
  readonly allowEmptyFollow?: boolean;
  readonly owner?: string;
  readonly staticSources?: FamilyStaticSources | null;
  readonly staticProof?: FamilyStaticProof;
  readonly pagedRows?: PagedRows;
  readonly tiledSeries?: Readonly<Record<string, ReadonlyMap<string, TiledLine>>>;
  readonly tiledRevision?: number;
  readonly hovers?: Readonly<Record<string, { readonly hour: number; readonly data: unknown }>>;
  readonly artResolutions?: Readonly<Record<string, ArtResolution>>;
  readonly extent?: readonly [number, number] | null;
  readonly plan?: SourcePlan | null;
  readonly exactReadView?: (hour: number, rowWindow: FamilyRowWindow | null) => Promise<BoundedReadView | null>;
  readonly offset: number;
  readonly pin: number | null;
  readonly bounds?: readonly [number, number] | null;
  readonly countryLooks?: CountryLooksMap;
  readonly identitySnapshot?: IdentitySnapshot | null;
  readonly onIdentityDemands?: (demands: readonly (readonly [EntityKind, string, number])[]) => void;
  readonly onArtDemands?: (demands: readonly FamilyArtDemand[]) => void;
  readonly onViewportChange?: (viewport: FamilyViewportChange) => void;
  readonly onHoverHour?: (hour: number | null) => void;
  readonly selectedKey?: string | null;
  readonly onActivate?: (action: InstalledAction) => void;
  readonly onToggle?: (action: InstalledAction) => void;
  readonly onRegions?: (command: RegionCommand) => string | null;
  readonly emptyAction?: ReactNode | ((openRegions: (anchor: { readonly x: number; readonly y: number }) => void) => ReactNode);
  readonly contextMenuItems?: (hit: FamilyContextHit) => MenuItem[];
  readonly headerSlot?: HTMLElement | null;
  readonly onDiagnostics?: (warnings: string[]) => void;
  readonly extraWarnings?: readonly string[];
  readonly blockedState?: ReactNode;
  readonly held?: boolean;
  readonly loadingLabel?: string;
  readonly className?: string;
}

function useViewportHeight(): number {
  const [height, setHeight] = useState(() => (typeof globalThis.innerHeight === 'number' ? globalThis.innerHeight : 800));
  useEffect(() => {
    if (typeof globalThis.addEventListener !== 'function') return;
    const update = () => setHeight(typeof globalThis.innerHeight === 'number' ? globalThis.innerHeight : 800);
    globalThis.addEventListener('resize', update);
    return () => globalThis.removeEventListener('resize', update);
  }, []);
  return height;
}

export default function FamilyHost<TModel = unknown, TOptions = unknown, TBinding = unknown, TSettings = unknown>({
  adapter,
  settings,
  onSettings,
  editableSettings = false,
  chrome: Chrome,
  onWindowChange,
  declaration,
  view,
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
  staticProof,
  pagedRows,
  tiledSeries,
  tiledRevision = 0,
  hovers,
  artResolutions,
  extent = null,
  plan = null,
  exactReadView,
  offset,
  pin,
  bounds = null,
  countryLooks,
  identitySnapshot = null,
  onIdentityDemands,
  onArtDemands,
  onViewportChange,
  onHoverHour,
  selectedKey = null,
  onActivate,
  onToggle,
  onRegions,
  emptyAction,
  contextMenuItems,
  headerSlot = null,
  onDiagnostics,
  extraWarnings,
  blockedState = null,
  held = false,
  loadingLabel,
  className = '',
}: FamilyHostComponentProps<TModel, TOptions, TBinding, TSettings>) {
  const prepared = useContext(PreparingPage);
  useWidgetSkeleton(adapter.skeleton ?? 'chart');
  const viewportHeight = useViewportHeight();
  const contextMenu = useContextMenu(`${label} actions`);
  const contextMenuRef = useRef(contextMenu);
  contextMenuRef.current = contextMenu;

  const frameRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const listboxMirrorRef = useRef<HTMLDivElement>(null);
  const accessibleTableRef = useRef<HTMLTableElement>(null);
  const mirrorBodyRef = useRef<HTMLTableSectionElement>(null);
  const mirrorHeadRef = useRef<HTMLTableSectionElement>(null);
  const bodyWrapRef = useRef<HTMLDivElement>(null);
  const [targets, setTargets] = useState<FamilyChromeTargets>(NO_TARGETS);
  const windowRef = useRef<{ readonly window: FamilyRowWindow; readonly hour: number } | null>(null);
  const requestedWindowRef = useRef<FamilyRowWindow | null>(null);
  const [chromeState, setChromeState] = useState<unknown>(undefined);
  const chromeStateRef = useRef<unknown>(undefined);
  const [chromeWidth, setChromeWidth] = useState(0);
  const surfaceRef = useRef<MountedFamilyCanvas | null>(null);
  const descriptionRef = useRef<HTMLDivElement>(null);
  const lastDescriptionRef = useRef<string | null>(null);

  const scrollMode = adapter.scroll ?? 'none';
  const mirrorKind = adapter.mirror ?? 'none';
  const pointerMode = adapter.pointerMode ?? 'discrete';

  const viewportRef = useRef({
    width: 0,
    height: 0,
    dpr: currentDpr(),
    scrollLeft: 0,
    scrollTop: 0,
    zoomWindow: null as readonly [number, number] | null,
  });
  const scrollGeometryRef = useRef<ScrollGeometry>(computeScrollGeometry(0, 0));
  const lastViewportReportRef = useRef('');

  const focusedKeyRef = useRef<string | null>(null);
  const focusInsideRef = useRef(false);
  const focusAnchorRef = useRef({ x: 0, y: 0 });
  const regionDragRef = useRef<RegionRowRef | null>(null);
  const regionDropRef = useRef<HTMLDivElement>(null);
  const restoringFocusRef = useRef(false);
  const inspectedKeyRef = useRef<string | null>(null);
  const inspectedHitRef = useRef<ParsedFamilyHit | null>(null);
  const lastPointerRef = useRef<{ x: number; y: number } | null>(null);
  const lastHoverHourRef = useRef<number | null>(null);
  const heldRef = useRef(held);
  heldRef.current = held;
  const pauseFlushRef = useRef(0);
  const recordRef = useRef<InstalledRecord | null>(null);
  const resolveInstalledRef = useRef<() => InstalledPicture | null>(() => recordRef.current?.picture ?? null);
  const selectionsRef = useRef(new BoundSelections());
  const typeaheadRef = useRef({ query: '', until: 0 });
  const expansionRef = useRef<FamilyExpansion>(NO_EXPANSION);

  const lastMirrorKeyRef = useRef('');
  const lastMirrorObjRef = useRef<MirrorModel | null>(null);
  const lastMirrorSelectedRef = useRef<string | null>(null);
  const fieldsNow = () => recordRef.current?.fields ?? EMPTY_FIELDS;
  const navNow = () => recordRef.current?.nav ?? EMPTY_NAV;
  const lastVisibleRef = useRef<boolean | null>(null);
  const lastExtentRef = useRef({ width: -1, height: -1 });
  const lastRawMetaRef = useRef<{ mode: string; reason: string | null; failureOrigin: FailureOrigin; raw: unknown } | null>(null);
  const lastPhasesRef = useRef<unknown>(null);
  const lastIdentityDemandsRef = useRef<unknown>(null);
  const lastArtDemandsRef = useRef<unknown>(null);
  const lastTableRef = useRef<unknown>(null);
  const lastRowCountRef = useRef(-1);
  const lastFocusNavRef = useRef<unknown>(null);
  const lastAuditRef = useRef<{ readonly id: number; readonly key: readonly unknown[]; readonly context: { fields: Readonly<Record<string, unknown>> } } | null>(null);

  const [meta, setMeta] = useState<FamilyMetaState>(() => ({
    mode: staticSources ? 'picture' : 'reading',
    reason: null,
    failureOrigin: 'local',
    empty: null,
    emptyDetail: null,
    emptyKind: 'empty',
    summaryText: null,
    summaryPartial: null,
    notes: NO_STRINGS,
    warnings: NO_STRINGS,
  }));
  const metaRef = useRef(meta);
  metaRef.current = meta;

  const { theme, reducedMotion } = useFamilyTheme(undefined, countryLooks);
  const { announce, hasFrame, liveRegionRef } = useFamilyAnnouncer(adapter.family);
  const announceRef = useRef(announce);
  announceRef.current = announce;
  const say = useCallback((message: string) => announceRef.current(message), []);

  const hasTiled = !!tiledSeries && Object.keys(tiledSeries).length > 0;
  const enabled = active && !prepared && !blockedState && (subscriptions.length > 0 || staticSources !== null || hasTiled || allowEmptyFollow);
  const auditable = !!exactReadView && !!adapter.deriveAuditRows;
  const resolvedOwner = owner ?? `${widget.id}:${declaration.kind}:${subject}`;

  const placement = useFamilyPlacement({
    family: adapter.family,
    declarationKind: declaration.kind,
    widgetId: widget.id,
    widgetKind: widget.kind,
    owner: resolvedOwner,
    subject,
    widgetTrackIds: widget.trackIds,
    runtime,
    enabled,
    auditable,
    subscriptions,
    offset,
    pin: pin ?? (subscriptions.length === 0 && staticSources && !staticSources.follows ? staticSources.hour : null),
    bounds,
    frameRef,
    surfaceLabel: adapter.family,
  });
  const { pipeline, pipe, sourceIdsRef, subsReady, instanceId, tooltipOwner, dataContextKey, retryStream, retryLocal } = placement;

  const tooltips = useFamilyTooltip(tooltipOwner, dataContextKey);
  useFamilyHold(frameRef, held, tooltips);

  const audit = useFamilyAudit<unknown>({
    pipeline,
    pipe,
    ownerId: adapter.auditOwnerId ?? DEFAULT_AUDIT_OWNER,
    auditable,
  });

  const reportShown = useShownHour();
  const reportShownRef = useRef(reportShown);
  reportShownRef.current = reportShown;

  const latest = {
    adapter,
    settings,
    onWindowChange,
    hasChrome: !!Chrome,
    declaration,
    view,
    options,
    theme,
    reducedMotion,
    staticSources,
    staticProof,
    pagedRows,
    tiledSeries,
    hovers,
    artResolutions,
    extent,
    plan,
    exactReadView,
    onIdentityDemands,
    onArtDemands,
    onViewportChange,
    onHoverHour,
    selectedKey,
    onActivate,
    onToggle,
    onRegions,
    contextMenuItems,
    viewportHeight,
    bounds,
    allowEmptyFollow,
    captureActive: audit.captureActive,
  };
  const latestRef = useRef(latest);
  const openRegionsAt = (anchor: { readonly x: number; readonly y: number }) => {
    const field = fieldsNow().regions as RegionRowsField | undefined;
    latestRef.current.onRegions?.({ op: 'open', x: anchor.x, y: anchor.y, rows: field?.rows ?? [], ...(field?.active ? { active: field.active } : {}) });
  };
  latestRef.current = latest;

  const publishHoverHour = useCallback((nextHour: number | null) => {
    const normalized = nextHour !== null && Number.isFinite(nextHour) ? Math.floor(nextHour) : null;
    if (normalized === lastHoverHourRef.current) return;
    lastHoverHourRef.current = normalized;
    latestRef.current.onHoverHour?.(normalized);
  }, []);

  const notifyViewportChange = useCallback(() => {
    const cb = latestRef.current.onViewportChange;
    const vp = viewportRef.current;
    if (!cb || vp.width <= 0) return;
    const win = vp.zoomWindow;
    const sig = `${vp.width}|${vp.height}|${vp.dpr}|${win ? `${win[0]}:${win[1]}` : ''}`;
    if (sig === lastViewportReportRef.current) return;
    lastViewportReportRef.current = sig;
    cb({ width: vp.width, height: vp.height, dpr: vp.dpr, window: win });
  }, []);

  const refreshFocusAnchor = useCallback(() => {
    const box = frameRef.current?.getBoundingClientRect();
    if (box) focusAnchorRef.current = { x: box.left + Math.min(200, box.width / 2), y: box.top + 48 };
  }, []);

  const reportWindow = useCallback(() => {
    const requested = requestedWindowRef.current;
    const installed = recordRef.current?.picture;
    const reported = windowRef.current;
    if (!requested || !installed || (reported && reported.window === requested && installed.hour >= reported.hour)) return;
    windowRef.current = { window: requested, hour: installed.hour };
    latestRef.current.onWindowChange?.(requested, installed.hour);
  }, []);

  const syncMirror = useCallback((mirror: MirrorModel | null) => {
    const kind = latestRef.current.adapter.mirror ?? 'none';
    const selected = recordRef.current?.selectedKey ?? null;
    if (kind === 'table') {
      const moved = syncTableMirror(mirrorBodyRef.current, mirror, {
        selectedKey: selected,
        rowsFocusable: (latestRef.current.adapter.mirrorFocus ?? 'body') === 'rows',
        focusedKeyRef,
        focusInsideRef,
        restoringFocusRef,
        lastMirrorKeyRef,
      });
      if (moved) queueMicrotask(() => bindRef.current());
      return;
    }
    if (kind !== 'listbox') return;
    const res = syncListboxMirror(listboxMirrorRef.current, mirror ? listboxItems(mirror, selected) : null, {
      optionIdPrefix: `${latestRef.current.adapter.family}-${instanceId}`,
      total: mirror?.total ?? 0,
      focusedKey: focusedKeyRef.current,
      lastMirrorKeyRef,
    });
    if (!bodyRef.current) return;
    if (res.activeDescendantId) bodyRef.current.setAttribute('aria-activedescendant', res.activeDescendantId);
    else bodyRef.current.removeAttribute('aria-activedescendant');
  }, [instanceId]);
  const syncMirrorRef = useRef(syncMirror);
  syncMirrorRef.current = syncMirror;

  const datedNow = useCallback(() => {
    const cur = latestRef.current;
    return boundDated(
      cur.staticSources,
      ownProof(cur.staticSources, cur.staticProof?.read() ?? null),
      Object.keys(sourceIdsRef.current).length > 0,
      cur.allowEmptyFollow || (!!cur.tiledSeries && Object.keys(cur.tiledSeries).length > 0),
      cur.bounds,
    ) ?? null;
  }, [sourceIdsRef]);

  const supply = useMemo(() => (pipeline && pipe !== null ? new StaticSupply(change => pipeline.supply(pipe, change)) : null), [pipeline, pipe]);
  const supplyDated = useCallback(() => supply?.date(latestRef.current.staticSources, datedNow()), [supply, datedNow]);

  const bind = useCallback(() => {
    const mounted = surfaceRef.current;
    if (!mounted || !pipeline || pipe === null) return;
    const cur = latestRef.current;
    const curAdapter = cur.adapter;
    const [defW, defH] = curAdapter.defaultSize ?? [320, 200];
    const vp = viewportRef.current;
    const w = Math.max(1, vp.width || defW);
    const h = Math.max(1, vp.height || defH);
    const dpr = currentDpr();
    vp.dpr = dpr;
    const viewport: FamilyViewportState = {
      width: w,
      height: h,
      dpr,
      scrollLeft: vp.scrollLeft,
      scrollTop: physicalToLogicalTop(vp.scrollTop, scrollGeometryRef.current),
      zoomWindow: vp.zoomWindow,
    };
    const asked = requestedRowWindow(curAdapter, cur.plan, viewport, cur.settings, fieldsNow());
    const held = requestedWindowRef.current;
    const rowWindow = asked && held && asked.first === held.first && asked.count === held.count ? held : asked;

    let auditId: number | undefined;
    if (perfLog && cur.captureActive && cur.exactReadView && curAdapter.deriveAuditRows) {
      const key = [cur.declaration, cur.options, cur.exactReadView, curAdapter.deriveAuditRows, viewport.width, viewport.height, viewport.scrollLeft, viewport.scrollTop, rowWindow?.first, rowWindow?.count, cur.settings, expansionRef.current, cur.theme];
      const prev = lastAuditRef.current;
      if (prev && prev.key.length === key.length && prev.key.every((part, i) => part === key[i])) {
        auditId = prev.id;
      } else {
        const deriveRows = curAdapter.deriveAuditRows;
        const readExact = cur.exactReadView;
        const context = { viewport, settings: cur.settings, expanded: expansionRef.current, theme: cur.theme, fields: fieldsNow() };
        const snapshot: PlanAuditSnapshot<TModel, TOptions> = {
          projection: cur.declaration,
          options: cur.options,
          exactReadView: hour => readExact(hour, rowWindow),
          deriveRows: (model, hour) => deriveRows(model, hour, context),
          ...(curAdapter.auditOrdered !== undefined ? { ordered: curAdapter.auditOrdered } : {}),
        };
        auditId = audit.capture(snapshot, PLAN_VERIFIER);
        lastAuditRef.current = { id: auditId, key, context };
      }
    }

    const binding = familyBinding(curAdapter, {
      kind: cur.declaration.kind,
      view: cur.view,
      options: cur.options,
      settings: cur.settings,
      sources: sourceIdsRef.current,
      ...(cur.pagedRows && Object.keys(cur.pagedRows).length ? { pagedRows: cur.pagedRows } : {}),
      ...(cur.tiledSeries ? { tiledSeries: cur.tiledSeries } : {}),
      ...(cur.hovers ? { hovers: cur.hovers } : {}),
      ...(cur.artResolutions ? { artResolutions: cur.artResolutions } : {}),
      extent: cur.extent,
      plan: cur.plan,
      viewport,
      theme: cur.theme,
      reducedMotion: cur.reducedMotion,
      selectedKey: cur.selectedKey,
      focusedKey: focusInsideRef.current ? focusedKeyRef.current : null,
      expanded: expansionRef.current,
      audit: auditId !== undefined,
      auditId,
      pauseFlush: pauseFlushRef.current,
      window: rowWindow,
    });
    if (binding === null) return;
    requestedWindowRef.current = rowWindow;
    reportWindow();

    const widthPx = `${w}px`;
    const heightPx = `${h}px`;
    if (mounted.canvas.style.width !== widthPx) mounted.canvas.style.width = widthPx;
    if (mounted.canvas.style.height !== heightPx) mounted.canvas.style.height = heightPx;
    pipeline.present(pipe, curAdapter.family);
    pipeline.bind(pipe, [mounted.surface], { surfaces: [{ width: w, height: h, dpr }] }, {}, binding);
    const bound = pipeline.view(pipe)?.binding;
    if (bound !== undefined) selectionsRef.current.bound(bound, cur.selectedKey);
    supplyDated();
  }, [pipeline, pipe, audit.capture, sourceIdsRef, supplyDated, reportWindow]);

  const bindRef = useRef(bind);
  bindRef.current = bind;
  const clearHoverRef = useRef<() => void>(() => {});

  const setZoomWindow = useCallback((next: readonly [number, number] | null) => {
    const prev = viewportRef.current.zoomWindow;
    if (prev === next || (prev && next && prev[0] === next[0] && prev[1] === next[1])) return;
    viewportRef.current.zoomWindow = next;
    clearHoverRef.current();
    notifyViewportChange();
    bindRef.current();
  }, [notifyViewportChange]);

  const gestureContext = useCallback((): FamilyGestureContext => ({
    fields: fieldsNow(),
    viewport: { ...viewportRef.current },
    extent: latestRef.current.extent,
    setZoomWindow,
  }), [setZoomWindow]);

  const gateNow = useCallback((): HostInputGate => ({
    current: installedRecordOf(resolveInstalledRef.current()),
    held: heldRef.current,
    expired: !!pipeline && pipe !== null && pipeline.view(pipe)?.expired === true,
  }), [pipeline, pipe]);

  const dispatch = useCallback((command: HostCommand, input: HostInputAt): boolean => {
    const cur = latestRef.current;
    const effect = resolveHostInput(cur.adapter, command, input, gateNow(), {
      expansion: expansionRef.current,
      takesToggles: !!cur.onToggle,
      regions: !!cur.onRegions,
      menu: !!cur.contextMenuItems,
    }, parseGridKey);
    if (!effect) return false;
    switch (effect.kind) {
      case 'menu': {
        const items = cur.contextMenuItems!(effect.hit);
        if (!items.length) return false;
        const at = input.at ?? focusAnchorRef.current;
        contextMenuRef.current.open(at.x, at.y, items);
        return true;
      }
      case 'pin':
        tooltips.pinFocused(effect.tip, effect.hour, focusAnchorRef.current, cur.viewportHeight, say);
        return true;
      case 'region-open': {
        const field = effect.field;
        const box = bodyRef.current?.getBoundingClientRect();
        const at = field.add
          ? { x: (box?.left ?? 0) + field.nameLeft, y: (box?.top ?? 0) + field.add.top + field.add.height - viewportRef.current.scrollTop }
          : focusAnchorRef.current;
        cur.onRegions!({ op: 'open', x: at.x, y: at.y, rows: field.rows, ...(field.active ? { active: field.active } : {}) });
        return true;
      }
      case 'region-remove': {
        const message = cur.onRegions!({ op: 'remove', row: effect.row });
        if (message) say(message);
        return true;
      }
      case 'activate':
        tooltips.dismissActivated(effect.key);
        cur.onActivate?.(effect.action);
        return true;
      case 'toggle':
        if (command !== 'toggle' && input.key !== null) tooltips.dismissActivated(input.key);
        if (!effect.expansion) {
          cur.onToggle?.(effect.action);
          return true;
        }
        expansionRef.current = effect.expansion.next;
        say(`${effect.expansion.label} ${effect.expansion.open ? 'expanded' : 'collapsed'}.`);
        bindRef.current();
        return true;
    }
  }, [gateNow, say, tooltips]);

  const revealRow = useCallback((entry: FamilyNavTuple): boolean => {
    const top = entry[6];
    const height = entry[7];
    if (top === undefined || height === undefined) return false;
    const vp = viewportRef.current;
    const rows = { scrollTop: vp.scrollTop, height: usableRowHeight(fieldsNow(), vp.height) };
    const moved = scrollRangeIntoView(bodyRef.current, rows, top, height, scrollGeometryRef.current);
    vp.scrollTop = rows.scrollTop;
    return moved;
  }, []);

  const focusRowByKey = useCallback((key: string) => {
    const entry = navNow().find(item => item[0] === key);
    if (!entry) return;
    focusedKeyRef.current = key;
    revealRow(entry);
    if ((latestRef.current.adapter.mirrorFocus ?? 'body') === 'rows') {
      const tr = mirrorBodyRef.current?.querySelector<HTMLTableRowElement>(mirrorRowSelector(key));
      if (tr) {
        for (const sibling of mirrorBodyRef.current!.querySelectorAll<HTMLTableRowElement>('tr[tabindex="0"]')) {
          if (sibling !== tr) sibling.tabIndex = -1;
        }
        tr.tabIndex = 0;
        tr.focus({ preventScroll: true });
      }
    }
    bindRef.current();
  }, [revealRow]);

  const focusGridCell = useCallback((row: number, column: number) => {
    const curAdapter = latestRef.current.adapter;
    const metrics = curAdapter.getGridMetrics?.(fieldsNow());
    focusedKeyRef.current = (curAdapter.formatGridKey ?? formatGridKey)(row, column);
    const node = bodyRef.current;
    if (node && metrics) {
      const vp = viewportRef.current;
      const nextLeft = revealGridAxis(column, metrics.cellWidth ?? metrics.cellSize, node.scrollLeft, Math.max(1, vp.width - metrics.gridLeft));
      const nextTop = revealGridAxis(row, metrics.cellSize, node.scrollTop, Math.max(1, vp.height - metrics.gridTop));
      if (nextLeft !== node.scrollLeft) node.scrollLeft = vp.scrollLeft = nextLeft;
      if (nextTop !== node.scrollTop) node.scrollTop = vp.scrollTop = nextTop;
      const rowLabel = metrics.rowLabel?.(row);
      const columnLabel = metrics.columnLabel?.(column);
      if (rowLabel && columnLabel) say(`${rowLabel} to ${columnLabel}`);
    }
    bindRef.current();
  }, [say]);

  useLayoutEffect(() => {
    recordRef.current = null;
    regionDragRef.current = null;
    expansionRef.current = NO_EXPANSION;
    focusedKeyRef.current = null;
    inspectedKeyRef.current = null;
    inspectedHitRef.current = null;
    lastMirrorObjRef.current = null;
    publishHoverHour(null);
    if (viewportRef.current.zoomWindow) {
      viewportRef.current.zoomWindow = null;
      notifyViewportChange();
    }
  }, [resolvedOwner, publishHoverHour, notifyViewportChange]);

  useLayoutEffect(() => {
    const node = bodyRef.current;
    if (!node || !pipeline || pipe === null) return;
    lastExtentRef.current = { width: -1, height: -1 };
    lastRawMetaRef.current = null;
    lastMirrorObjRef.current = null;
    lastVisibleRef.current = null;
    lastPhasesRef.current = null;
    lastIdentityDemandsRef.current = null;
    lastArtDemandsRef.current = null;
    lastViewportReportRef.current = '';

    const mountAdapter = latestRef.current.adapter;
    const mounted = mountFamilyCanvas(node, pipeline, pipe, mountAdapter.family, 'fh-canvas', 'fh-extent');
    if ((mountAdapter.scroll ?? 'none') === 'none') {
      mounted.extent.style.width = '100%';
      mounted.extent.style.height = '100%';
    }
    surfaceRef.current = mounted;
    if (latestRef.current.hasChrome) setTargets({ body: node, bodyWrap: bodyWrapRef.current, extent: mounted.extent, mirrorHead: mirrorHeadRef.current });

    const auditWatch = attachAuditWatch(pipeline, pipe, {
      isAuditable: () => !!latestRef.current.exactReadView && !!latestRef.current.adapter.deriveAuditRows,
      onCaptureChange: activeAudit => { latestRef.current.captureActive = activeAudit; },
      bindNow: () => bindRef.current(),
    });

    const describe = (text: string | null) => {
      if (text === lastDescriptionRef.current) return;
      lastDescriptionRef.current = text;
      const target = descriptionRef.current;
      if (target) target.textContent = text ?? '';
      if (text && target) node.setAttribute('aria-describedby', target.id);
      else node.removeAttribute('aria-describedby');
    };

    const updateMeta = (mode: string, reason: string | null, failureOrigin: FailureOrigin, raw: RawMeta | undefined) => {
      const prev = lastRawMetaRef.current;
      if (prev && prev.mode === mode && prev.reason === reason && prev.failureOrigin === failureOrigin && prev.raw === raw) return;
      lastRawMetaRef.current = { mode, reason, failureOrigin, raw };
      const empty = raw?.empty ?? null;
      const emptyDetail = raw?.emptyDetail ?? null;
      const emptyKind = raw?.emptyKind ?? 'empty';
      const summaryText = raw?.summaryText ?? null;
      const summaryPartial = raw?.summaryPartial ?? null;
      const notes = raw?.notes ?? NO_STRINGS;
      const warnings = raw?.warnings ?? NO_STRINGS;
      const shown = metaRef.current;
      if (
        shown.mode === mode && shown.reason === reason && shown.failureOrigin === failureOrigin
        && shown.empty === empty && shown.emptyDetail === emptyDetail && shown.emptyKind === emptyKind
        && shown.summaryText === summaryText && shown.summaryPartial === summaryPartial && sameStrings(shown.notes, notes) && sameStrings(shown.warnings, warnings)
      ) return;
      const next: FamilyMetaState = { mode, reason, failureOrigin, empty, emptyDetail, emptyKind, summaryText, summaryPartial, notes, warnings };
      metaRef.current = next;
      setMeta(next);
    };

    const install = () => {
      const shown = pipeline.view(pipe);
      const frameNode = frameRef.current;
      const cur = latestRef.current;
      const activeAdapter = cur.adapter;
      holdElement(frameNode, !!shown?.dim);
      const visible = shown?.mode === 'picture';
      if (lastVisibleRef.current !== visible) {
        lastVisibleRef.current = visible;
        node.toggleAttribute('data-family-visible', visible);
        node.toggleAttribute('data-family-pending', !visible);
      }
      const shownHour = reportFamilyShownHour(reportShownRef.current, visible, shown);
      auditWatch.settleIfReady(visible, shown?.mode ?? 'reading', shown?.installedBinding, shown?.fields);

      const fields = shown?.fields;
      const rawMeta = fields?.meta as RawMeta | undefined;
      updateMeta(shown?.mode ?? 'reading', shown?.reason ?? null, shown?.failureOrigin ?? 'local', rawMeta);

      const idDemands = fields?.identityDemands as readonly (readonly [EntityKind, string, number])[] | undefined;
      if (idDemands && idDemands !== lastIdentityDemandsRef.current && cur.onIdentityDemands) {
        lastIdentityDemandsRef.current = idDemands;
        cur.onIdentityDemands(idDemands);
      }
      const artDemands = fields?.artDemands as readonly FamilyArtDemand[] | undefined;
      if (artDemands && artDemands !== lastArtDemandsRef.current && cur.onArtDemands) {
        lastArtDemandsRef.current = artDemands;
        cur.onArtDemands(artDemands);
      }

      if (!visible || !fields || shown?.id === undefined) {
        recordRef.current = null;
        describe(null);
        if (lastMirrorObjRef.current !== null) {
          lastMirrorObjRef.current = null;
          syncMirrorRef.current(null);
        }
        tooltips.hideIfUnpinned();
        return;
      }

      const installedHour = displayedHour(shownHour ?? shown.painted ?? shown.h ?? 0);
      const base = shown.viewport ?? {
        width: viewportRef.current.width,
        height: viewportRef.current.height,
        dpr: viewportRef.current.dpr,
        scrollLeft: viewportRef.current.scrollLeft,
        scrollTop: viewportRef.current.scrollTop,
      };
      const installedAuditId = auditIdOf(fields.audit);
      const capturedAudit = lastAuditRef.current;
      if (capturedAudit && capturedAudit.id === installedAuditId) capturedAudit.context.fields = fields;
      const installed: InstalledPicture = {
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
        viewport: rawMeta?.transform && !base.transform ? { ...base, transform: rawMeta.transform } : base,
        optionsRevision: '',
        themeRevision: '',
        actionKeys: lazyActionKeys((fields.nav as readonly FamilyNavTuple[] | undefined) ?? EMPTY_NAV, installedHour),
        accessibleDescription: typeof fields.accessibleDescription === 'string' ? fields.accessibleDescription : null,
        ...(installedAuditId !== undefined ? { auditId: installedAuditId } : {}),
      };
      const record = recordInstalled(installed, fields, selectionsRef.current.at(installed.generation.binding));
      recordRef.current = record;
      reportWindow();
      describe(installed.accessibleDescription);

      const extentSize = activeAdapter.getScrollExtent?.(fields);
      if (extentSize) {
        if (extentSize.width !== undefined) {
          const width = Math.max(viewportRef.current.width, extentSize.width);
          if (lastExtentRef.current.width !== width) {
            lastExtentRef.current.width = width;
            mounted.extent.style.width = `${width}px`;
          }
        }
        if (extentSize.height !== undefined) {
          const geometry = computeScrollGeometry(extentSize.height, viewportRef.current.height);
          scrollGeometryRef.current = geometry;
          if (lastExtentRef.current.height !== geometry.physicalHeight) {
            lastExtentRef.current.height = geometry.physicalHeight;
            mounted.extent.style.height = `${geometry.physicalHeight}px`;
          }
        }
      }

      if (activeAdapter.chromeState) {
        const nextChrome = activeAdapter.chromeState(fields, chromeStateRef.current);
        if (nextChrome !== chromeStateRef.current) {
          chromeStateRef.current = nextChrome;
          setChromeState(nextChrome);
        }
      }

      const table = fields.table as { readonly count: number } | undefined;
      if (table && table !== lastTableRef.current) {
        lastTableRef.current = table;
        if (lastRowCountRef.current !== table.count) {
          lastRowCountRef.current = table.count;
          accessibleTableRef.current?.setAttribute('aria-rowcount', String(table.count + 1));
        }
        const hovered = inspectedKeyRef.current;
        if (hovered && !record.nav.some(entry => entry[0] === hovered)) clearHoverRef.current();
      }
      if (focusInsideRef.current && focusedKeyRef.current && navNow() !== lastFocusNavRef.current) {
        lastFocusNavRef.current = navNow();
        const focusedKey = focusedKeyRef.current;
        const entry = navNow().find(item => item[0] === focusedKey);
        if (entry && revealRow(entry)) queueMicrotask(() => bindRef.current());
      }

      const mirror = mirrorToSync(fields.mirror as MirrorModel | undefined, lastMirrorObjRef.current, record.selectedKey, lastMirrorSelectedRef.current);
      if (mirror !== undefined) {
        lastMirrorObjRef.current = mirror;
        lastMirrorSelectedRef.current = record.selectedKey;
        syncMirrorRef.current(mirror);
      }

      const phases = fields.phases;
      if (perfLog && phases && phases !== lastPhasesRef.current && frameNode) {
        lastPhasesRef.current = phases;
        frameNode.dataset.familyPhases = JSON.stringify(phases);
      }

      const inspected = inspectedKeyRef.current;
      const focusInside = focusInsideRef.current;
      if (!inspected && !focusInside) {
        tooltips.hideIfUnpinned();
        return;
      }
      const hoverTip = inspected ? (fields.tooltip as FamilyTooltipField | undefined) : undefined;
      const focusTip = focusInside ? (fields.focusTooltip as FamilyTooltipField | undefined) : undefined;
      if (focusTip?.text && (activeAdapter.mirrorFocus ?? 'body') === 'rows' && focusTip.text.rowKey === focusedKeyRef.current) {
        const tr = mirrorBodyRef.current?.querySelector<HTMLTableRowElement>(mirrorRowSelector(focusTip.text.rowKey));
        tr?.setAttribute('aria-description', tooltips.describeFocused(focusTip.text, installedHour, cur.viewportHeight));
      }
      const inspectedHit = inspectedHitRef.current;
      tooltips.syncFromInstall({
        dim: !!shown.dim,
        held: heldRef.current,
        inspectedKey: inspected,
        inspectedToken: inspectedHit ? (inspectedHit.inspectToken ?? familyInspectToken(activeAdapter, inspectedHit.key)) : null,
        pointerPos: lastPointerRef.current,
        hoverTip: hoverTip?.text ?? null,
        hoverFieldKey: hoverTip?.key ?? null,
        keepHoverWhilePending: (activeAdapter.pointerMode ?? 'discrete') === 'continuous',
        focusInside,
        focusedKey: focusedKeyRef.current,
        focusAnchor: focusAnchorRef.current,
        focusTip: focusTip?.text ?? null,
        viewportHeight: cur.viewportHeight,
      });
    };

    install();
    const stopInstall = pipeline.onInstall(pipe, install);
    const resolveInstalled = () => {
      const shown = pipeline.view(pipe);
      const shownId = shown?.mode === 'picture' ? shown.id : undefined;
      if (shownId !== recordRef.current?.picture?.id) install();
      return recordRef.current?.picture ?? null;
    };
    resolveInstalledRef.current = resolveInstalled;

    const gestures = mountAdapter.gestures;
    const stopPointer = bindFamilyPointer({
      bodyNode: node,
      menuHost: frameRef.current ?? node,
      pipeline,
      pipe,
      tooltips,
      inspectedKeyRef,
      inspectedHitRef,
      lastPointerRef,
      focusInsideRef,
      focusedKeyRef,
      focusAnchorRef,
      accessibleRef: (mountAdapter.mirror ?? 'none') === 'table' ? accessibleTableRef : (mountAdapter.mirror ?? 'none') === 'listbox' ? listboxMirrorRef : bodyRef,
      pointerMode,
      isBlocked: () => inputBlocked(gateNow()),
      getInstalled: resolveInstalled,
      getViewportHeight: () => latestRef.current.viewportHeight,
      parseHit: hitKey => parseFamilyHit(latestRef.current.adapter, hitKey),
      inspectToken: key => familyInspectToken(latestRef.current.adapter, key),
      onHoverChange: hit => publishHoverHour(hit?.hour ?? null),
      ...(gestures
        ? {
            gestures: {
              ...(gestures.onWheel ? { onWheel: (event: WheelEvent, point: readonly [number, number]) => gestures.onWheel!(event, point, gestureContext()) } : {}),
              ...(gestures.onDragStart ? { onDragStart: (point: readonly [number, number]) => gestures.onDragStart!(point, gestureContext()) } : {}),
              ...(gestures.onDragMove ? { onDragMove: (point: readonly [number, number], delta: readonly [number, number]) => gestures.onDragMove!(point, delta, gestureContext()) } : {}),
              ...(gestures.onDragEnd ? { onDragEnd: () => gestures.onDragEnd!(gestureContext()) } : {}),
              ...(gestures.onDoubleClick ? { onDoubleClick: (point: readonly [number, number]) => gestures.onDoubleClick!(point, gestureContext()) } : {}),
            },
          }
        : {}),
      rowDrag: {
        starts: hit => {
          const row = hit.kind === 'region-grip' && latestRef.current.onRegions
            ? regionRowOf(fieldsNow().regions as RegionRowsField | undefined, hit.key)
            : null;
          regionDragRef.current = row;
          return row !== null;
        },
        move: point => {
          const row = regionDragRef.current;
          const field = fieldsNow().regions as RegionRowsField | undefined;
          const line = regionDropRef.current;
          if (!line) return;
          const drop = row && field ? regionDropAt(field, row, point[1] + viewportRef.current.scrollTop) : null;
          line.hidden = drop === null;
          if (drop) line.style.top = `${Math.round(drop.lineY - viewportRef.current.scrollTop) - 1}px`;
        },
        end: point => {
          const row = regionDragRef.current;
          const field = fieldsNow().regions as RegionRowsField | undefined;
          const send = latestRef.current.onRegions;
          regionDragRef.current = null;
          if (regionDropRef.current) regionDropRef.current.hidden = true;
          if (!row || !field || !point || !send) return;
          const drop = regionDropAt(field, row, point[1] + viewportRef.current.scrollTop);
          if (!drop) return;
          const message = send({ op: 'drop', row, to: drop.to });
          if (message) say(message);
        },
      },
      onActivate: (key, toggle, hit, picture) => { dispatch(toggle ? 'toggle' : 'activate', { key, hit, picture }); },
      onContextMenu: (key, clientX, clientY, hit, picture) => dispatch('menu', { key, hit: hit ?? null, picture: picture ?? null, at: { x: clientX, y: clientY } }),
    });
    clearHoverRef.current = stopPointer.clearHover;

    const onScroll = scrollMode !== 'none'
      ? () => {
          const left = Math.floor(node.scrollLeft);
          const top = Math.floor(node.scrollTop);
          if (left === viewportRef.current.scrollLeft && top === viewportRef.current.scrollTop) return;
          viewportRef.current.scrollLeft = left;
          viewportRef.current.scrollTop = top;
          bindRef.current();
        }
      : null;
    if (onScroll) node.addEventListener('scroll', onScroll, { passive: true });

    bindRef.current();

    return () => {
      resolveInstalledRef.current = () => recordRef.current?.picture ?? null;
      describe(null);
      if (onScroll) node.removeEventListener('scroll', onScroll);
      clearHoverRef.current = () => {};
      stopPointer();
      stopInstall();
      auditWatch.dispose();
      lastMirrorObjRef.current = null;
      syncMirrorRef.current(null);
      recordRef.current = null;
      publishHoverHour(null);
      mounted.unmount();
      surfaceRef.current = null;
      holdElement(frameRef.current, false);
      reportFamilyShownHour(reportShownRef.current, false, null);
    };
  }, [pipeline, pipe, instanceId, tooltips, scrollMode, pointerMode, gestureContext, dispatch, gateNow, publishHoverHour, reportWindow]);

  useLayoutEffect(() => {
    const node = bodyRef.current;
    if (!node) return;
    const measure = (width: number, height: number) => {
      refreshFocusAnchor();
      const w = Math.floor(width);
      const h = Math.floor(height);
      const dpr = currentDpr();
      const vp = viewportRef.current;
      if (w === vp.width && h === vp.height && dpr === vp.dpr) return;
      vp.width = w;
      vp.height = h;
      vp.dpr = dpr;
      if (latestRef.current.hasChrome) setChromeWidth(w);
      notifyViewportChange();
      bindRef.current();
    };
    measure(Math.floor(node.clientWidth), Math.floor(node.clientHeight));
    if (typeof ResizeObserver !== 'function') return;
    const observer = new ResizeObserver(entries => {
      const rect = entries[0]?.contentRect;
      measure(rect ? rect.width : node.clientWidth, rect ? rect.height : node.clientHeight);
    });
    observer.observe(node);
    return () => observer.disconnect();
  }, [notifyViewportChange, refreshFocusAnchor]);

  useLayoutEffect(() => {
    if (pipe !== null && enabled) bind();
  }, [
    pipe,
    enabled,
    bind,
    declaration,
    view,
    options,
    theme,
    reducedMotion,
    pagedRows,
    tiledSeries,
    tiledRevision,
    hovers,
    artResolutions,
    extent,
    plan,
    selectedKey,
    subsReady,
    auditable,
    exactReadView,
    audit.captureActive,
    settings,
  ]);

  useLayoutEffect(() => {
    supply?.statics(staticSources, datedNow());
  }, [supply, staticSources, resolvedOwner, datedNow]);

  useLayoutEffect(() => {
    if (!pipeline || pipe === null) return;
    pipeline.supply(pipe, { identity: identitySnapshot });
  }, [pipeline, pipe, identitySnapshot]);

  useLayoutEffect(() => staticProof?.subscribe(supplyDated), [staticProof, supplyDated]);

  useFamilyPauseFlush(runtime.clock, () => {
    if (!pipeline || pipe === null) return;
    pauseFlushRef.current++;
    bindRef.current();
    const mirror = pipeline.view(pipe)?.fields?.mirror as MirrorModel | undefined;
    if (mirror) syncMirrorRef.current(mirror);
  });

  useEffect(() => {
    if (!onDiagnostics) return;
    onDiagnostics([...(extraWarnings ?? []), ...meta.warnings]);
    return () => onDiagnostics([]);
  }, [onDiagnostics, extraWarnings, meta.warnings]);

  const onKeyDown = useCallback((event: ReactKeyboardEvent) => {
    const picture = resolveInstalledRef.current();
    const activeAdapter = latestRef.current.adapter;
    const gate = gateNow();
    const fields = gate.current?.fields ?? EMPTY_FIELDS;
    const keyNav = gate.current?.nav ?? EMPTY_NAV;
    const blocked = inputBlocked(gate);
    const run = (command: HostCommand, key: string | null) => {
      refreshFocusAnchor();
      return dispatch(command, { key, hit: null, picture });
    };
    const unpin = () => tooltips.unpin(say);

    if (activeAdapter.navigation === 'grid') {
      const metrics = activeAdapter.getGridMetrics?.(fields);
      const parseKey = activeAdapter.parseGridKey ?? parseGridKey;
      const formatKey = activeAdapter.formatGridKey ?? formatGridKey;
      handleFamilyGridKeyDown(event, {
        rows: metrics?.rows ?? 0,
        columns: metrics?.columns ?? 0,
        current: parseKey(focusedKeyRef.current) ?? { row: 0, column: 0 },
        pageStep: metrics ? Math.max(1, Math.floor(Math.max(1, viewportRef.current.height - metrics.gridTop) / Math.max(1, metrics.cellSize))) : 1,
        blocked,
        isPinnedTooltip: () => tooltips.isPinned(),
        onFocusCell: focusGridCell,
        onActivate: (row, column) => { run('activate', formatKey(row, column)); },
        onContextMenu: (row, column) => run('menu', row >= 0 && column >= 0 ? formatKey(row, column) : null),
        onTogglePinTooltip: (row, column) => { run('pin', formatKey(row, column)); },
        onUnpinTooltip: unpin,
      });
      return;
    }

    if (activeAdapter.navigation === 'points') {
      handleFamilyPointsKeyDown(event, {
        nav: keyNav,
        focusedKey: focusedKeyRef.current,
        pageStep: DEFAULT_POINTS_PAGE,
        blocked,
        isPinnedTooltip: () => tooltips.isPinned(),
        onFocusKey: key => {
          focusedKeyRef.current = key;
          const hour = parseFamilyHit(activeAdapter, key)?.hour;
          if (hour !== undefined) publishHoverHour(hour);
          bindRef.current();
        },
        onActivate: entry => { run('activate', entry[0]); },
        onContextMenu: entry => run('menu', entry ? entry[0] : null),
        onTogglePinTooltip: key => { run('pin', key); },
        onUnpinTooltip: unpin,
      });
      return;
    }

    const sendRegions = latestRef.current.onRegions;
    const regionRow = sendRegions && !blocked ? regionRowOf(fields.regions as RegionRowsField | undefined, focusedKeyRef.current) : null;
    if (sendRegions && regionRow && (event.key === 'Delete' || (event.altKey && (event.key === 'ArrowUp' || event.key === 'ArrowDown')))) {
      event.preventDefault();
      const message = event.key === 'Delete'
        ? sendRegions({ op: 'remove', row: regionRow })
        : sendRegions({ op: 'step', row: regionRow, delta: event.key === 'ArrowUp' ? -1 : 1 });
      if (message) say(message);
      return;
    }

    handleFamilyKeyDown(event, {
      nav: keyNav,
      focusedKey: focusedKeyRef.current,
      pageStep: navPageSteps(keyNav, focusedKeyRef.current, usableRowHeight(fields, viewportRef.current.height)),
      blocked,
      typeaheadRef,
      isPinnedTooltip: () => tooltips.isPinned(),
      onFocusKey: focusRowByKey,
      onActivate: entry => { run('activate', entry[0]); },
      onToggleTree: key => { run('toggle', key); },
      onContextMenu: entry => run('menu', entry ? entry[0] : null),
      onTogglePinTooltip: key => { run('pin', key); },
      onUnpinTooltip: unpin,
    });
  }, [say, dispatch, gateNow, focusGridCell, focusRowByKey, publishHoverHour, refreshFocusAnchor, tooltips]);

  const onFocus = useCallback(() => {
    focusInsideRef.current = true;
    refreshFocusAnchor();
    const activeAdapter = latestRef.current.adapter;
    if (!focusedKeyRef.current) {
      if (activeAdapter.navigation === 'grid') {
        const metrics = activeAdapter.getGridMetrics?.(fieldsNow());
        if (metrics && metrics.rows > 0 && metrics.columns > 0) focusedKeyRef.current = (activeAdapter.formatGridKey ?? formatGridKey)(0, 0);
      } else if (navNow().length) {
        focusedKeyRef.current = navNow()[0]![0];
      }
    }
    bindRef.current();
  }, [refreshFocusAnchor]);

  const onBlur = useCallback(() => {
    focusInsideRef.current = false;
    if (!inspectedKeyRef.current) tooltips.hideIfUnpinned();
    bindRef.current();
  }, [tooltips]);

  const onRowFocus = useCallback((event: ReactFocusEvent<HTMLTableElement>) => {
    if (restoringFocusRef.current) return;
    const wasInside = focusInsideRef.current;
    focusInsideRef.current = true;
    refreshFocusAnchor();
    const row = (event.target as HTMLElement | null)?.closest<HTMLTableRowElement>('tr[data-row-key]');
    if (row?.dataset.rowKey && (!wasInside || focusedKeyRef.current !== row.dataset.rowKey)) {
      focusedKeyRef.current = row.dataset.rowKey;
      bindRef.current();
    }
  }, [refreshFocusAnchor]);

  const onRowBlur = useCallback((event: ReactFocusEvent<HTMLTableElement>) => {
    if (restoringFocusRef.current) return;
    if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
    focusInsideRef.current = false;
    if (!inspectedKeyRef.current) tooltips.hideIfUnpinned();
    bindRef.current();
  }, [tooltips]);

  const onRetryLocal = useCallback(() => retryLocal(() => bindRef.current()), [retryLocal]);

  const bodyStateNode = blockedState ?? renderFamilyBodyState({
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
    emptyAction: typeof emptyAction === 'function' ? emptyAction(openRegionsAt) : emptyAction,
  });

  const summary = meta.mode === 'picture' && !meta.empty && meta.summaryText
    ? <span className="fh-summary">{meta.summaryText}{meta.summaryPartial ? <span className="fh-summary-hatch" role="img" aria-label={meta.summaryPartial} title={meta.summaryPartial} /> : null}</span>
    : null;
  const rowsFocus = (adapter.mirrorFocus ?? 'body') === 'rows';
  const bodyRole = mirrorKind === 'listbox' ? 'listbox' : adapter.navigation === 'grid' ? 'grid' : 'region';
  const bodyFocus = rowsFocus
    ? {}
    : { tabIndex: blockedState ? -1 : 0, role: bodyRole, 'aria-label': label, onFocus, onBlur, onKeyDown };

  return (
    <div ref={frameRef} className={className ? `fh-frame ${className}` : 'fh-frame'}>
      {headerSlot && summary ? createPortal(summary, headerSlot) : null}
      {meta.notes.length > 0 && !bodyStateNode ? <WidgetCaption>{meta.notes.join(' ')}</WidgetCaption> : null}
      {Chrome ? (
        <Chrome
          state={chromeState}
          meta={meta}
          settings={settings}
          {...(onSettings ? { onSettings } : {})}
          editable={editableSettings}
          viewportWidth={chromeWidth}
          bodyBlocked={!!bodyStateNode}
          announce={say}
          targets={targets}
          headerSlot={headerSlot}
        />
      ) : null}
      <div ref={bodyWrapRef} className="fh-body-wrap">
        <div
          ref={bodyRef}
          className={scrollMode === 'none' ? 'fh-body' : 'fh-body themed-scrollbar'}
          data-scroll={scrollMode}
          data-clickable={onActivate ? 'true' : undefined}
          {...bodyFocus}
        >
          {mirrorKind === 'listbox' ? <div ref={listboxMirrorRef} className="fh-sr-only" /> : null}
        </div>
        <div ref={regionDropRef} className="fh-region-drop" hidden />
        {bodyStateNode ? <div className="fh-overlay">{bodyStateNode}</div> : null}
      </div>
      <div ref={descriptionRef} id={`${adapter.family}-${instanceId}-description`} className="fh-sr-only" />
      {mirrorKind === 'table' ? (
        <table
          ref={accessibleTableRef}
          className="fh-sr-only"
          aria-label={label}
          role={adapter.tree ? 'treegrid' : undefined}
          {...(rowsFocus ? { onFocus: onRowFocus, onBlur: onRowBlur, onKeyDown } : {})}
        >
          <thead ref={mirrorHeadRef} />
          <tbody ref={mirrorBodyRef} />
        </table>
      ) : null}
      {!hasFrame ? <div ref={liveRegionRef} className="fh-sr-only" role="status" aria-live="polite" aria-atomic="true" /> : null}
      {contextMenu.menu}
    </div>
  );
}
