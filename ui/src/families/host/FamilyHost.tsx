import { useCallback, useContext, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { useContextMenu, type MenuItem } from '../../components/ContextMenu';
import { useViewportHeight } from '../../charts/useChartTokens';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { FailureOrigin } from '../../dashboard/follow/page.ts';
import type { WidgetSpec } from '../../dashboard/model';
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
import {
  lazyActionKeys,
  type FamilyArtDemand,
  type FamilyContextHit,
  type FamilyGestureContext,
  type FamilyHostAdapter,
  type FamilyMetaState,
  type FamilyStaticSources,
  type FamilyViewportChange,
  type FamilyViewportState,
  type ParsedFamilyHit,
} from './adapter.ts';
import { createPlanAuditVerifier, type FamilyAuditVerifier, type PlanAuditSnapshot } from './auditBinding.ts';
import { renderFamilyBodyState } from './FamilyBodyState.tsx';
import {
  handleFamilyGridKeyDown,
  handleFamilyKeyDown,
  handleFamilyPointsKeyDown,
  type FamilyNavTuple,
} from './keyboard.ts';
import { NO_EXPANSION, toggleExpansion, type FamilyExpansion } from './expansion.ts';
import { syncListboxMirror, syncTableMirror, type ListboxMirrorItem } from './mirror.ts';
import { bindFamilyPointer } from './pointer.ts';
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
const DEFAULT_ROW_STRIDE = 28;
const DEFAULT_POINTS_PAGE = 10;
const PLAN_VERIFIER = createPlanAuditVerifier<unknown, unknown>() as FamilyAuditVerifier<unknown>;
const EMPTY_NAV: readonly FamilyNavTuple[] = [];
const EMPTY_FIELDS: Readonly<Record<string, unknown>> = {};
const NO_STRINGS: readonly string[] = [];
const PICTURE_FIELDS = new WeakMap<InstalledPicture, Readonly<Record<string, unknown>>>();

interface RawMeta {
  readonly empty?: string | null;
  readonly emptyDetail?: string | null;
  readonly emptyKind?: 'empty' | 'unavailable';
  readonly summaryText?: string | null;
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

type HostCommand = 'activate' | 'toggle' | 'menu' | 'pin';

interface HostInput {
  readonly key: string | null;
  readonly hit: ParsedFamilyHit | null;
  readonly picture: InstalledPicture | null;
  readonly at?: { readonly x: number; readonly y: number };
}

function keyInPicture(adapter: FamilyHostAdapter<any, any, any>, picture: InstalledPicture, key: string): boolean {
  if (adapter.navigation !== 'grid') return picture.actionKeys.has(key);
  const metrics = adapter.getGridMetrics?.(PICTURE_FIELDS.get(picture) ?? EMPTY_FIELDS);
  const cell = (adapter.parseGridKey ?? parseGridKey)(key);
  return !!metrics && !!cell && cell.row < metrics.rows && cell.column < metrics.columns;
}

function currentDpr(): number {
  return typeof devicePixelRatio === 'number' ? devicePixelRatio : 1;
}

export interface FamilyHostComponentProps<TModel = unknown, TOptions = unknown, TBinding = unknown> {
  readonly adapter: FamilyHostAdapter<TModel, TOptions, TBinding>;
  readonly declaration: FamilyProjection<TModel, TOptions>;
  readonly view: string;
  readonly options: TOptions;
  readonly label: string;
  readonly subject: string;
  readonly widget: WidgetSpec;
  readonly runtime: WidgetRuntime;
  readonly active: boolean;
  readonly subscriptions: readonly FamilySourceSubscription[];
  readonly allowEmptyFollow?: boolean;
  readonly owner?: string;
  readonly staticSources?: FamilyStaticSources | null;
  readonly tiledSeries?: Readonly<Record<string, ReadonlyMap<string, TiledLine>>>;
  readonly tiledRevision?: number;
  readonly hovers?: Readonly<Record<string, { readonly hour: number; readonly data: unknown }>>;
  readonly artResolutions?: Readonly<Record<string, ArtResolution>>;
  readonly extent?: readonly [number, number] | null;
  readonly plan?: SourcePlan | null;
  readonly exactReadView?: (hour: number) => Promise<BoundedReadView | null>;
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
  readonly contextMenuItems?: (hit: FamilyContextHit) => MenuItem[];
  readonly headerSlot?: HTMLElement | null;
  readonly onDiagnostics?: (warnings: string[]) => void;
  readonly extraWarnings?: readonly string[];
  readonly blockedState?: ReactNode;
  readonly held?: boolean;
  readonly loadingLabel?: string;
  readonly className?: string;
}

export default function FamilyHost<TModel = unknown, TOptions = unknown, TBinding = unknown>({
  adapter,
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
  contextMenuItems,
  headerSlot = null,
  onDiagnostics,
  extraWarnings,
  blockedState = null,
  held = false,
  loadingLabel,
  className = '',
}: FamilyHostComponentProps<TModel, TOptions, TBinding>) {
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
  const restoringFocusRef = useRef(false);
  const inspectedKeyRef = useRef<string | null>(null);
  const inspectedHitRef = useRef<ParsedFamilyHit | null>(null);
  const lastPointerRef = useRef<{ x: number; y: number } | null>(null);
  const lastHoverHourRef = useRef<number | null>(null);
  const heldRef = useRef(held);
  heldRef.current = held;
  const dimRef = useRef(false);
  const pauseFlushRef = useRef(0);
  const installedPictureRef = useRef<InstalledPicture | null>(null);
  const installedFieldsRef = useRef<Readonly<Record<string, unknown>>>(EMPTY_FIELDS);
  const resolveInstalledRef = useRef<() => InstalledPicture | null>(() => installedPictureRef.current);
  const navRef = useRef<readonly FamilyNavTuple[]>(EMPTY_NAV);
  const typeaheadRef = useRef({ query: '', until: 0 });
  const expansionRef = useRef<FamilyExpansion>(NO_EXPANSION);

  const lastMirrorKeyRef = useRef('');
  const lastMirrorObjRef = useRef<MirrorModel | null>(null);
  const lastVisibleRef = useRef<boolean | null>(null);
  const lastExtentRef = useRef({ width: -1, height: -1 });
  const lastRawMetaRef = useRef<{ mode: string; reason: string | null; failureOrigin: FailureOrigin; raw: unknown } | null>(null);
  const lastPhasesRef = useRef<unknown>(null);
  const lastIdentityDemandsRef = useRef<unknown>(null);
  const lastArtDemandsRef = useRef<unknown>(null);
  const lastAuditRef = useRef<{ readonly id: number; readonly key: readonly unknown[] } | null>(null);

  const [meta, setMeta] = useState<FamilyMetaState>(() => ({
    mode: staticSources ? 'picture' : 'reading',
    reason: null,
    failureOrigin: 'local',
    empty: null,
    emptyDetail: null,
    emptyKind: 'empty',
    summaryText: null,
    notes: NO_STRINGS,
    warnings: NO_STRINGS,
  }));
  const metaRef = useRef(meta);
  metaRef.current = meta;

  const { theme, reducedMotion } = useFamilyTheme(undefined, countryLooks);
  const { announce, hasFrame, liveRegionRef } = useFamilyAnnouncer(adapter.family);

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
    declaration,
    view,
    options,
    theme,
    reducedMotion,
    staticSources,
    tiledSeries,
    hovers,
    artResolutions,
    extent,
    plan,
    exactReadView,
    identitySnapshot,
    onIdentityDemands,
    onArtDemands,
    onViewportChange,
    onHoverHour,
    selectedKey,
    onActivate,
    onToggle,
    contextMenuItems,
    viewportHeight,
    bounds,
    allowEmptyFollow,
    captureActive: audit.captureActive,
  };
  const latestRef = useRef(latest);
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

  const syncMirror = useCallback((mirror: MirrorModel | null) => {
    const kind = latestRef.current.adapter.mirror ?? 'none';
    const selected = latestRef.current.selectedKey ?? null;
    if (kind === 'table') {
      syncTableMirror(mirrorBodyRef.current, mirror, {
        selectedKey: selected,
        focusedKeyRef,
        focusInsideRef,
        restoringFocusRef,
        lastMirrorKeyRef,
      });
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

    let auditId: number | undefined;
    if (perfLog && cur.captureActive && cur.exactReadView && curAdapter.deriveAuditRows) {
      const key = [cur.declaration, cur.options, cur.exactReadView, curAdapter.deriveAuditRows, viewport.width, viewport.height, viewport.scrollLeft, viewport.scrollTop];
      const prev = lastAuditRef.current;
      if (prev && prev.key.length === key.length && prev.key.every((part, i) => part === key[i])) {
        auditId = prev.id;
      } else {
        const deriveRows = curAdapter.deriveAuditRows;
        const snapshot: PlanAuditSnapshot<TModel, TOptions> = {
          projection: cur.declaration,
          options: cur.options,
          exactReadView: cur.exactReadView,
          deriveRows: (model, hour) => deriveRows(model, hour, { viewport }),
          ...(curAdapter.auditOrdered !== undefined ? { ordered: curAdapter.auditOrdered } : {}),
        };
        auditId = audit.capture(snapshot, PLAN_VERIFIER);
        lastAuditRef.current = { id: auditId, key };
      }
    }

    const staticPanels = cur.staticSources?.panels;
    const staticAnswers = cur.staticSources?.answers;
    const binding = curAdapter.buildBinding({
      kind: cur.declaration.kind,
      view: cur.view,
      options: cur.options,
      sources: sourceIdsRef.current,
      ...(staticPanels && Object.keys(staticPanels).length ? { staticPanels } : {}),
      ...(staticAnswers && Object.keys(staticAnswers).length ? { staticAnswers } : {}),
      ...(cur.tiledSeries ? { tiledSeries: cur.tiledSeries } : {}),
      ...(cur.hovers ? { hovers: cur.hovers } : {}),
      ...(cur.artResolutions ? { artResolutions: cur.artResolutions } : {}),
      extent: cur.extent,
      plan: cur.plan,
      viewport,
      theme: cur.theme,
      reducedMotion: cur.reducedMotion,
      identitySnapshot: cur.identitySnapshot,
      selectedKey: cur.selectedKey,
      focusedKey: focusInsideRef.current ? focusedKeyRef.current : null,
      expanded: expansionRef.current,
      audit: auditId !== undefined,
      auditId,
      pauseFlush: pauseFlushRef.current,
    });
    if (binding === null) return;

    const widthPx = `${w}px`;
    const heightPx = `${h}px`;
    if (mounted.canvas.style.width !== widthPx) mounted.canvas.style.width = widthPx;
    if (mounted.canvas.style.height !== heightPx) mounted.canvas.style.height = heightPx;
    pipeline.present(pipe, curAdapter.family);
    const noSources = Object.keys(sourceIdsRef.current).length === 0;
    const dated = !noSources
      ? undefined
      : cur.staticSources
        ? (cur.staticSources.provenInterval
          ? { h: cur.staticSources.provenInterval[0], c: cur.staticSources.provenInterval[1] }
          : { h: cur.staticSources.hour, c: cur.staticSources.hour })
        : (cur.allowEmptyFollow || (cur.tiledSeries && Object.keys(cur.tiledSeries).length > 0)) && cur.bounds
          ? { h: cur.bounds[0], c: cur.bounds[1] }
          : undefined;
    pipeline.bind(pipe, [mounted.surface], { surfaces: [{ width: w, height: h, dpr }], ...(dated ? { dated } : {}) }, {}, binding);
  }, [pipeline, pipe, audit.capture, sourceIdsRef]);

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
    fields: installedFieldsRef.current,
    viewport: { ...viewportRef.current },
    extent: latestRef.current.extent,
    setZoomWindow,
  }), [setZoomWindow]);

  const dispatchAction = useCallback((action: InstalledAction) => {
    const cur = latestRef.current;
    if (action.toggle && cur.onToggle) cur.onToggle(action);
    else if (action.toggle) {
      const toggled = toggleExpansion(expansionRef.current, navRef.current, action.subject);
      if (!toggled) return;
      expansionRef.current = toggled.next;
      announce(`${toggled.label} ${toggled.open ? 'expanded' : 'collapsed'}.`);
      bindRef.current();
    } else cur.onActivate?.(action);
  }, [announce]);

  const actionFor = useCallback((installed: InstalledPicture, hit: ParsedFamilyHit, toggle: boolean): InstalledAction | null => {
    const refine = latestRef.current.adapter.resolveHitAction;
    if (refine) return refine(hit, installed, PICTURE_FIELDS.get(installed) ?? EMPTY_FIELDS);
    const target = installed.actionKeys.get(hit.key);
    const label = hit.label ?? target?.label;
    const meta = hit.meta ?? target?.meta;
    return {
      installedId: installed.id,
      owner: installed.owner,
      subject: hit.subject ?? target?.subject ?? hit.key,
      hour: displayedHour(hit.hour ?? target?.hour ?? installed.hour),
      ...(toggle || target?.toggle ? { toggle: true } : {}),
      ...(label !== undefined ? { label } : {}),
      ...(meta !== undefined ? { meta } : {}),
    };
  }, []);

  const dispatch = useCallback((command: HostCommand, input: HostInput): boolean => {
    const current = resolveInstalledRef.current();
    const picture = input.picture;
    if (heldRef.current || dimRef.current || !current || !picture || picture.id !== current.id || picture.owner !== current.owner) return false;
    const activeAdapter = latestRef.current.adapter;
    if (input.key !== null && !input.hit && !keyInPicture(activeAdapter, picture, input.key)) return false;
    if (command === 'menu') {
      const build = latestRef.current.contextMenuItems;
      if (!build) return false;
      const key = input.key;
      const hit = input.hit;
      const target = key ? picture.actionKeys.get(key) : undefined;
      const hitHour = hit?.hour ?? target?.hour ?? null;
      const items = build({
        key,
        subject: hit?.subject ?? target?.subject ?? key,
        label: hit?.label ?? target?.label ?? key,
        hour: hitHour ?? picture.hour,
        hitHour,
        ...(hit?.kind !== undefined ? { kind: hit.kind } : {}),
        ...(hit?.meta !== undefined ? { meta: hit.meta } : target?.meta !== undefined ? { meta: target.meta } : {}),
        installed: picture,
      });
      if (!items.length) return false;
      const at = input.at ?? focusAnchorRef.current;
      contextMenuRef.current.open(at.x, at.y, items);
      return true;
    }
    if (input.key === null) return false;
    if (command === 'pin') {
      const focusTip = (PICTURE_FIELDS.get(picture) ?? EMPTY_FIELDS).focusTooltip as FamilyTooltipField | undefined;
      if (!focusTip?.text || focusTip.text.rowKey !== input.key) return false;
      tooltips.pinFocused(focusTip.text, picture.hour, focusAnchorRef.current, latestRef.current.viewportHeight, announce);
      return true;
    }
    const toggle = command === 'toggle';
    const action = actionFor(picture, input.hit ?? activeAdapter.parseHit(input.key) ?? { key: input.key }, toggle);
    if (!action) return false;
    dispatchAction(toggle ? { ...action, toggle: true } : action);
    return true;
  }, [actionFor, announce, dispatchAction, tooltips]);

  const focusRowByKey = useCallback((key: string) => {
    const entry = navRef.current.find(item => item[0] === key);
    if (!entry) return;
    focusedKeyRef.current = key;
    const stride = Math.max(1, latestRef.current.adapter.getRowStride?.(installedFieldsRef.current) ?? DEFAULT_ROW_STRIDE);
    const top = entry[6] ?? entry[2] * stride;
    const height = entry[7] ?? stride;
    scrollRangeIntoView(bodyRef.current, viewportRef.current, top, height, scrollGeometryRef.current);
    bindRef.current();
  }, []);

  const focusGridCell = useCallback((row: number, column: number) => {
    const curAdapter = latestRef.current.adapter;
    const metrics = curAdapter.getGridMetrics?.(installedFieldsRef.current);
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
      if (rowLabel && columnLabel) announce(`${rowLabel} to ${columnLabel}`);
    }
    bindRef.current();
  }, [announce]);

  useLayoutEffect(() => {
    installedPictureRef.current = null;
    installedFieldsRef.current = EMPTY_FIELDS;
    navRef.current = EMPTY_NAV;
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
      const notes = raw?.notes ?? NO_STRINGS;
      const warnings = raw?.warnings ?? NO_STRINGS;
      const shown = metaRef.current;
      if (
        shown.mode === mode && shown.reason === reason && shown.failureOrigin === failureOrigin
        && shown.empty === empty && shown.emptyDetail === emptyDetail && shown.emptyKind === emptyKind
        && shown.summaryText === summaryText && sameStrings(shown.notes, notes) && sameStrings(shown.warnings, warnings)
      ) return;
      const next: FamilyMetaState = { mode, reason, failureOrigin, empty, emptyDetail, emptyKind, summaryText, notes, warnings };
      metaRef.current = next;
      setMeta(next);
    };

    const install = () => {
      const shown = pipeline.view(pipe);
      const frameNode = frameRef.current;
      const cur = latestRef.current;
      const activeAdapter = cur.adapter;
      dimRef.current = !!shown?.dim && shown.installedBinding === shown.binding;
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
        installedPictureRef.current = null;
        installedFieldsRef.current = EMPTY_FIELDS;
        navRef.current = EMPTY_NAV;
        describe(null);
        if (lastMirrorObjRef.current !== null) {
          lastMirrorObjRef.current = null;
          syncMirrorRef.current(null);
        }
        tooltips.hideIfUnpinned();
        return;
      }

      installedFieldsRef.current = fields;
      navRef.current = (fields.nav as readonly FamilyNavTuple[] | undefined) ?? EMPTY_NAV;
      const installedHour = displayedHour(shownHour ?? shown.painted ?? shown.h ?? 0);
      const base = shown.viewport ?? {
        width: viewportRef.current.width,
        height: viewportRef.current.height,
        dpr: viewportRef.current.dpr,
        scrollLeft: viewportRef.current.scrollLeft,
        scrollTop: viewportRef.current.scrollTop,
      };
      const installedAuditId = auditIdOf(fields.audit);
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
        actionKeys: lazyActionKeys(navRef.current, installedHour),
        accessibleDescription: typeof fields.accessibleDescription === 'string' ? fields.accessibleDescription : null,
        ...(installedAuditId !== undefined ? { auditId: installedAuditId } : {}),
      };
      PICTURE_FIELDS.set(installed, fields);
      installedPictureRef.current = installed;
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

      const mirror = fields.mirror as MirrorModel | undefined;
      if (mirror && mirror !== lastMirrorObjRef.current) {
        lastMirrorObjRef.current = mirror;
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
      const inspectedHit = inspectedHitRef.current;
      tooltips.syncFromInstall({
        dim: !!shown.dim,
        held: heldRef.current,
        inspectedKey: inspected,
        inspectedToken: inspectedHit ? (inspectedHit.inspectToken ?? activeAdapter.inspectToken(inspectedHit.key)) : null,
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
      if (shownId !== installedPictureRef.current?.id) install();
      return installedPictureRef.current;
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
      isBlocked: () => heldRef.current || dimRef.current || !installedPictureRef.current,
      getInstalled: resolveInstalled,
      getViewportHeight: () => latestRef.current.viewportHeight,
      parseHit: hitKey => latestRef.current.adapter.parseHit(hitKey),
      inspectToken: key => latestRef.current.adapter.inspectToken(key),
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
      onActivate: (key, toggle, _action, hit, picture) => { dispatch(toggle ? 'toggle' : 'activate', { key: hit?.key ?? key, hit: hit ?? null, picture: picture ?? null }); },
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
      resolveInstalledRef.current = () => installedPictureRef.current;
      describe(null);
      if (onScroll) node.removeEventListener('scroll', onScroll);
      clearHoverRef.current = () => {};
      stopPointer();
      stopInstall();
      auditWatch.dispose();
      installedPictureRef.current = null;
      installedFieldsRef.current = EMPTY_FIELDS;
      navRef.current = EMPTY_NAV;
      publishHoverHour(null);
      mounted.unmount();
      surfaceRef.current = null;
      holdElement(frameRef.current, false);
      reportFamilyShownHour(reportShownRef.current, false, null);
    };
  }, [pipeline, pipe, instanceId, tooltips, scrollMode, pointerMode, gestureContext, dispatch, publishHoverHour]);

  useLayoutEffect(() => {
    const node = bodyRef.current;
    if (!node) return;
    const measure = (width: number, height: number) => {
      refreshFocusAnchor();
      const w = Math.round(width);
      const h = Math.round(height);
      const dpr = currentDpr();
      const vp = viewportRef.current;
      if (w === vp.width && h === vp.height && dpr === vp.dpr) return;
      vp.width = w;
      vp.height = h;
      vp.dpr = dpr;
      notifyViewportChange();
      bindRef.current();
    };
    measure(node.clientWidth, node.clientHeight);
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
    staticSources,
    tiledSeries,
    tiledRevision,
    hovers,
    artResolutions,
    extent,
    plan,
    identitySnapshot,
    selectedKey,
    subsReady,
    auditable,
    exactReadView,
    audit.captureActive,
  ]);

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
    const fields = installedFieldsRef.current;
    const blocked = heldRef.current || dimRef.current || !picture;
    const run = (command: HostCommand, key: string | null) => {
      refreshFocusAnchor();
      return dispatch(command, { key, hit: null, picture });
    };
    const unpin = () => tooltips.unpin(announce);

    if (activeAdapter.navigation === 'grid') {
      const metrics = activeAdapter.getGridMetrics?.(fields);
      const parseKey = activeAdapter.parseGridKey ?? parseGridKey;
      const formatKey = activeAdapter.formatGridKey ?? formatGridKey;
      handleFamilyGridKeyDown(event, {
        rows: metrics?.rows ?? 0,
        columns: metrics?.columns ?? 0,
        current: parseKey(focusedKeyRef.current) ?? { row: 0, column: 0 },
        pageStep: activeAdapter.getPageStep?.(fields, viewportRef.current)
          ?? Math.max(1, Math.floor(Math.max(1, viewportRef.current.height - (metrics?.gridTop ?? 0)) / Math.max(1, metrics?.cellSize ?? DEFAULT_ROW_STRIDE))),
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
        nav: navRef.current,
        focusedKey: focusedKeyRef.current,
        pageStep: activeAdapter.getPageStep?.(fields, viewportRef.current) ?? DEFAULT_POINTS_PAGE,
        blocked,
        isPinnedTooltip: () => tooltips.isPinned(),
        onFocusKey: key => {
          focusedKeyRef.current = key;
          const hour = activeAdapter.parseHit(key)?.hour;
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

    const stride = Math.max(1, activeAdapter.getRowStride?.(fields) ?? DEFAULT_ROW_STRIDE);
    handleFamilyKeyDown(event, {
      nav: navRef.current,
      focusedKey: focusedKeyRef.current,
      pageStep: activeAdapter.getPageStep?.(fields, viewportRef.current) ?? Math.max(1, Math.floor(Math.max(1, viewportRef.current.height) / stride) - 1),
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
  }, [announce, dispatch, focusGridCell, focusRowByKey, publishHoverHour, refreshFocusAnchor, tooltips]);

  const onFocus = useCallback(() => {
    focusInsideRef.current = true;
    refreshFocusAnchor();
    const activeAdapter = latestRef.current.adapter;
    if (!focusedKeyRef.current) {
      if (activeAdapter.navigation === 'grid') {
        const metrics = activeAdapter.getGridMetrics?.(installedFieldsRef.current);
        if (metrics && metrics.rows > 0 && metrics.columns > 0) focusedKeyRef.current = (activeAdapter.formatGridKey ?? formatGridKey)(0, 0);
      } else if (navRef.current.length) {
        focusedKeyRef.current = navRef.current[0]![0];
      }
    }
    bindRef.current();
  }, [refreshFocusAnchor]);

  const onBlur = useCallback(() => {
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
  });

  const summary = meta.mode === 'picture' && !meta.empty && meta.summaryText
    ? <span className="fh-summary">{meta.summaryText}</span>
    : null;
  const bodyRole = mirrorKind === 'listbox' ? 'listbox' : adapter.navigation === 'grid' ? 'grid' : 'region';

  return (
    <div ref={frameRef} className={className ? `fh-frame ${className}` : 'fh-frame'}>
      {headerSlot && summary ? createPortal(summary, headerSlot) : null}
      {meta.notes.length > 0 && !bodyStateNode ? <WidgetCaption>{meta.notes.join(' ')}</WidgetCaption> : null}
      <div className="fh-body-wrap">
        <div
          ref={bodyRef}
          className="fh-body"
          data-scroll={scrollMode}
          data-clickable={onActivate ? 'true' : undefined}
          tabIndex={blockedState ? -1 : 0}
          role={bodyRole}
          aria-label={label}
          onFocus={onFocus}
          onBlur={onBlur}
          onKeyDown={onKeyDown}
        >
          {mirrorKind === 'listbox' ? <div ref={listboxMirrorRef} className="fh-sr-only" /> : null}
        </div>
        {bodyStateNode ? <div className="fh-overlay">{bodyStateNode}</div> : null}
      </div>
      <div ref={descriptionRef} id={`${adapter.family}-${instanceId}-description`} className="fh-sr-only" />
      {mirrorKind === 'table' ? (
        <table ref={accessibleTableRef} className="fh-sr-only" aria-label={label}>
          <tbody ref={mirrorBodyRef} />
        </table>
      ) : null}
      {!hasFrame ? <div ref={liveRegionRef} className="fh-sr-only" role="status" aria-live="polite" aria-atomic="true" /> : null}
      {contextMenu.menu}
    </div>
  );
}
