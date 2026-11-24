import { useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
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
import type { FollowedAnswer } from '../../dashboard/follow/message';
import { auditIdOf } from '../audit.ts';
import { displayedHour } from '../boundedView.ts';
import type { BoundedReadView, DisplayedHour, EntityKind, FamilySourceSubscription, InstalledAction, InstalledActionTarget, InstalledPicture, MirrorModel } from '../contract.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import {
  attachAuditWatch,
  bindFamilyPointer,
  handleFamilyKeyDown,
  mountFamilyCanvas,
  renderFamilyBodyState,
  reportFamilyShownHour,
  scrollRangeIntoView,
  syncTableMirror,
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
import type { CountryLooksMap } from '../looks.ts';
import type { FamilyTableBinding } from './laneView.ts';
import { buildColumnLayouts, layoutFamilyTable } from './layout.ts';
import { deriveTableAudit, resolveTableRows, type CountryColorSource, type TableColumnSpec, type TableProjection } from './model.ts';
import { ROW_HEIGHT, updateTableSettings, type TableSettings } from './settings.ts';
import { columnWidthVar, DISPLAY_LABELS, FamilyTableHeader, type TableHeaderColumn } from './TableHeader.tsx';
import './tableFamily.css';

const navActionIndexCache = new WeakMap<readonly FamilyNavTuple[], Map<string, FamilyNavTuple>>();

function lazyActionKeys(
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

export type { FamilySourceSubscription };

const TABLE_AUDIT = -1;
const OVERSCAN = 4;
const HEADER_FALLBACK_WIDTH = 120;

interface AuditSnapshot<TOptions> {
  readonly declaration: TableProjection<TOptions>;
  readonly options: TOptions;
  readonly settings: TableSettings;
  readonly expanded: Readonly<Record<string, boolean>>;
  readonly exactReadView: (hour: number) => Promise<BoundedReadView | null>;
  readonly width: number;
  readonly window: { readonly first: number; readonly count: number };
  readonly measuredWidths: Readonly<Record<string, number>>;
  readonly countryColors?: CountryColorSource;
}

async function verifyTableAuditSnapshot<TOptions>(hour: number, snap: AuditSnapshot<TOptions>) {
  const exact = await snap.exactReadView(hour);
  if (!exact) return null;
  const model = snap.declaration.project(exact, snap.options);
  const flatRows = resolveTableRows(model, snap.settings, snap.expanded);
  const rankShown = model.rank === true;
  const rankWidth = rankShown ? (model.rankMarkers ? 64 : 36) : 0;
  const allLayouts = buildColumnLayouts(model.columns, snap.settings, snap.measuredWidths);
  const protectedKeys = new Set<string>();
  if (!model.unsorted && snap.settings.sort.column) protectedKeys.add(snap.settings.sort.column);
  const computedLayout = layoutFamilyTable(allLayouts, Math.max(1, snap.width), snap.settings.overflow, {
    rankShown,
    rankWidth,
    protectedKeys,
  });
  const rows = deriveTableAudit({
    model,
    flatRows,
    visibleColumns: computedLayout.columns,
    hiddenKeys: new Set(computedLayout.hidden),
    rankShown,
    window: snap.window,
    countryColors: snap.countryColors,
  });
  return { values: new Map(rows.map(row => [row.key, row.value ?? ''])), ordered: true };
}

export interface FamilyTableHostProps<TOptions> {
  readonly declaration: TableProjection<TOptions>;
  readonly options: TOptions;
  readonly label: string;
  readonly subject: string;
  readonly widget: WidgetSpec;
  readonly runtime: WidgetRuntime;
  readonly active: boolean;
  readonly columns: readonly TableColumnSpec[];
  readonly settings: TableSettings;
  readonly onSettings?: (next: TableSettings) => void;
  readonly editableColumns?: boolean;
  readonly subscriptions: readonly FamilySourceSubscription[];
  readonly allowEmptyFollow?: boolean;
  readonly owner?: string;
  readonly staticSources?: {
    readonly panels?: Readonly<Record<string, unknown>>;
    readonly answers?: Readonly<Record<string, FollowedAnswer>>;
    readonly hour: number;
    readonly follows?: boolean;
    readonly provenInterval?: readonly [lo: number, hi: number];
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

interface TableMetaState {
  mode: string;
  reason: string | null;
  failureOrigin: FailureOrigin;
  empty: string | null;
  emptyDetail: string | null;
  emptyKind: 'empty' | 'unavailable';
  summaryPill: string | null;
  notes: readonly string[];
  warnings: readonly string[];
  hasTree: boolean;
  unsorted: boolean;
  rankShown: boolean;
  rankWidth: number;
  columns: readonly TableColumnSpec[];
}

interface WidthsState {
  widths: Record<string, number>;
  visible: readonly string[];
  hidden: readonly string[];
  totalWidth: number;
  scrollable: boolean;
  moreWidth: number;
  signature: string;
}

export default function FamilyTableHost<TOptions>({
  declaration,
  options,
  label,
  subject,
  widget,
  runtime,
  active,
  columns: initialColumns,
  settings,
  onSettings,
  editableColumns = true,
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
}: FamilyTableHostProps<TOptions>) {
  const prepared = useContext(PreparingPage);
  useWidgetSkeleton('table');
  const viewportHeight = useViewportHeight();
  const contextMenu = useContextMenu(`${label} actions`);
  const contextMenuRef = useRef(contextMenu);
  contextMenuRef.current = contextMenu;
  const frameRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const accessibleRef = useRef<HTMLTableElement>(null);
  const mirrorBodyRef = useRef<HTMLTableSectionElement>(null);

  const viewportRef = useRef({ width: 0, height: 0, scrollLeft: 0, scrollTop: 0 });
  const [viewportWidth, setViewportWidth] = useState(0);
  const [scrollLeft, setScrollLeft] = useState(0);
  const [resizing, setResizing] = useState<{ key: string; width: number } | null>(null);
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const expandedRef = useRef(expanded);
  expandedRef.current = expanded;

  const focusedKeyRef = useRef<string | null>(null);
  const focusInsideRef = useRef(false);
  const focusAnchorRef = useRef({ x: 0, y: 0 });
  const restoringFocusRef = useRef(false);
  const inspectedKeyRef = useRef<string | null>(null);
  const lastPointerRef = useRef<{ x: number; y: number } | null>(null);
  const heldRef = useRef(held);
  heldRef.current = held;
  const pauseFlushRef = useRef(0);
  const lastAuditRef = useRef<{
    id: number;
    declaration: TableProjection<TOptions>;
    options: TOptions;
    settings: TableSettings;
    expanded: Readonly<Record<string, boolean>>;
    exactReadView: (hour: number) => Promise<BoundedReadView | null>;
    width: number;
    windowFirst: number;
    windowCount: number;
    countryLooks: CountryLooksMap | undefined;
    countryPalette: Readonly<Record<string, string>> | undefined;
  } | null>(null);
  const measuredWidthsRef = useRef<Readonly<Record<string, number>>>({});
  const lastMirrorKeyRef = useRef('');
  const lastExtentRef = useRef({ height: -1, width: -1, count: -1 });
  const navRef = useRef<readonly FamilyNavTuple[]>([]);
  const installedPictureRef = useRef<InstalledPicture | null>(null);
  const typeaheadRef = useRef<{ query: string; until: number }>({ query: '', until: 0 });
  const viewWindowRef = useRef({ first: 0, count: 24 });

  const [meta, setMeta] = useState<TableMetaState>(() => ({
    mode: staticSources ? 'picture' : 'reading',
    reason: null,
    failureOrigin: 'local',
    empty: null,
    emptyDetail: null,
    emptyKind: 'empty',
    summaryPill: null,
    notes: [],
    warnings: [],
    hasTree: false,
    unsorted: false,
    rankShown: false,
    rankWidth: 0,
    columns: initialColumns,
  }));
  const metaSigRef = useRef('');
  const lastMetaKeyRef = useRef<{ mode: string; reason: string | null; failureOrigin: FailureOrigin; rawMeta: unknown } | null>(null);
  const lastWidthsRef = useRef<unknown>(null);
  const lastMirrorObjRef = useRef<MirrorModel | null>(null);
  const lastNavFieldRef = useRef<unknown>(null);
  const lastTableInfoRef = useRef<unknown>(null);
  const lastVisibleRef = useRef<boolean | null>(null);
  const lastPhasesRef = useRef<unknown>(null);
  const lastAuditUpdatedRef = useRef<{ id: number; widths: unknown } | null>(null);

  const [widthsState, setWidthsState] = useState<WidthsState>({
    widths: {},
    visible: initialColumns.map(c => c.key),
    hidden: [],
    totalWidth: 0,
    scrollable: false,
    moreWidth: 0,
    signature: '',
  });

  const { theme, reducedMotion } = useFamilyTheme(countryColours, countryLooks);
  const { announce, hasFrame, liveRegionRef } = useFamilyAnnouncer('table');

  const enabled = active && !prepared && !blockedState && (subscriptions.length > 0 || staticSources !== null || allowEmptyFollow);
  const auditable = !!exactReadView;
  const resolvedOwner = owner ?? `${widget.id}:${declaration.kind}:${subject}`;
  const placement = useFamilyPlacement({
    family: 'table',
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
    surfaceLabel: 'table',
  });
  const { pipeline, pipe, sourceIdsRef, subsReady, tooltipOwner, dataContextKey, retryStream, retryLocal } = placement;

  const tooltips = useFamilyTooltip(tooltipOwner, dataContextKey);
  useFamilyHold(frameRef, held, tooltips);

  const audit = useFamilyAudit<AuditSnapshot<TOptions>>({
    pipeline,
    pipe,
    ownerId: TABLE_AUDIT,
    auditable,
  });

  const changeSettings = useCallback((update: (next: TableSettings) => void) => {
    if (!onSettings) return;
    const next = updateTableSettings(settings, update);
    if (next.sort.column !== settings.sort.column || next.sort.direction !== settings.sort.direction) {
      const col = meta.columns.find(c => c.key === next.sort.column);
      announce(`Sorted by ${col?.label ?? next.sort.column}, ${next.sort.direction === 'descending' ? 'highest first' : 'lowest first'}.`);
    }
    onSettings(next);
  }, [settings, onSettings, meta.columns, announce]);

  const reportShown = useShownHour();
  const reportShownRef = useRef(reportShown);
  reportShownRef.current = reportShown;

  const surfaceRef = useRef<MountedFamilyCanvas | null>(null);
  const rowHeight = ROW_HEIGHT[settings.density];

  const latestRef = useRef({
    declaration,
    options,
    settings,
    theme,
    reducedMotion,
    staticSources,
    exactReadView,
    identitySnapshot,
    onIdentityDemands,
    onViewportSize,
    selectedKey,
    onActivate,
    contextMenuItems,
    viewportHeight,
    rowHeight,
    offset,
    bounds,
    allowEmptyFollow,
    captureActive: audit.captureActive,
    columns: initialColumns,
  });
  latestRef.current = {
    declaration,
    options,
    settings,
    theme,
    reducedMotion,
    staticSources,
    exactReadView,
    identitySnapshot,
    onIdentityDemands,
    onViewportSize,
    selectedKey,
    onActivate,
    contextMenuItems,
    viewportHeight,
    rowHeight,
    offset,
    bounds,
    allowEmptyFollow,
    captureActive: audit.captureActive,
    columns: meta.columns.length ? meta.columns : initialColumns,
  };

  const bind = useCallback(() => {
    const mounted = surfaceRef.current;
    if (!mounted || !pipeline || pipe === null) return;
    const { width, height, scrollLeft: sl, scrollTop: st } = viewportRef.current;
    const w = Math.max(1, width || 320);
    const h = Math.max(1, height || 200);
    const cur = latestRef.current;
    const staticPanels = cur.staticSources?.panels;
    const staticAnswers = cur.staticSources?.answers;
    let auditId: number | undefined;
    if (perfLog && cur.captureActive && cur.exactReadView) {
      const prev = lastAuditRef.current;
      const win = viewWindowRef.current;
      if (
        prev
        && prev.declaration === cur.declaration
        && prev.options === cur.options
        && prev.settings === cur.settings
        && prev.expanded === expandedRef.current
        && prev.exactReadView === cur.exactReadView
        && prev.width === w
        && prev.windowFirst === win.first
        && prev.windowCount === win.count
        && prev.countryLooks === cur.theme?.countryLooks
        && prev.countryPalette === cur.theme?.country
      ) {
        auditId = prev.id;
      } else {
        auditId = audit.capture({
          declaration: cur.declaration,
          options: cur.options,
          settings: cur.settings,
          expanded: expandedRef.current,
          exactReadView: cur.exactReadView,
          width: w,
          window: { first: win.first, count: win.count },
          measuredWidths: measuredWidthsRef.current,
          countryColors: cur.theme ? { countryLooks: cur.theme.countryLooks, country: cur.theme.country } : undefined,
        }, verifyTableAuditSnapshot);
        lastAuditRef.current = {
          id: auditId,
          declaration: cur.declaration,
          options: cur.options,
          settings: cur.settings,
          expanded: expandedRef.current,
          exactReadView: cur.exactReadView,
          width: w,
          windowFirst: win.first,
          windowCount: win.count,
          countryLooks: cur.theme?.countryLooks,
          countryPalette: cur.theme?.country,
        };
      }
    }
    const bindingModel: FamilyTableBinding<TOptions> = {
      family: 'table',
      kind: cur.declaration.kind,
      options: cur.options,
      settings: cur.settings,
      sources: sourceIdsRef.current,
      ...(staticPanels && Object.keys(staticPanels).length ? { staticPanels } : {}),
      ...(staticAnswers && Object.keys(staticAnswers).length ? { staticAnswers } : {}),
      width: w,
      height: h,
      scrollLeft: sl,
      scrollTop: st,
      theme: cur.theme,
      ...(cur.identitySnapshot ? { identitySnapshot: cur.identitySnapshot } : {}),
      selectedKey: cur.selectedKey,
      focusedKey: focusInsideRef.current ? focusedKeyRef.current : null,
      expanded: expandedRef.current,
      reducedMotion: cur.reducedMotion,
      window: viewWindowRef.current,
      pauseFlush: pauseFlushRef.current,
      ...(perfLog && cur.captureActive && auditId !== undefined ? { audit: true, auditId } : {}),
    };
    mounted.canvas.style.width = `${w}px`;
    mounted.canvas.style.height = `${h}px`;
    pipeline.present(pipe, 'table');
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

  const paintMirror = useCallback((mirror: MirrorModel | null) => {
    syncTableMirror(mirrorBodyRef.current, mirror, {
      selectedKey: latestRef.current.selectedKey ?? null,
      focusedKeyRef,
      focusInsideRef,
      restoringFocusRef,
      lastMirrorKeyRef,
    });
  }, []);
  const paintMirrorRef = useRef(paintMirror);
  paintMirrorRef.current = paintMirror;

  const toggleTreeKey = useCallback((key: string) => {
    const entry = navRef.current.find(item => item[0] === key);
    if (!entry || entry[3] < 0) return;
    const nextOpen = entry[3] === 0;
    setExpanded(prev => ({ ...prev, [key]: nextOpen }));
    announce(`${entry[1]} ${nextOpen ? 'expanded' : 'collapsed'}.`);
  }, [announce]);
  const toggleTreeKeyRef = useRef(toggleTreeKey);
  toggleTreeKeyRef.current = toggleTreeKey;

  const scrollRowIntoView = useCallback((index: number) => {
    const rh = latestRef.current.rowHeight;
    if (scrollRangeIntoView(bodyRef.current, viewportRef.current, index * rh, rh)) {
      bindRef.current();
    }
  }, []);
  const scrollRowIntoViewRef = useRef(scrollRowIntoView);
  scrollRowIntoViewRef.current = scrollRowIntoView;

  const refreshFocusAnchor = useCallback(() => {
    const box = frameRef.current?.getBoundingClientRect();
    if (box) focusAnchorRef.current = { x: box.left + Math.min(200, box.width / 2), y: box.top + 48 };
  }, []);
  const refreshFocusAnchorRef = useRef(refreshFocusAnchor);
  refreshFocusAnchorRef.current = refreshFocusAnchor;

  const auditUpdateRef = useRef(audit.update);
  auditUpdateRef.current = audit.update;

  const focusRowByKey = useCallback((key: string) => {
    const entry = navRef.current.find(item => item[0] === key);
    if (!entry) return;
    focusedKeyRef.current = key;
    scrollRowIntoView(entry[2]);
    const tr = mirrorBodyRef.current?.querySelector<HTMLTableRowElement>(`tr[data-row-key="${CSS.escape(key)}"]`);
    if (tr) {
      for (const sibling of mirrorBodyRef.current!.querySelectorAll<HTMLTableRowElement>('tr[tabindex="0"]')) {
        if (sibling !== tr) sibling.tabIndex = -1;
      }
      tr.tabIndex = 0;
      tr.focus({ preventScroll: true });
    }
    bindRef.current();
  }, [scrollRowIntoView]);

  useLayoutEffect(() => {
    const node = bodyRef.current;
    if (!node || !pipeline || pipe === null) return;
    lastExtentRef.current = { height: -1, width: -1, count: -1 };
    lastMetaKeyRef.current = null;
    lastWidthsRef.current = null;
    lastMirrorObjRef.current = null;
    lastNavFieldRef.current = null;
    lastTableInfoRef.current = null;
    lastVisibleRef.current = null;
    lastPhasesRef.current = null;
    lastAuditUpdatedRef.current = null;

    const mounted = mountFamilyCanvas(node, pipeline, pipe, 'table', 'tf-canvas', 'tf-extent');
    surfaceRef.current = mounted;

    const auditWatch = attachAuditWatch(pipeline, pipe, {
      isAuditable: () => !!latestRef.current.exactReadView,
      onCaptureChange: activeAudit => { latestRef.current.captureActive = activeAudit; },
      bindNow: () => bindRef.current(),
    });

    const activateHit = (rowKey: string, isToggle: boolean, action?: InstalledAction) => {
      const installed = installedPictureRef.current;
      if (!installed) return;
      if (action && (action.installedId !== installed.id || action.owner !== installed.owner)) return;
      const target = installed.actionKeys.get(rowKey);
      if (isToggle || target?.toggle) {
        toggleTreeKeyRef.current(rowKey);
        return;
      }
      latestRef.current.onActivate?.(target?.subject ?? rowKey, target?.meta, target?.hour ?? installed.hour);
    };

    const install = () => {
      const shown = pipeline.view(pipe);
      const frameNode = frameRef.current;
      holdElement(frameNode, !!shown?.dim);
      const visible = shown?.mode === 'picture';
      if (lastVisibleRef.current !== visible) {
        lastVisibleRef.current = visible;
        node.toggleAttribute('data-table-visible', visible);
        node.toggleAttribute('data-table-pending', !visible);
      }

      const mode = shown?.mode ?? 'reading';
      const reason = shown?.reason ?? null;
      const failureOrigin: FailureOrigin = shown?.failureOrigin ?? 'local';
      const shownHour = reportFamilyShownHour(reportShownRef.current, visible, shown);

      auditWatch.settleIfReady(visible, mode, shown?.installedBinding, shown?.fields);

      const rawMeta = shown?.fields?.meta as Omit<TableMetaState, 'mode' | 'reason' | 'failureOrigin'> | undefined;
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
          rawMeta?.summaryPill ?? null,
          rawMeta?.notes ?? [],
          rawMeta?.warnings ?? [],
          rawMeta?.hasTree ?? false,
          rawMeta?.unsorted ?? false,
          rawMeta?.rankShown ?? false,
          rawMeta?.rankWidth ?? 0,
          rawMeta?.columns?.map(c => [c.key, c.label, c.priority, c.as, c.offered, c.text ?? false, c.hideable ?? true, c.sortable ?? true, c.numeric ?? false, c.hint ?? '']) ?? [],
        ]);
        if (nextMetaSig !== metaSigRef.current) {
          metaSigRef.current = nextMetaSig;
          setMeta(prev => ({
            mode,
            reason,
            failureOrigin,
            empty: rawMeta?.empty ?? null,
            emptyDetail: rawMeta?.emptyDetail ?? null,
            emptyKind: rawMeta?.emptyKind ?? 'empty',
            summaryPill: rawMeta?.summaryPill ?? null,
            notes: rawMeta?.notes ?? [],
            warnings: rawMeta?.warnings ?? [],
            hasTree: rawMeta?.hasTree ?? false,
            unsorted: rawMeta?.unsorted ?? false,
            rankShown: rawMeta?.rankShown ?? false,
            rankWidth: rawMeta?.rankWidth ?? 0,
            columns: rawMeta?.columns?.length ? rawMeta.columns : prev.columns,
          }));
        }
      }

      const demandsField = shown?.fields?.identityDemands as readonly (readonly [EntityKind, string, number])[] | undefined;
      if (demandsField && latestRef.current.onIdentityDemands) {
        latestRef.current.onIdentityDemands(demandsField);
      }

      if (!visible || !shown?.fields || shown.id === undefined) {
        installedPictureRef.current = null;
        lastMirrorObjRef.current = null;
        paintMirrorRef.current(null);
        tooltips.hideIfUnpinned();
        return;
      }

      const navField = shown.fields.nav as readonly FamilyNavTuple[] | undefined;
      if (navField && navField !== lastNavFieldRef.current) {
        lastNavFieldRef.current = navField;
        navRef.current = navField;
      }
      const activeNav = navRef.current;
      const installedHour = displayedHour(shownHour ?? shown.painted ?? shown.h ?? 0);
      const actionKeys = lazyActionKeys(activeNav, installedHour);
      const installedViewport = shown.viewport ?? {
        width: viewportRef.current.width,
        height: viewportRef.current.height,
        dpr: typeof devicePixelRatio === 'number' ? devicePixelRatio : 1,
        scrollLeft: viewportRef.current.scrollLeft,
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

      const widths = shown.fields.widths as Omit<WidthsState, 'signature'> | undefined;
      if (widths && widths !== lastWidthsRef.current) {
        lastWidthsRef.current = widths;
        measuredWidthsRef.current = widths.widths;
        for (const [key, px] of Object.entries(widths.widths)) {
          frameNode?.style.setProperty(columnWidthVar(key), `${px}px`);
        }
        const extWidth = Math.max(viewportRef.current.width, widths.totalWidth);
        if (lastExtentRef.current.width !== extWidth) {
          lastExtentRef.current.width = extWidth;
          mounted.extent.style.width = `${extWidth}px`;
        }
        const sig = JSON.stringify([widths.visible, widths.hidden, widths.scrollable, widths.moreWidth, widths.widths]);
        setWidthsState(prev => prev.signature === sig ? prev : { ...widths, signature: sig });
      }

      if (perfLog && installedAuditId !== undefined && widths) {
        const prevUpdated = lastAuditUpdatedRef.current;
        if (!prevUpdated || prevUpdated.id !== installedAuditId || prevUpdated.widths !== widths.widths) {
          lastAuditUpdatedRef.current = { id: installedAuditId, widths: widths.widths };
          auditUpdateRef.current(installedAuditId, snap => ({ ...snap, measuredWidths: widths.widths }));
        }
      }

      const tableInfo = shown.fields.table as { count: number; focusedIndex: number; inspectedPresent: boolean } | undefined;
      if (tableInfo && (tableInfo !== lastTableInfoRef.current || focusInsideRef.current || inspectedKeyRef.current)) {
        lastTableInfoRef.current = tableInfo;
        if (lastExtentRef.current.count !== tableInfo.count) {
          lastExtentRef.current.count = tableInfo.count;
          accessibleRef.current?.setAttribute('aria-rowcount', String(tableInfo.count + 1));
        }
        const extHeight = Math.max(viewportRef.current.height, latestRef.current.rowHeight * tableInfo.count);
        if (lastExtentRef.current.height !== extHeight) {
          lastExtentRef.current.height = extHeight;
          mounted.extent.style.height = `${extHeight}px`;
        }
        if (focusInsideRef.current && focusedKeyRef.current) {
          const entry = navRef.current.find(item => item[0] === focusedKeyRef.current);
          if (entry) {
            scrollRowIntoViewRef.current(entry[2]);
          } else {
            const fallbackKey = navRef.current[0]?.[0] ?? null;
            if (fallbackKey !== focusedKeyRef.current) {
              focusedKeyRef.current = fallbackKey;
              queueMicrotask(() => bindRef.current());
            }
          }
        }
        if (inspectedKeyRef.current && !tableInfo.inspectedPresent) {
          inspectedKeyRef.current = null;
          pipeline.inspect(pipe, null);
          if (!focusInsideRef.current) tooltips.hideIfUnpinned();
        }
      }

      const mirror = shown.fields.mirror as MirrorModel | undefined;
      if (mirror && mirror !== lastMirrorObjRef.current) {
        lastMirrorObjRef.current = mirror;
        paintMirrorRef.current(mirror);
      }

      const phases = shown.fields.phases as Record<string, unknown> | undefined;
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

      const hoverTipField = activeInspected ? (shown.fields.tooltip as FamilyTooltipField | undefined) : undefined;
      const focusTipField = activeFocusInside ? (shown.fields.focusTooltip as FamilyTooltipField | undefined) : undefined;
      if (activeFocusInside && focusedKeyRef.current && focusTipField?.text && focusTipField.text.rowKey === focusedKeyRef.current) {
        const tr = mirrorBodyRef.current?.querySelector<HTMLTableRowElement>(`tr[data-row-key="${CSS.escape(focusedKeyRef.current)}"]`);
        if (tr) {
          const desc = tooltips.describeFocused(focusTipField.text, installedHour, latestRef.current.viewportHeight);
          tr.setAttribute('aria-description', desc);
        }
      }
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
      isBlocked: () => heldRef.current || !installedPictureRef.current,
      getInstalled: () => installedPictureRef.current,
      getViewportHeight: () => latestRef.current.viewportHeight,
      onActivate: activateHit,
      onContextMenu: (key, clientX, clientY) => {
        const installed = installedPictureRef.current;
        const builder = latestRef.current.contextMenuItems;
        if (!installed || !builder) return false;
        const target = key ? installed.actionKeys.get(key) : undefined;
        const items = builder(key, key ? (target?.label ?? key) : null, target?.hour ?? installed.hour);
        if (!items.length) return false;
        contextMenuRef.current.open(clientX, clientY, items);
        return true;
      },
    });

    const syncViewWindow = () => {
      const rh = latestRef.current.rowHeight;
      const first = Math.max(0, Math.floor(viewportRef.current.scrollTop / rh) - OVERSCAN);
      const count = Math.ceil(Math.max(1, viewportRef.current.height) / rh) + OVERSCAN * 2;
      viewWindowRef.current = { first, count };
    };

    const onScroll = () => {
      const sl = node.scrollLeft, st = node.scrollTop;
      const prevSl = viewportRef.current.scrollLeft;
      viewportRef.current.scrollLeft = sl;
      viewportRef.current.scrollTop = st;
      if (sl !== prevSl) setScrollLeft(sl);
      syncViewWindow();
      bindRef.current();
    };

    const resizeObserver = new ResizeObserver(entries => {
      const rect = entries[0]?.contentRect;
      const w = Math.round(rect ? rect.width : node.clientWidth);
      const h = Math.round(rect ? rect.height : node.clientHeight);
      refreshFocusAnchorRef.current();
      if (w === viewportRef.current.width && h === viewportRef.current.height) return;
      viewportRef.current.width = w;
      viewportRef.current.height = h;
      syncViewWindow();
      setViewportWidth(w);
      latestRef.current.onViewportSize?.(w, h);
      bindRef.current();
    });
    resizeObserver.observe(node);
    viewportRef.current.width = node.clientWidth;
    viewportRef.current.height = node.clientHeight;
    syncViewWindow();
    refreshFocusAnchorRef.current();
    setViewportWidth(node.clientWidth);
    latestRef.current.onViewportSize?.(node.clientWidth, node.clientHeight);

    node.addEventListener('scroll', onScroll, { passive: true });
    bindRef.current();

    return () => {
      auditWatch.dispose();
      stopInstall();
      stopPointer();
      installedPictureRef.current = null;
      resizeObserver.disconnect();
      node.removeEventListener('scroll', onScroll);
      mounted.unmount();
      surfaceRef.current = null;
      lastExtentRef.current = { height: -1, width: -1, count: -1 };
      holdElement(frameRef.current, false);
    };
  }, [pipeline, pipe, tooltips]);

  useLayoutEffect(() => {
    const rh = ROW_HEIGHT[settings.density];
    const first = Math.max(0, Math.floor(viewportRef.current.scrollTop / rh) - OVERSCAN);
    const count = Math.ceil(Math.max(1, viewportRef.current.height) / rh) + OVERSCAN * 2;
    if (viewWindowRef.current.first !== first || viewWindowRef.current.count !== count) {
      viewWindowRef.current = { first, count };
      if (pipe !== null && enabled) bind();
    }
  }, [settings.density, pipe, enabled, bind]);

  useLayoutEffect(() => {
    if (pipe !== null && enabled) bind();
  }, [pipe, enabled, bind, options, settings, theme, reducedMotion, staticSources, identitySnapshot, selectedKey, expanded, subsReady, auditable, exactReadView, audit.captureActive]);

  useFamilyPauseFlush(runtime.clock, () => {
    if (!pipeline || pipe === null) return;
    pauseFlushRef.current++;
    bindRef.current();
    const view = pipeline.view(pipe);
    const mirror = view?.fields?.mirror as MirrorModel | undefined;
    if (mirror) paintMirror(mirror);
  });

  useEffect(() => {
    if (!onDiagnostics) return;
    const all = [...(extraWarnings ?? []), ...meta.warnings];
    onDiagnostics(all);
    return () => onDiagnostics([]);
  }, [onDiagnostics, extraWarnings, meta.warnings]);

  const handleKeyDown = useCallback((event: React.KeyboardEvent) => {
    const installed = installedPictureRef.current;
    const pageStep = Math.max(1, Math.floor((viewportRef.current.height || 200) / latestRef.current.rowHeight) - 1);
    handleFamilyKeyDown(event, {
      nav: navRef.current,
      focusedKey: focusedKeyRef.current,
      pageStep,
      blocked: heldRef.current || !installed,
      typeaheadRef,
      isPinnedTooltip: () => tooltips.isPinned(),
      onFocusKey: focusRowByKey,
      onActivate: entry => {
        const cur = installedPictureRef.current;
        if (!cur) return;
        const target = cur.actionKeys.get(entry[0]);
        latestRef.current.onActivate?.(target?.subject ?? entry[0], target?.meta ?? entry[5], target?.hour ?? cur.hour);
      },
      onToggleTree: toggleTreeKey,
      onContextMenu: entry => {
        const cur = installedPictureRef.current;
        const builder = latestRef.current.contextMenuItems;
        if (!cur || !builder) return false;
        const target = entry ? cur.actionKeys.get(entry[0]) : undefined;
        const items = builder(entry ? entry[0] : null, entry ? (target?.label ?? entry[1]) : null, target?.hour ?? cur.hour);
        if (!items.length) return false;
        refreshFocusAnchor();
        contextMenuRef.current.open(focusAnchorRef.current.x, focusAnchorRef.current.y, items);
        return true;
      },
      onTogglePinTooltip: currentKey => {
        const cur = installedPictureRef.current;
        if (!pipeline || pipe === null || heldRef.current || !cur || !currentKey) return;
        const focusTipField = pipeline.view(pipe)?.fields?.focusTooltip as FamilyTooltipField | undefined;
        if (focusTipField?.text && focusTipField.text.rowKey === currentKey) {
          refreshFocusAnchor();
          tooltips.pinFocused(focusTipField.text, cur.hour, focusAnchorRef.current, latestRef.current.viewportHeight, announce);
        }
      },
      onUnpinTooltip: () => tooltips.unpin(announce),
    });
  }, [announce, focusRowByKey, pipe, pipeline, refreshFocusAnchor, toggleTreeKey, tooltips]);

  const activeColumns = meta.columns.length ? meta.columns : initialColumns;
  const specMap = useMemo(() => new Map(activeColumns.map(col => [col.key, col])), [activeColumns]);
  const layout = useMemo(() => {
    const base = buildColumnLayouts(activeColumns, settings, widthsState.widths).map(col => ({
      ...col,
      width: resizing?.key === col.key ? resizing.width : (widthsState.widths[col.key] ?? (col.width || HEADER_FALLBACK_WIDTH)),
    }));
    if (widthsState.signature) {
      const byKey = new Map(base.map(col => [col.key, col]));
      const visibleCols = widthsState.visible.map(key => byKey.get(key)).filter((col): col is NonNullable<typeof col> => col !== undefined);
      if (visibleCols.length === widthsState.visible.length) {
        const rank = meta.rankShown ? (meta.rankWidth || 36) : 0;
        const totalWidth = rank + widthsState.moreWidth + visibleCols.reduce((sum, col) => sum + col.width, 0);
        return {
          columns: visibleCols,
          hidden: [...widthsState.hidden],
          totalWidth,
          scrollable: widthsState.scrollable,
          pinnedWidth: rank + (visibleCols[0]?.width ?? 0),
          moreWidth: widthsState.moreWidth,
        };
      }
    }
    const protectedKeys = new Set<string>();
    if (!meta.unsorted && settings.sort.column) protectedKeys.add(settings.sort.column);
    return layoutFamilyTable(base, Math.max(1, viewportWidth || 320), settings.overflow, {
      rankShown: meta.rankShown,
      rankWidth: meta.rankWidth || 36,
      protectedKeys,
    });
  }, [activeColumns, settings, widthsState, resizing, meta.unsorted, meta.rankShown, meta.rankWidth, viewportWidth]);

  const headerColumns = useMemo<TableHeaderColumn[]>(() => layout.columns.map(col => {
    const spec = specMap.get(col.key);
    return {
      ...col,
      text: spec?.text,
      hideable: spec?.hideable,
      sortable: spec?.sortable,
      hint: spec?.hint,
      displays: spec && spec.offered.length > 1 ? spec.offered.map(display => [display, DISPLAY_LABELS[display]] as const) : [],
    };
  }), [layout.columns, specMap]);

  const onRetryLocal = useCallback(() => retryLocal(() => bindRef.current()), [retryLocal]);

  if (blockedState) return <>{blockedState}</>;

  const summaryPillNode = headerSlot && meta.mode === 'picture' && !meta.empty && meta.summaryPill
    ? createPortal(<span className="pill">{meta.summaryPill}</span>, headerSlot)
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

  const hiddenCols = settings.overflow === 'tooltip' ? (widthsState.hidden.length ? widthsState.hidden : layout.hidden) : [];

  return <div ref={frameRef} className={`tf-frame${className ? ` ${className}` : ''}`} data-density={settings.density} data-overflow={settings.overflow}>
    {summaryPillNode}
    {meta.notes.length > 0 && !bodyStateNode && <WidgetCaption>{meta.notes.join(' ')}</WidgetCaption>}
    <div className="tf-head-clip" style={bodyStateNode ? { display: 'none' } : undefined}>
      <FamilyTableHeader
        columns={headerColumns}
        allColumns={activeColumns}
        hiddenColumns={hiddenCols}
        settings={settings}
        unsorted={meta.unsorted}
        editableColumns={editableColumns}
        rankWidth={meta.rankShown ? meta.rankWidth : 0}
        pinned={layout.scrollable}
        scrollLeft={settings.overflow === 'scroll' ? scrollLeft : 0}
        onChange={changeSettings}
        onResizing={(key, nextWidth) => setResizing({ key, width: nextWidth })}
        onResize={(key, nextWidth) => {
          setResizing(null);
          changeSettings(draft => { if (draft.columns[key]) draft.columns[key]!.width = nextWidth; });
        }}
      />
    </div>
    <div className="tf-body-wrap">
      <div
        ref={bodyRef}
        className="tf-body"
        data-overflow={settings.overflow}
        data-clickable={onActivate ? 'true' : undefined}
      />
      {bodyStateNode && <div className="tf-empty-body">{bodyStateNode}</div>}
    </div>
    <table
      ref={accessibleRef}
      className="table-accessibility"
      role={meta.hasTree ? 'treegrid' : undefined}
      aria-label={label}
      aria-rowcount={1}
      onFocus={event => {
        if (restoringFocusRef.current) return;
        const wasInside = focusInsideRef.current;
        focusInsideRef.current = true;
        refreshFocusAnchor();
        const target = (event.target as HTMLElement | null)?.closest<HTMLTableRowElement>('tr[data-row-key]');
        if (target?.dataset.rowKey && (!wasInside || focusedKeyRef.current !== target.dataset.rowKey)) {
          focusedKeyRef.current = target.dataset.rowKey;
          bindRef.current();
        }
      }}
      onBlur={event => {
        if (restoringFocusRef.current) return;
        if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
        focusInsideRef.current = false;
        if (!inspectedKeyRef.current) tooltips.hideIfUnpinned();
        bindRef.current();
      }}
      onKeyDown={handleKeyDown}
    >
      <thead>
        <tr>
          {meta.rankShown && <th scope="col">Rank</th>}
          {layout.columns.map(col => <th key={col.key} scope="col">{col.label}</th>)}
        </tr>
      </thead>
      <tbody ref={mirrorBodyRef} />
    </table>
    {!hasFrame && <div ref={liveRegionRef} className="tf-live-region" role="status" aria-live="polite" aria-atomic="true" />}
    {contextMenu.menu}
  </div>;
}
