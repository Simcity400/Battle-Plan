import { useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from 'react';
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
  DisplayedHour,
  EntityKind,
  FamilyProjection,
  FamilySourceSubscription,
  InstalledAction,
  InstalledActionTarget,
  InstalledPicture,
  InstalledTransform,
  MirrorModel,
  SourcePlan,
} from '../contract.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import type { CountryLooksMap } from '../looks.ts';
import type {
  FamilyArtDemand,
  FamilyContextHit,
  FamilyGestureContext,
  FamilyHostAdapter,
  FamilyMetaState,
  FamilySlotRenderContext,
  FamilyStaticSources,
  FamilyViewportChange,
  FamilyViewportState,
  FamilyViewWindow,
  ParsedFamilyHit,
} from './adapter.ts';
import { createPlanAuditVerifier, type FamilyAuditVerifier, type PlanAuditSnapshot } from './auditBinding.ts';
import { renderFamilyBodyState } from './FamilyBodyState.tsx';
import {
  handleFamilyGridKeyDown,
  handleFamilyKeyDown,
  handleFamilyPointsKeyDown,
  type FamilyNavTuple,
} from './keyboard.ts';
import { syncListboxMirror, syncTableMirror, type ListboxMirrorItem } from './mirror.ts';
import { bindFamilyPointer, parseDefaultRowHit } from './pointer.ts';
import {
  computeScrollGeometry,
  physicalToLogicalTop,
  scrollRangeIntoView,
  type ScrollGeometry,
} from './scroll.ts';
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
const NO_EXTRA = {} as Record<string, never>;
const DEFAULT_PLAN_VERIFIER = createPlanAuditVerifier<unknown, unknown>();
const navActionIndexCache = new WeakMap<readonly FamilyNavTuple[], Map<string, FamilyNavTuple>>();

export function lazyActionKeys(
  nav: readonly FamilyNavTuple[],
  hour: DisplayedHour,
): ReadonlyMap<string, InstalledActionTarget> {
  let materialized: Map<string, InstalledActionTarget> | null = null;
  const toTarget = (entry: FamilyNavTuple): InstalledActionTarget => ({
    subject: entry[0],
    hour,
    ...(entry[3] >= 0 ? { toggle: true } : {}),
    label: entry[1],
    meta: entry[5],
  });
  const index = () => {
    let idx = navActionIndexCache.get(nav);
    if (!idx) {
      idx = new Map();
      for (let i = 0; i < nav.length; i++) {
        const e = nav[i]!;
        idx.set(e[0], e);
      }
      navActionIndexCache.set(nav, idx);
    }
    return idx;
  };
  const ensureAll = () => {
    if (!materialized) {
      materialized = new Map();
      for (let i = 0; i < nav.length; i++) {
        const e = nav[i]!;
        materialized.set(e[0], toTarget(e));
      }
    }
    return materialized;
  };
  return {
    get size() { return nav.length; },
    has(key: string) { return index().has(key); },
    get(key: string) {
      const entry = index().get(key);
      return entry ? toTarget(entry) : undefined;
    },
    forEach(cb, thisArg) { ensureAll().forEach(cb, thisArg); },
    entries() { return ensureAll().entries(); },
    keys() { return ensureAll().keys(); },
    values() { return ensureAll().values(); },
    [Symbol.iterator]() { return ensureAll()[Symbol.iterator](); },
  };
}

function sameArray(a: readonly unknown[], b: readonly unknown[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

function defaultParseGridKey(key: string | null): { readonly row: number; readonly column: number } | null {
  if (!key) return null;
  const idx = key.indexOf(':');
  if (idx <= 0) return null;
  const row = Number(key.slice(0, idx));
  const column = Number(key.slice(idx + 1));
  return Number.isInteger(row) && Number.isInteger(column) && row >= 0 && column >= 0 ? { row, column } : null;
}

function defaultFormatGridKey(row: number, column: number): string {
  return `${row}:${column}`;
}

function revealGridAxis(index: number, cellSize: number, scroll: number, span: number): number {
  const start = index * cellSize;
  const end = start + cellSize;
  if (start < scroll) return start;
  if (end > scroll + span) return Math.max(0, end - span);
  return scroll;
}

function defaultListboxItems(mirror: MirrorModel, selectedKey: string | null): readonly ListboxMirrorItem[] {
  return mirror.rows.map(row => ({
    key: row.key,
    spoken: [row.name, ...row.cells.map(c => c.spoken)].filter(Boolean).join(', '),
    index: row.rowIndex,
    selected: selectedKey === row.key,
  }));
}

export interface FamilyHostProps<
  TModel = unknown,
  TOptions = unknown,
  TBinding = unknown,
  TSlotState = undefined,
  TExtra = Record<string, never>,
> {
  readonly adapter: FamilyHostAdapter<TModel, TOptions, TBinding, TSlotState, TExtra>;
  readonly declaration: FamilyProjection<TModel, TOptions>;
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
  readonly spriteResolutions?: Readonly<Record<string, ArtResolution>>;
  readonly extent?: readonly [number, number] | null;
  readonly plan?: SourcePlan | null;
  readonly exactReadView?: (hour: number) => Promise<BoundedReadView | null>;
  readonly offset: number;
  readonly pin: number | null;
  readonly bounds?: readonly [number, number] | null;
  readonly countryColours?: ReadonlyMap<string, string>;
  readonly countryLooks?: CountryLooksMap;
  readonly identitySnapshot?: IdentitySnapshot | null;
  readonly onIdentityDemands?: (demands: readonly (readonly [EntityKind, string, number])[]) => void;
  readonly onArtDemands?: (demands: readonly FamilyArtDemand[]) => void;
  readonly onViewportSize?: (width: number, height: number) => void;
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
  readonly extra?: TExtra;
  readonly extraVersion?: unknown;
}

export default function FamilyHost<
  TModel = unknown,
  TOptions = unknown,
  TBinding = unknown,
  TSlotState = undefined,
  TExtra = Record<string, never>,
>({
  adapter,
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
  tiledSeries,
  tiledRevision = 0,
  hovers,
  spriteResolutions,
  extent = null,
  plan = null,
  exactReadView,
  offset,
  pin,
  bounds = null,
  countryColours,
  countryLooks,
  identitySnapshot = null,
  onIdentityDemands,
  onArtDemands,
  onViewportSize,
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
  extra,
  extraVersion,
}: FamilyHostProps<TModel, TOptions, TBinding, TSlotState, TExtra>) {
  const resolvedExtra = (extra ?? NO_EXTRA) as TExtra;
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

  const scrollMode = adapter.scroll ?? 'none';
  const mirrorKind = adapter.mirror ?? 'none';
  const focusTarget = adapter.focusTarget ?? 'body';
  const pointerMode = adapter.pointerMode ?? 'discrete';

  const viewportRef = useRef<{
    width: number;
    height: number;
    dpr: number;
    scrollLeft: number;
    scrollTop: number;
    logicalScrollTop: number;
    zoomWindow: readonly [number, number] | null;
  }>({
    width: 0,
    height: 0,
    dpr: typeof devicePixelRatio === 'number' ? devicePixelRatio : 1,
    scrollLeft: 0,
    scrollTop: 0,
    logicalScrollTop: 0,
    zoomWindow: null,
  });
  const scrollGeometryRef = useRef<ScrollGeometry>(computeScrollGeometry(0, 0));
  const viewWindowRef = useRef<FamilyViewWindow | null>(null);
  const lastViewportReportRef = useRef<string>('');

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
  const installedFieldsRef = useRef<Readonly<Record<string, unknown>>>({});
  const navRef = useRef<readonly FamilyNavTuple[]>([]);
  const typeaheadRef = useRef({ query: '', until: 0 });

  const lastMirrorKeyRef = useRef('');
  const lastMirrorObjRef = useRef<MirrorModel | null>(null);
  const lastNavFieldRef = useRef<unknown>(null);
  const lastTableInfoRef = useRef<unknown>(null);
  const lastVisibleRef = useRef<boolean | null>(null);
  const lastExtentRef = useRef({ width: -1, height: -1, rowCount: -1 });
  const lastMetaKeyRef = useRef<{ mode: string; reason: string | null; failureOrigin: FailureOrigin; rawMeta: unknown } | null>(null);
  const metaSigRef = useRef('');
  const lastPhasesRef = useRef<unknown>(null);
  const lastIdentityDemandsRef = useRef<unknown>(null);
  const lastArtDemandsRef = useRef<unknown>(null);
  const lastAuditRef = useRef<{ readonly id: number; readonly cacheKey: readonly unknown[] } | null>(null);

  const [meta, setMeta] = useState<FamilyMetaState>(() => ({
    mode: staticSources ? 'picture' : 'reading',
    reason: null,
    failureOrigin: 'local',
    empty: null,
    emptyDetail: null,
    emptyKind: 'empty',
    summaryText: null,
    summaryPill: null,
    notes: [],
    warnings: [],
  }));
  const metaRef = useRef(meta);
  metaRef.current = meta;

  const [slotState, setSlotState] = useState<TSlotState | undefined>(undefined);
  const slotStateRef = useRef<TSlotState | undefined>(undefined);

  const { theme, reducedMotion } = useFamilyTheme(countryColours, countryLooks);
  const { announce, hasFrame, liveRegionRef } = useFamilyAnnouncer(adapter.family);

  const hasTiled = !!tiledSeries && Object.keys(tiledSeries).length > 0;
  const enabled = active && !prepared && !blockedState && (subscriptions.length > 0 || staticSources !== null || hasTiled || allowEmptyFollow);
  const auditable = !!exactReadView && (!!adapter.createAudit || !!adapter.deriveAuditRows);
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

  const latestRef = useRef({
    adapter,
    declaration,
    options,
    theme,
    reducedMotion,
    staticSources,
    tiledSeries,
    hovers,
    spriteResolutions,
    extent,
    plan,
    exactReadView,
    identitySnapshot,
    onIdentityDemands,
    onArtDemands,
    onViewportSize,
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
    extra: resolvedExtra,
  });
  latestRef.current = {
    adapter,
    declaration,
    options,
    theme,
    reducedMotion,
    staticSources,
    tiledSeries,
    hovers,
    spriteResolutions,
    extent,
    plan,
    exactReadView,
    identitySnapshot,
    onIdentityDemands,
    onArtDemands,
    onViewportSize,
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
    extra: resolvedExtra,
  };

  const auditUpdateRef = useRef(audit.update);
  auditUpdateRef.current = audit.update;

  const publishHoverHour = useCallback((nextHour: number | null) => {
    const normalized = nextHour !== null && Number.isFinite(nextHour) ? Math.floor(nextHour) : null;
    if (normalized === lastHoverHourRef.current) return;
    lastHoverHourRef.current = normalized;
    latestRef.current.onHoverHour?.(normalized);
  }, []);

  const notifyViewportChange = useCallback(() => {
    const cb = latestRef.current.onViewportChange;
    if (!cb) return;
    const vp = viewportRef.current;
    if (vp.width <= 0) return;
    const deviceWidth = Math.max(1, Math.round(vp.width * vp.dpr));
    const win = vp.zoomWindow;
    const sig = `${vp.width}|${vp.height}|${vp.dpr}|${win ? `${win[0]}:${win[1]}` : ''}`;
    if (sig === lastViewportReportRef.current) return;
    lastViewportReportRef.current = sig;
    cb({
      width: vp.width,
      height: vp.height,
      dpr: vp.dpr,
      deviceWidth,
      window: win,
    });
  }, []);

  const refreshFocusAnchor = useCallback(() => {
    const box = frameRef.current?.getBoundingClientRect();
    if (box) focusAnchorRef.current = { x: box.left + Math.min(200, box.width / 2), y: box.top + 48 };
  }, []);
  const refreshFocusAnchorRef = useRef(refreshFocusAnchor);
  refreshFocusAnchorRef.current = refreshFocusAnchor;

  const syncMirror = useCallback((mirror: MirrorModel | null) => {
    const curAdapter = latestRef.current.adapter;
    const kind = curAdapter.mirror ?? 'none';
    if (kind === 'table') {
      syncTableMirror(mirrorBodyRef.current, mirror, {
        selectedKey: latestRef.current.selectedKey ?? null,
        focusedKeyRef,
        focusInsideRef,
        restoringFocusRef,
        lastMirrorKeyRef,
      });
      return;
    }
    if (kind === 'listbox') {
      const items = mirror
        ? (curAdapter.toListboxItems ? curAdapter.toListboxItems(mirror, latestRef.current.selectedKey ?? null) : defaultListboxItems(mirror, latestRef.current.selectedKey ?? null))
        : null;
      const res = syncListboxMirror(listboxMirrorRef.current, items, {
        optionIdPrefix: `${curAdapter.family}-${instanceId}`,
        total: mirror?.total ?? 0,
        focusedKey: focusedKeyRef.current,
        lastMirrorKeyRef,
      });
      if (bodyRef.current && (curAdapter.focusTarget ?? 'body') === 'body') {
        if (res.activeDescendantId) bodyRef.current.setAttribute('aria-activedescendant', res.activeDescendantId);
        else bodyRef.current.removeAttribute('aria-activedescendant');
      }
    }
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
    const dpr = typeof devicePixelRatio === 'number' ? devicePixelRatio : 1;
    vp.dpr = dpr;

    const viewportSnap: FamilyViewportState = {
      width: w,
      height: h,
      dpr,
      scrollLeft: vp.scrollLeft,
      scrollTop: vp.scrollTop,
      logicalScrollTop: vp.logicalScrollTop,
      zoomWindow: vp.zoomWindow,
    };

    if (curAdapter.computeViewWindow) {
      viewWindowRef.current = curAdapter.computeViewWindow(viewportSnap, cur.extra);
    }

    let auditId: number | undefined;
    if (perfLog && cur.captureActive && cur.exactReadView) {
      if (curAdapter.createAudit) {
        const created = curAdapter.createAudit({
          projection: cur.declaration,
          options: cur.options,
          exactReadView: cur.exactReadView,
          viewport: viewportSnap,
          viewWindow: viewWindowRef.current,
          theme: cur.theme,
          extra: cur.extra,
        });
        if (created) {
          const prev = lastAuditRef.current;
          if (prev && sameArray(prev.cacheKey, created.cacheKey)) {
            auditId = prev.id;
          } else {
            auditId = audit.capture(created.snapshot, created.verify);
            lastAuditRef.current = { id: auditId, cacheKey: created.cacheKey };
          }
        }
      } else if (curAdapter.deriveAuditRows) {
        const cacheKey = [cur.declaration, cur.options, cur.exactReadView];
        const prev = lastAuditRef.current;
        if (prev && sameArray(prev.cacheKey, cacheKey)) {
          auditId = prev.id;
        } else {
          const deriveRows = curAdapter.deriveAuditRows;
          const ordered = curAdapter.auditOrdered;
          const snap: PlanAuditSnapshot<TModel, TOptions> = {
            projection: cur.declaration,
            options: cur.options,
            exactReadView: cur.exactReadView,
            deriveRows,
            ...(ordered !== undefined ? { ordered } : {}),
          };
          auditId = audit.capture(snap, DEFAULT_PLAN_VERIFIER as FamilyAuditVerifier<unknown>);
          lastAuditRef.current = { id: auditId, cacheKey };
        }
      }
    }

    const staticPanels = cur.staticSources?.panels;
    const staticAnswers = cur.staticSources?.answers;
    const bindingModel = curAdapter.buildBinding({
      kind: cur.declaration.kind,
      options: cur.options,
      sources: sourceIdsRef.current,
      ...(staticPanels && Object.keys(staticPanels).length ? { staticPanels } : {}),
      ...(staticAnswers && Object.keys(staticAnswers).length ? { staticAnswers } : {}),
      ...(cur.tiledSeries ? { tiledSeries: cur.tiledSeries } : {}),
      ...(cur.hovers ? { hovers: cur.hovers } : {}),
      ...(cur.spriteResolutions ? { spriteResolutions: cur.spriteResolutions } : {}),
      extent: cur.extent,
      plan: cur.plan,
      viewport: viewportSnap,
      viewWindow: viewWindowRef.current,
      theme: cur.theme,
      reducedMotion: cur.reducedMotion,
      identitySnapshot: cur.identitySnapshot,
      selectedKey: cur.selectedKey,
      focusedKey: focusInsideRef.current ? focusedKeyRef.current : null,
      focusInside: focusInsideRef.current,
      audit: !!(perfLog && cur.captureActive && auditId !== undefined),
      auditId,
      pauseFlush: pauseFlushRef.current,
      extra: cur.extra,
    });
    if (bindingModel === null) return;

    mounted.canvas.style.width = `${w}px`;
    mounted.canvas.style.height = `${h}px`;
    pipeline.present(pipe, curAdapter.family);
    const dated = Object.keys(sourceIdsRef.current).length === 0
      ? (cur.staticSources
        ? (cur.staticSources.provenInterval
          ? { h: cur.staticSources.provenInterval[0], c: cur.staticSources.provenInterval[1] }
          : { h: cur.staticSources.hour, c: cur.staticSources.hour })
        : (cur.allowEmptyFollow || (cur.tiledSeries && Object.keys(cur.tiledSeries).length > 0)) && cur.bounds
          ? { h: cur.bounds[0], c: cur.bounds[1] }
          : undefined)
      : undefined;

    pipeline.bind(
      pipe,
      [mounted.surface],
      {
        surfaces: [{ width: w, height: h, dpr }],
        ...(dated ? { dated } : {}),
      },
      {},
      bindingModel,
    );
  }, [pipeline, pipe, audit.capture, sourceIdsRef]);

  const bindRef = useRef(bind);
  bindRef.current = bind;

  const setZoomWindow = useCallback((next: readonly [number, number] | null) => {
    const prev = viewportRef.current.zoomWindow;
    if (prev === next || (prev && next && prev[0] === next[0] && prev[1] === next[1])) return;
    viewportRef.current.zoomWindow = next;
    notifyViewportChange();
    bindRef.current();
  }, [notifyViewportChange]);

  const makeGestureContext = useCallback((): FamilyGestureContext<TOptions, TExtra> => ({
    installed: installedPictureRef.current,
    fields: installedFieldsRef.current,
    viewport: { ...viewportRef.current },
    extent: latestRef.current.extent,
    options: latestRef.current.options,
    extra: latestRef.current.extra,
    setZoomWindow,
    requestBind: () => bindRef.current(),
  }), [setZoomWindow]);

  const dispatchAction = useCallback((action: InstalledAction, toggle: boolean) => {
    if ((toggle || action.toggle) && latestRef.current.onToggle) {
      latestRef.current.onToggle(action);
      return;
    }
    latestRef.current.onActivate?.(action);
  }, []);

  const openContextMenuFor = useCallback((
    key: string | null,
    clientX: number,
    clientY: number,
    hit: ParsedFamilyHit | null,
  ): boolean => {
    const installed = installedPictureRef.current;
    if (!installed) return false;
    const target = key ? installed.actionKeys.get(key) : undefined;
    const contextHit: FamilyContextHit = {
      key,
      subject: hit?.subject ?? target?.subject ?? key,
      label: hit?.label ?? target?.label ?? key,
      hour: hit?.hour ?? target?.hour ?? installed.hour,
      ...(hit?.kind !== undefined ? { kind: hit.kind } : {}),
      ...(hit?.meta !== undefined ? { meta: hit.meta } : target?.meta !== undefined ? { meta: target.meta } : {}),
      installed,
    };
    const items = latestRef.current.contextMenuItems?.(contextHit)
      ?? latestRef.current.adapter.contextMenuItems?.(contextHit, latestRef.current.extra)
      ?? [];
    if (!items.length) return false;
    contextMenuRef.current.open(clientX, clientY, items);
    return true;
  }, []);

  const focusRowByKey = useCallback((key: string) => {
    const entry = navRef.current.find(item => item[0] === key);
    if (!entry) return;
    focusedKeyRef.current = key;
    const stride = Math.max(1, latestRef.current.adapter.getRowStride?.(installedFieldsRef.current, latestRef.current.extra) ?? 28);
    scrollRangeIntoView(bodyRef.current, viewportRef.current, entry[2] * stride, stride, scrollGeometryRef.current);
    viewportRef.current.logicalScrollTop = physicalToLogicalTop(viewportRef.current.scrollTop, scrollGeometryRef.current);
    if ((latestRef.current.adapter.focusTarget ?? 'body') === 'mirror-rows') {
      const tr = mirrorBodyRef.current?.querySelector<HTMLTableRowElement>(`tr[data-row-key="${CSS.escape(key)}"]`);
      if (tr) {
        for (const sibling of mirrorBodyRef.current!.querySelectorAll<HTMLTableRowElement>('tr[tabindex="0"]')) {
          if (sibling !== tr) sibling.tabIndex = -1;
        }
        tr.tabIndex = 0;
        tr.focus({ preventScroll: true });
      }
    }
    bindRef.current();
  }, []);

  const focusGridCell = useCallback((row: number, column: number) => {
    const curAdapter = latestRef.current.adapter;
    const fields = installedFieldsRef.current;
    const metrics = curAdapter.getGridMetrics?.(fields);
    const fmt = curAdapter.formatGridKey ?? defaultFormatGridKey;
    const key = fmt(row, column);
    focusedKeyRef.current = key;
    const node = bodyRef.current;
    if (node && metrics) {
      const nextLeft = revealGridAxis(column, metrics.cellSize, node.scrollLeft, Math.max(1, viewportRef.current.width - metrics.gridLeft));
      const nextTop = revealGridAxis(row, metrics.cellSize, node.scrollTop, Math.max(1, viewportRef.current.height - metrics.gridTop));
      if (nextLeft !== node.scrollLeft) {
        node.scrollLeft = nextLeft;
        viewportRef.current.scrollLeft = nextLeft;
      }
      if (nextTop !== node.scrollTop) {
        node.scrollTop = nextTop;
        viewportRef.current.scrollTop = nextTop;
        viewportRef.current.logicalScrollTop = physicalToLogicalTop(nextTop, scrollGeometryRef.current);
      }
      const rLabel = metrics.rowLabel?.(row);
      const cLabel = metrics.columnLabel?.(column);
      if (rLabel && cLabel) announce(`${rLabel} to ${cLabel}`);
    }
    bindRef.current();
  }, [announce]);

  useLayoutEffect(() => {
    installedPictureRef.current = null;
    inspectedKeyRef.current = null;
    inspectedHitRef.current = null;
    viewportRef.current.zoomWindow = null;
    publishHoverHour(null);
  }, [resolvedOwner, publishHoverHour]);

  useLayoutEffect(() => {
    const node = bodyRef.current;
    if (!node || !pipeline || pipe === null) return;
    lastExtentRef.current = { width: -1, height: -1, rowCount: -1 };
    lastMetaKeyRef.current = null;
    lastMirrorObjRef.current = null;
    lastNavFieldRef.current = null;
    lastTableInfoRef.current = null;
    lastVisibleRef.current = null;
    lastPhasesRef.current = null;
    lastIdentityDemandsRef.current = null;
    lastArtDemandsRef.current = null;
    lastViewportReportRef.current = '';

    const curAdapter = latestRef.current.adapter;
    const canvasClass = curAdapter.classNames?.canvas ? `fh-canvas ${curAdapter.classNames.canvas}` : 'fh-canvas';
    const extentClass = curAdapter.classNames?.extent ? `fh-extent ${curAdapter.classNames.extent}` : 'fh-extent';
    const mounted = mountFamilyCanvas(node, pipeline, pipe, curAdapter.family, canvasClass, extentClass);
    if ((curAdapter.scroll ?? 'none') === 'none') {
      mounted.extent.style.width = '100%';
      mounted.extent.style.height = '100%';
    }
    surfaceRef.current = mounted;

    const auditWatch = attachAuditWatch(pipeline, pipe, {
      isAuditable: () => !!latestRef.current.exactReadView && (!!latestRef.current.adapter.createAudit || !!latestRef.current.adapter.deriveAuditRows),
      onCaptureChange: activeAudit => { latestRef.current.captureActive = activeAudit; },
      bindNow: () => bindRef.current(),
    });

    const install = () => {
      const shown = pipeline.view(pipe);
      const frameNode = frameRef.current;
      const cur = latestRef.current;
      const activeAdapter = cur.adapter;
      dimRef.current = !!shown?.dim;
      holdElement(frameNode, !!shown?.dim);
      const visible = shown?.mode === 'picture';
      if (lastVisibleRef.current !== visible) {
        lastVisibleRef.current = visible;
        node.toggleAttribute('data-family-visible', visible);
        node.toggleAttribute('data-family-pending', !visible);
      }

      const mode = shown?.mode ?? 'reading';
      const reason = shown?.reason ?? null;
      const failureOrigin: FailureOrigin = shown?.failureOrigin ?? 'local';
      const shownHour = reportFamilyShownHour(reportShownRef.current, visible, shown);

      auditWatch.settleIfReady(visible, mode, shown?.installedBinding, shown?.fields);

      const rawMeta = shown?.fields?.meta as Record<string, unknown> | undefined;
      const prevMetaKey = lastMetaKeyRef.current;
      if (
        !prevMetaKey
        || prevMetaKey.mode !== mode
        || prevMetaKey.reason !== reason
        || prevMetaKey.failureOrigin !== failureOrigin
        || prevMetaKey.rawMeta !== rawMeta
      ) {
        lastMetaKeyRef.current = { mode, reason, failureOrigin, rawMeta };
        const notes = Array.isArray(rawMeta?.notes)
          ? (rawMeta.notes as readonly string[])
          : (typeof rawMeta?.note === 'string' && rawMeta.note ? [rawMeta.note] : []);
        const nextMeta: FamilyMetaState = {
          mode,
          reason,
          failureOrigin,
          empty: (rawMeta?.empty as string | null | undefined) ?? null,
          emptyDetail: (rawMeta?.emptyDetail as string | null | undefined) ?? null,
          emptyKind: (rawMeta?.emptyKind as 'empty' | 'unavailable' | undefined) ?? 'empty',
          summaryText: (rawMeta?.summaryText as string | null | undefined) ?? null,
          summaryPill: (rawMeta?.summaryPill as string | null | undefined) ?? null,
          notes,
          warnings: (rawMeta?.warnings as readonly string[] | undefined) ?? [],
        };
        const nextSig = JSON.stringify(nextMeta);
        if (nextSig !== metaSigRef.current) {
          metaSigRef.current = nextSig;
          metaRef.current = nextMeta;
          setMeta(nextMeta);
        }
      }

      const idDemands = shown?.fields?.identityDemands as readonly (readonly [EntityKind, string, number])[] | undefined;
      if (idDemands && idDemands !== lastIdentityDemandsRef.current && cur.onIdentityDemands) {
        lastIdentityDemandsRef.current = idDemands;
        cur.onIdentityDemands(idDemands);
      }
      const artDemands = shown?.fields?.artDemands as readonly FamilyArtDemand[] | undefined;
      if (artDemands && artDemands !== lastArtDemandsRef.current && cur.onArtDemands) {
        lastArtDemandsRef.current = artDemands;
        cur.onArtDemands(artDemands);
      }

      if (!visible || !shown?.fields || shown.id === undefined) {
        installedPictureRef.current = null;
        installedFieldsRef.current = {};
        lastMirrorObjRef.current = null;
        syncMirrorRef.current(null);
        if (activeAdapter.onInstallFields) {
          activeAdapter.onInstallFields({
            fields: {},
            visible: false,
            frameNode,
            bodyNode: node,
            installed: null,
            extra: cur.extra,
            updateAudit: auditUpdateRef.current,
          });
        }
        if (activeAdapter.extractSlotState) {
          const nextSlot = activeAdapter.extractSlotState(undefined, viewportRef.current, metaRef.current, cur.extra, slotStateRef.current);
          if (nextSlot !== slotStateRef.current) {
            slotStateRef.current = nextSlot;
            setSlotState(nextSlot);
          }
        }
        tooltips.hideIfUnpinned();
        return;
      }

      const fields = shown.fields;
      installedFieldsRef.current = fields;

      const navField = fields.nav as readonly FamilyNavTuple[] | undefined;
      if (navField && navField !== lastNavFieldRef.current) {
        lastNavFieldRef.current = navField;
        navRef.current = navField;
      }
      const activeNav = navRef.current;
      const installedHour = displayedHour(shownHour ?? shown.painted ?? shown.h ?? 0);
      const actionKeys = activeAdapter.buildActionKeys
        ? activeAdapter.buildActionKeys(fields, installedHour)
        : lazyActionKeys(activeNav, installedHour);
      const metaTransform = rawMeta?.transform as InstalledTransform | undefined;
      const baseViewport = shown.viewport ?? {
        width: viewportRef.current.width,
        height: viewportRef.current.height,
        dpr: viewportRef.current.dpr,
        scrollLeft: viewportRef.current.scrollLeft,
        scrollTop: viewportRef.current.scrollTop,
      };
      const installedViewport = metaTransform && !baseViewport.transform
        ? { ...baseViewport, transform: metaTransform }
        : baseViewport;
      const installedAuditId = auditIdOf(fields.audit);
      const accessibleDescription = typeof fields.accessibleDescription === 'string' ? fields.accessibleDescription : null;
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
        viewport: installedViewport,
        optionsRevision: '',
        themeRevision: '',
        actionKeys,
        accessibleDescription,
        ...(installedAuditId !== undefined ? { auditId: installedAuditId } : {}),
      };
      installedPictureRef.current = installed;

      if (activeAdapter.getScrollExtent) {
        const extentVal = activeAdapter.getScrollExtent(fields, viewportRef.current, cur.extra);
        if (extentVal) {
          if (extentVal.width !== undefined) {
            const extW = Math.max(viewportRef.current.width, extentVal.width);
            if (lastExtentRef.current.width !== extW) {
              lastExtentRef.current.width = extW;
              mounted.extent.style.width = `${extW}px`;
            }
          }
          if (extentVal.height !== undefined) {
            const geom = computeScrollGeometry(extentVal.height, viewportRef.current.height);
            scrollGeometryRef.current = geom;
            viewportRef.current.logicalScrollTop = physicalToLogicalTop(viewportRef.current.scrollTop, geom);
            if (lastExtentRef.current.height !== geom.physicalHeight) {
              lastExtentRef.current.height = geom.physicalHeight;
              mounted.extent.style.height = `${geom.physicalHeight}px`;
            }
          }
        }
      }

      if (activeAdapter.onInstallFields) {
        activeAdapter.onInstallFields({
          fields,
          visible: true,
          frameNode,
          bodyNode: node,
          installed,
          extra: cur.extra,
          updateAudit: auditUpdateRef.current,
        });
      }

      if (activeAdapter.extractSlotState) {
        const nextSlot = activeAdapter.extractSlotState(fields, viewportRef.current, metaRef.current, cur.extra, slotStateRef.current);
        if (nextSlot !== slotStateRef.current) {
          slotStateRef.current = nextSlot;
          setSlotState(nextSlot);
        }
      }

      const tableInfo = fields.table as { count: number; focusedIndex: number; inspectedPresent: boolean } | undefined;
      if (tableInfo && (tableInfo !== lastTableInfoRef.current || focusInsideRef.current || inspectedKeyRef.current)) {
        lastTableInfoRef.current = tableInfo;
        if (lastExtentRef.current.rowCount !== tableInfo.count) {
          lastExtentRef.current.rowCount = tableInfo.count;
          accessibleTableRef.current?.setAttribute('aria-rowcount', String(tableInfo.count + 1));
        }
        if (focusInsideRef.current && focusedKeyRef.current) {
          const entry = navRef.current.find(item => item[0] === focusedKeyRef.current);
          if (entry) {
            const stride = Math.max(1, activeAdapter.getRowStride?.(fields, cur.extra) ?? 28);
            if (scrollRangeIntoView(bodyRef.current, viewportRef.current, entry[2] * stride, stride, scrollGeometryRef.current)) {
              viewportRef.current.logicalScrollTop = physicalToLogicalTop(viewportRef.current.scrollTop, scrollGeometryRef.current);
              queueMicrotask(() => bindRef.current());
            }
          } else if (activeAdapter.fallbackToFirstOnActivate !== false) {
            const fallbackKey = navRef.current[0]?.[0] ?? null;
            if (fallbackKey !== focusedKeyRef.current) {
              focusedKeyRef.current = fallbackKey;
              queueMicrotask(() => bindRef.current());
            }
          }
        }
        if (inspectedKeyRef.current && !tableInfo.inspectedPresent) {
          inspectedKeyRef.current = null;
          inspectedHitRef.current = null;
          publishHoverHour(null);
          pipeline.inspect(pipe, null);
          if (!focusInsideRef.current) tooltips.hideIfUnpinned();
        }
      }

      const mirror = fields.mirror as MirrorModel | undefined;
      if (mirror && mirror !== lastMirrorObjRef.current) {
        lastMirrorObjRef.current = mirror;
        syncMirrorRef.current(mirror);
      }

      const phases = fields.phases as Record<string, unknown> | undefined;
      if (phases && phases !== lastPhasesRef.current && frameNode && perfLog) {
        lastPhasesRef.current = phases;
        frameNode.dataset.familyPhases = JSON.stringify(phases);
      }

      const activeInspected = inspectedKeyRef.current;
      const activeFocusInside = focusInsideRef.current;
      if (!activeInspected && !activeFocusInside) {
        tooltips.hideIfUnpinned();
        return;
      }

      const hoverTipField = activeInspected ? (fields.tooltip as FamilyTooltipField | undefined) : undefined;
      const focusTipField = activeFocusInside ? (fields.focusTooltip as FamilyTooltipField | undefined) : undefined;
      if (
        activeFocusInside
        && focusedKeyRef.current
        && (activeAdapter.mirror ?? 'none') === 'table'
        && (activeAdapter.focusTarget ?? 'body') === 'mirror-rows'
        && focusTipField?.text
        && focusTipField.text.rowKey === focusedKeyRef.current
      ) {
        const tr = mirrorBodyRef.current?.querySelector<HTMLTableRowElement>(`tr[data-row-key="${CSS.escape(focusedKeyRef.current)}"]`);
        if (tr) {
          const desc = tooltips.describeFocused(focusTipField.text, installedHour, cur.viewportHeight);
          tr.setAttribute('aria-description', desc);
        }
      }
      const inspectedToken = inspectedHitRef.current
        ? (inspectedHitRef.current.inspectToken ?? (activeAdapter.inspectToken ? activeAdapter.inspectToken(inspectedHitRef.current.key) : `row:${inspectedHitRef.current.key}`))
        : null;
      tooltips.syncFromInstall({
        dim: !!shown.dim,
        held: heldRef.current,
        inspectedKey: activeInspected,
        inspectedToken,
        pointerPos: lastPointerRef.current,
        hoverTip: hoverTipField?.text ?? null,
        hoverFieldKey: hoverTipField?.key ?? null,
        keepHoverWhilePending: (activeAdapter.pointerMode ?? 'discrete') === 'continuous',
        focusInside: activeFocusInside,
        focusedKey: focusedKeyRef.current,
        focusAnchor: focusAnchorRef.current,
        focusTip: focusTipField?.text ?? null,
        viewportHeight: cur.viewportHeight,
      });
    };

    install();
    const stopInstall = pipeline.onInstall(pipe, install);

    const accessibleTargetRef = (curAdapter.mirror ?? 'none') === 'table'
      ? accessibleTableRef
      : (curAdapter.mirror ?? 'none') === 'listbox'
        ? listboxMirrorRef
        : bodyRef;

    const gestures = curAdapter.gestures
      ? {
          ...(curAdapter.gestures.onWheel ? { onWheel: (event: WheelEvent, pt: readonly [number, number]) => latestRef.current.adapter.gestures?.onWheel?.(event, pt, makeGestureContext()) } : {}),
          ...(curAdapter.gestures.onDragStart ? { onDragStart: (pt: readonly [number, number]) => latestRef.current.adapter.gestures?.onDragStart?.(pt, makeGestureContext()) } : {}),
          ...(curAdapter.gestures.onDragMove ? { onDragMove: (pt: readonly [number, number], delta: readonly [number, number]) => latestRef.current.adapter.gestures?.onDragMove?.(pt, delta, makeGestureContext()) } : {}),
          ...(curAdapter.gestures.onDragEnd ? { onDragEnd: () => latestRef.current.adapter.gestures?.onDragEnd?.(makeGestureContext()) } : {}),
          ...(curAdapter.gestures.onDoubleClick ? { onDoubleClick: (pt: readonly [number, number]) => latestRef.current.adapter.gestures?.onDoubleClick?.(pt, makeGestureContext()) } : {}),
        }
      : undefined;

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
      accessibleRef: accessibleTargetRef,
      pointerMode,
      isBlocked: () => heldRef.current || dimRef.current || !installedPictureRef.current,
      getInstalled: () => installedPictureRef.current,
      getViewportHeight: () => latestRef.current.viewportHeight,
      parseHit: (hitKey, inst) => (latestRef.current.adapter.parseHit
        ? latestRef.current.adapter.parseHit(hitKey, inst)
        : parseDefaultRowHit(hitKey)),
      inspectToken: key => (latestRef.current.adapter.inspectToken ? latestRef.current.adapter.inspectToken(key) : `row:${key}`),
      onHoverChange: hit => {
        publishHoverHour(hit?.hour ?? null);
      },
      gestures,
      onActivate: (key, isToggle, action, hit) => {
        const installed = installedPictureRef.current;
        if (!installed) return;
        if (action && (action.installedId !== installed.id || action.owner !== installed.owner)) return;
        const resolved = hit && latestRef.current.adapter.resolveHitAction
          ? latestRef.current.adapter.resolveHitAction(hit, installed, installedFieldsRef.current)
          : action;
        if (!resolved) {
          dispatchAction({
            installedId: installed.id,
            owner: installed.owner,
            subject: hit?.subject ?? key,
            hour: displayedHour(hit?.hour ?? installed.hour),
            ...(isToggle ? { toggle: true } : {}),
            ...(hit?.label !== undefined ? { label: hit.label } : {}),
            ...(hit?.meta !== undefined ? { meta: hit.meta } : {}),
          }, isToggle);
          return;
        }
        dispatchAction(resolved, isToggle);
      },
      onContextMenu: (key, clientX, clientY, hit) => openContextMenuFor(key, clientX, clientY, hit ?? null),
    });

    const onScroll = scrollMode !== 'none'
      ? () => {
          const sl = Math.floor(node.scrollLeft);
          const st = Math.floor(node.scrollTop);
          if (sl === viewportRef.current.scrollLeft && st === viewportRef.current.scrollTop) return;
          viewportRef.current.scrollLeft = sl;
          viewportRef.current.scrollTop = st;
          viewportRef.current.logicalScrollTop = physicalToLogicalTop(st, scrollGeometryRef.current);
          const curAdapterNow = latestRef.current.adapter;
          if (curAdapterNow.extractSlotState) {
            const nextSlot = curAdapterNow.extractSlotState(installedFieldsRef.current, viewportRef.current, metaRef.current, latestRef.current.extra, slotStateRef.current);
            if (nextSlot !== slotStateRef.current) {
              slotStateRef.current = nextSlot;
              setSlotState(nextSlot);
            }
          }
          bindRef.current();
        }
      : null;
    if (onScroll) node.addEventListener('scroll', onScroll, { passive: true });

    const measure = (rectWidth?: number, rectHeight?: number) => {
      const rawW = rectWidth ?? node.clientWidth;
      const rawH = rectHeight ?? node.clientHeight;
      latestRef.current.onViewportSize?.(rawW, rawH);
      const w = Math.round(rawW);
      const h = Math.round(rawH);
      refreshFocusAnchorRef.current();
      const dpr = typeof devicePixelRatio === 'number' ? devicePixelRatio : 1;
      if (w === viewportRef.current.width && h === viewportRef.current.height && dpr === viewportRef.current.dpr) return;
      viewportRef.current.width = w;
      viewportRef.current.height = h;
      viewportRef.current.dpr = dpr;
      const curAdapterNow = latestRef.current.adapter;
      if (curAdapterNow.extractSlotState) {
        const nextSlot = curAdapterNow.extractSlotState(installedFieldsRef.current, viewportRef.current, metaRef.current, latestRef.current.extra, slotStateRef.current);
        if (nextSlot !== slotStateRef.current) {
          slotStateRef.current = nextSlot;
          setSlotState(nextSlot);
        }
      }
      notifyViewportChange();
      bindRef.current();
    };

    const observer = typeof ResizeObserver === 'function'
      ? new ResizeObserver(entries => {
          const rect = entries[0]?.contentRect;
          measure(rect?.width, rect?.height);
        })
      : null;
    observer?.observe(node);
    viewportRef.current.width = node.clientWidth;
    viewportRef.current.height = node.clientHeight;
    viewportRef.current.dpr = typeof devicePixelRatio === 'number' ? devicePixelRatio : 1;
    latestRef.current.onViewportSize?.(node.clientWidth, node.clientHeight);
    refreshFocusAnchorRef.current();
    notifyViewportChange();
    bindRef.current();

    return () => {
      observer?.disconnect();
      if (onScroll) node.removeEventListener('scroll', onScroll);
      stopPointer();
      stopInstall();
      auditWatch.dispose();
      installedPictureRef.current = null;
      installedFieldsRef.current = {};
      publishHoverHour(null);
      mounted.unmount();
      surfaceRef.current = null;
      lastExtentRef.current = { width: -1, height: -1, rowCount: -1 };
      holdElement(frameRef.current, false);
      reportFamilyShownHour(reportShownRef.current, false, null);
    };
  }, [pipeline, pipe, instanceId, tooltips, scrollMode, pointerMode, makeGestureContext, dispatchAction, openContextMenuFor, publishHoverHour, notifyViewportChange]);

  useLayoutEffect(() => {
    if (pipe !== null && enabled) bind();
  }, [
    pipe,
    enabled,
    bind,
    declaration,
    options,
    theme,
    reducedMotion,
    staticSources,
    tiledSeries,
    tiledRevision,
    hovers,
    spriteResolutions,
    extent,
    identitySnapshot,
    selectedKey,
    subsReady,
    auditable,
    exactReadView,
    audit.captureActive,
    extraVersion,
  ]);

  useFamilyPauseFlush(runtime.clock, () => {
    if (!pipeline || pipe === null) return;
    pauseFlushRef.current++;
    bindRef.current();
    const view = pipeline.view(pipe);
    const mirror = view?.fields?.mirror as MirrorModel | undefined;
    if (mirror) syncMirrorRef.current(mirror);
  });

  useEffect(() => {
    if (!onDiagnostics) return;
    const all = [...(extraWarnings ?? []), ...meta.warnings];
    onDiagnostics(all);
    return () => onDiagnostics([]);
  }, [onDiagnostics, extraWarnings, meta.warnings]);

  const pinFocusedTooltip = useCallback((key: string | null) => {
    const installed = installedPictureRef.current;
    if (!pipeline || pipe === null || heldRef.current || !installed || !key) return;
    const focusTipField = pipeline.view(pipe)?.fields?.focusTooltip as FamilyTooltipField | undefined;
    if (focusTipField?.text && focusTipField.text.rowKey === key) {
      refreshFocusAnchor();
      tooltips.pinFocused(focusTipField.text, installed.hour, focusAnchorRef.current, latestRef.current.viewportHeight, announce);
    }
  }, [announce, pipe, pipeline, refreshFocusAnchor, tooltips]);

  const onKeyDown = useCallback((event: ReactKeyboardEvent) => {
    const installed = installedPictureRef.current;
    const cur = latestRef.current;
    const activeAdapter = cur.adapter;
    const fields = installedFieldsRef.current;
    const blocked = heldRef.current || dimRef.current || !installed;

    if (activeAdapter.navigation === 'none') {
      if (event.key === 'Escape' && tooltips.isPinned()) {
        event.preventDefault();
        tooltips.unpin(announce);
        return;
      }
      if ((event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) && !blocked) {
        refreshFocusAnchor();
        if (openContextMenuFor(null, focusAnchorRef.current.x, focusAnchorRef.current.y, null)) {
          event.preventDefault();
        }
      }
      return;
    }

    if (activeAdapter.navigation === 'grid') {
      const metrics = activeAdapter.getGridMetrics?.(fields);
      const parseKey = activeAdapter.parseGridKey ?? defaultParseGridKey;
      const formatKey = activeAdapter.formatGridKey ?? defaultFormatGridKey;
      const current = parseKey(focusedKeyRef.current) ?? { row: 0, column: 0 };
      const pageStep = activeAdapter.getPageStep
        ? activeAdapter.getPageStep(fields, viewportRef.current, cur.extra)
        : Math.max(1, Math.floor(Math.max(1, viewportRef.current.height - (metrics?.gridTop ?? 0)) / Math.max(1, metrics?.cellSize ?? 28)));
      handleFamilyGridKeyDown(event, {
        rows: metrics?.rows ?? 0,
        columns: metrics?.columns ?? 0,
        current,
        pageStep,
        blocked,
        isPinnedTooltip: () => tooltips.isPinned(),
        onFocusCell: focusGridCell,
        onActivate: (r, c) => {
          const curInstalled = installedPictureRef.current;
          if (!curInstalled) return;
          const key = formatKey(r, c);
          const hit: ParsedFamilyHit = { key };
          const resolved = activeAdapter.resolveHitAction?.(hit, curInstalled, fields)
            ?? {
              installedId: curInstalled.id,
              owner: curInstalled.owner,
              subject: curInstalled.actionKeys.get(key)?.subject ?? key,
              hour: curInstalled.actionKeys.get(key)?.hour ?? curInstalled.hour,
              meta: curInstalled.actionKeys.get(key)?.meta,
            };
          dispatchAction(resolved, false);
        },
        onContextMenu: (r, c) => {
          refreshFocusAnchor();
          const key = r >= 0 && c >= 0 ? formatKey(r, c) : null;
          return openContextMenuFor(key, focusAnchorRef.current.x, focusAnchorRef.current.y, key ? { key } : null);
        },
        onTogglePinTooltip: (r, c) => pinFocusedTooltip(formatKey(r, c)),
        onUnpinTooltip: () => tooltips.unpin(announce),
      });
      return;
    }

    if (activeAdapter.navigation === 'points') {
      const pageStep = activeAdapter.getPageStep
        ? activeAdapter.getPageStep(fields, viewportRef.current, cur.extra)
        : 10;
      handleFamilyPointsKeyDown(event, {
        nav: navRef.current,
        focusedKey: focusedKeyRef.current,
        pageStep,
        blocked,
        isPinnedTooltip: () => tooltips.isPinned(),
        onFocusKey: key => {
          focusedKeyRef.current = key;
          const parsed = activeAdapter.parseHit?.(key, installedPictureRef.current);
          if (parsed?.hour !== undefined) publishHoverHour(parsed.hour);
          bindRef.current();
        },
        onActivate: entry => {
          const curInstalled = installedPictureRef.current;
          if (!curInstalled) return;
          const target = curInstalled.actionKeys.get(entry[0]);
          const hit = activeAdapter.parseHit?.(entry[0], curInstalled) ?? { key: entry[0], label: entry[1], meta: entry[5] };
          const resolved = activeAdapter.resolveHitAction?.(hit, curInstalled, fields) ?? {
            installedId: curInstalled.id,
            owner: curInstalled.owner,
            subject: target?.subject ?? entry[0],
            hour: target?.hour ?? curInstalled.hour,
            label: target?.label ?? entry[1],
            meta: target?.meta ?? entry[5],
          };
          dispatchAction(resolved, false);
        },
        onContextMenu: entry => {
          refreshFocusAnchor();
          const key = entry ? entry[0] : null;
          const hit = key ? (activeAdapter.parseHit?.(key, installedPictureRef.current) ?? { key, label: entry?.[1], meta: entry?.[5] }) : null;
          return openContextMenuFor(key, focusAnchorRef.current.x, focusAnchorRef.current.y, hit);
        },
        onTogglePinTooltip: pinFocusedTooltip,
        onUnpinTooltip: () => tooltips.unpin(announce),
      });
      return;
    }

    const stride = Math.max(1, activeAdapter.getRowStride?.(fields, cur.extra) ?? 28);
    const pageStep = activeAdapter.getPageStep
      ? activeAdapter.getPageStep(fields, viewportRef.current, cur.extra)
      : Math.max(1, Math.floor(Math.max(1, viewportRef.current.height) / stride) - 1);
    handleFamilyKeyDown(event, {
      nav: navRef.current,
      focusedKey: focusedKeyRef.current,
      fallbackToFirstOnActivate: activeAdapter.fallbackToFirstOnActivate,
      pageStep,
      blocked,
      typeaheadRef,
      isPinnedTooltip: () => tooltips.isPinned(),
      onFocusKey: focusRowByKey,
      onActivate: entry => {
        const curInstalled = installedPictureRef.current;
        if (!curInstalled) return;
        const target = curInstalled.actionKeys.get(entry[0]);
        const hit: ParsedFamilyHit = {
          key: entry[0],
          subject: target?.subject ?? entry[0],
          hour: target?.hour ?? curInstalled.hour,
          label: target?.label ?? entry[1],
          meta: target?.meta ?? entry[5],
        };
        const resolved = activeAdapter.resolveHitAction?.(hit, curInstalled, fields) ?? {
          installedId: curInstalled.id,
          owner: curInstalled.owner,
          subject: hit.subject ?? entry[0],
          hour: displayedHour(hit.hour ?? curInstalled.hour),
          label: hit.label,
          meta: hit.meta,
        };
        dispatchAction(resolved, false);
      },
      onToggleTree: key => {
        const curInstalled = installedPictureRef.current;
        if (!curInstalled) return;
        const target = curInstalled.actionKeys.get(key);
        dispatchAction({
          installedId: curInstalled.id,
          owner: curInstalled.owner,
          subject: target?.subject ?? key,
          hour: target?.hour ?? curInstalled.hour,
          toggle: true,
          ...(target?.label !== undefined ? { label: target.label } : {}),
          ...(target?.meta !== undefined ? { meta: target.meta } : {}),
        }, true);
      },
      onContextMenu: entry => {
        refreshFocusAnchor();
        const key = entry ? entry[0] : null;
        return openContextMenuFor(key, focusAnchorRef.current.x, focusAnchorRef.current.y, key ? { key, label: entry?.[1], meta: entry?.[5] } : null);
      },
      onTogglePinTooltip: pinFocusedTooltip,
      onUnpinTooltip: () => tooltips.unpin(announce),
    });
  }, [announce, dispatchAction, focusGridCell, focusRowByKey, openContextMenuFor, pinFocusedTooltip, publishHoverHour, refreshFocusAnchor, tooltips]);

  const onRetryLocal = useCallback(() => retryLocal(() => bindRef.current()), [retryLocal]);

  const slotCtx = useMemo<FamilySlotRenderContext<TOptions, TSlotState, TExtra>>(() => ({
    meta,
    options,
    slotState,
    extra: resolvedExtra,
    bodyBlocked: false,
    announce,
    requestBind: () => bindRef.current(),
  }), [meta, options, slotState, resolvedExtra, announce]);

  if (blockedState) return <>{blockedState}</>;

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

  const renderSlotCtx: FamilySlotRenderContext<TOptions, TSlotState, TExtra> = bodyStateNode
    ? { ...slotCtx, bodyBlocked: true }
    : slotCtx;

  const headerPortalContent = adapter.renderHeaderSlot
    ? adapter.renderHeaderSlot(renderSlotCtx)
    : (meta.mode === 'picture' && !meta.empty && meta.summaryText
      ? <span className="fh-summary">{meta.summaryText}</span>
      : null);

  const frameClass = ['fh-frame', adapter.classNames?.frame, className].filter(Boolean).join(' ');
  const bodyClass = ['fh-body', adapter.classNames?.body].filter(Boolean).join(' ');
  const mirrorClass = ['fh-sr-only', adapter.classNames?.mirror].filter(Boolean).join(' ');
  const resolvedBodyRole = adapter.bodyRole
    ?? (mirrorKind === 'listbox' ? 'listbox' : adapter.navigation === 'grid' ? 'grid' : 'region');
  const frameAttrs = adapter.frameAttributes?.(resolvedExtra);
  const bodyAttrs = adapter.bodyAttributes?.(resolvedExtra);

  const bodyFocusHandlers = focusTarget === 'body'
    ? {
        tabIndex: 0,
        role: resolvedBodyRole,
        'aria-label': label,
        onFocus: () => {
          focusInsideRef.current = true;
          refreshFocusAnchor();
          if (adapter.navigation === 'grid' && !focusedKeyRef.current) {
            const metrics = adapter.getGridMetrics?.(installedFieldsRef.current);
            if (metrics && metrics.rows > 0 && metrics.columns > 0) {
              focusedKeyRef.current = (adapter.formatGridKey ?? defaultFormatGridKey)(0, 0);
            }
          } else if ((adapter.navigation === 'rows' || adapter.navigation === 'points') && !focusedKeyRef.current && navRef.current.length) {
            focusedKeyRef.current = navRef.current[0]![0];
          }
          bindRef.current();
        },
        onBlur: () => {
          focusInsideRef.current = false;
          if (!inspectedKeyRef.current) tooltips.hideIfUnpinned();
          bindRef.current();
        },
        onKeyDown,
      }
    : {};

  return (
    <div ref={frameRef} className={frameClass} {...frameAttrs}>
      {headerSlot && headerPortalContent ? createPortal(headerPortalContent, headerSlot) : null}
      {meta.notes.length > 0 && !bodyStateNode ? <WidgetCaption>{meta.notes.join(' ')}</WidgetCaption> : null}
      {adapter.renderBeforeBody?.(renderSlotCtx)}
      <div className="fh-body-wrap">
        <div
          ref={bodyRef}
          className={bodyClass}
          data-scroll={scrollMode}
          data-clickable={onActivate ? 'true' : undefined}
          {...bodyAttrs}
          {...bodyFocusHandlers}
        >
          {mirrorKind === 'listbox' ? <div ref={listboxMirrorRef} className={mirrorClass} /> : null}
        </div>
        {bodyStateNode ? <div className="fh-overlay">{bodyStateNode}</div> : null}
      </div>
      {adapter.renderAfterBody?.(renderSlotCtx)}
      {mirrorKind === 'table' ? (
        <table
          ref={accessibleTableRef}
          className={mirrorClass}
          aria-label={label}
          aria-rowcount={1}
          onFocus={focusTarget === 'mirror-rows'
            ? event => {
                if (restoringFocusRef.current) return;
                const wasInside = focusInsideRef.current;
                focusInsideRef.current = true;
                refreshFocusAnchor();
                const target = (event.target as HTMLElement | null)?.closest<HTMLTableRowElement>('tr[data-row-key]');
                if (target?.dataset.rowKey && (!wasInside || focusedKeyRef.current !== target.dataset.rowKey)) {
                  focusedKeyRef.current = target.dataset.rowKey;
                  bindRef.current();
                }
              }
            : undefined}
          onBlur={focusTarget === 'mirror-rows'
            ? event => {
                if (restoringFocusRef.current) return;
                if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
                focusInsideRef.current = false;
                if (!inspectedKeyRef.current) tooltips.hideIfUnpinned();
                bindRef.current();
              }
            : undefined}
          onKeyDown={focusTarget === 'mirror-rows' ? onKeyDown : undefined}
        >
          {adapter.renderMirrorHead?.(renderSlotCtx)}
          <tbody ref={mirrorBodyRef} />
        </table>
      ) : null}
      {!hasFrame ? <div ref={liveRegionRef} className="fh-sr-only" role="status" aria-live="polite" aria-atomic="true" /> : null}
      {contextMenu.menu}
    </div>
  );
}
