import { useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { useContextMenu, type MenuItem } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { fitTooltip, hideTooltip, showTooltip, tooltipText, type TooltipContent } from '../../charts/tooltip.ts';
import { useViewportHeight } from '../../charts/useChartTokens';
import { formatDateTime } from '../../dashboard/calendar.ts';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { canonical } from '../../dashboard/follow/canonical.ts';
import type { FailureOrigin } from '../../dashboard/follow/page.ts';
import { restartPipeline } from '../../dashboard/follow/pipeline.ts';
import { useElement, useFollowedWidget, usePipeline, useSurfaceLabel } from '../../dashboard/follow/useFollowed.ts';
import { useWidgetKey } from '../../dashboard/follow/useFollow.ts';
import type { WidgetSpec } from '../../dashboard/model';
import { PreparingPage } from '../../dashboard/preparing.ts';
import { asFailure } from '../../dashboard/queryError.ts';
import { useWidgetFrame, useWidgetSkeleton, WidgetCaption } from '../../dashboard/widgetFrame.tsx';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { bitmapSurface, holdElement } from '../../draw/bitmapSurface.ts';
import { ROW_HEIGHT, updateTableSettings, type TableSettings } from './settings.ts';
import { columnWidthVar, DISPLAY_LABELS, FamilyTableHeader, type TableHeaderColumn } from './TableHeader.tsx';
import { perfLog } from '../../shell/perf.ts';
import { AuditSnapshotStore, auditIdOf, auditRowsOf, subscribeFamilyAuditing, useFamilyAuditing } from '../audit.ts';
import type { FamilySources, FamilySourceSubscription, MirrorModel } from '../contract.ts';
import type { CountryLooksMap } from '../looks.ts';
import { readFamilyTheme } from '../theme.ts';
import type { FamilyTableBinding } from './laneView.ts';
import { buildColumnLayouts, layoutFamilyTable } from './layout.ts';
import { deriveTableAudit, resolveTableRows, type CountryColorSource, type TableColumnSpec, type TableProjection } from './model.ts';
import './tableFamily.css';

export type { FamilySourceSubscription };

const TABLE_AUDIT = -1;
const STATIC_HOLD = {};
const OVERSCAN = 4;
const CLICK_CANCEL_PX = 4;
const TYPEAHEAD_RESET_MS = 700;
const HOVER_OPEN_MS = 150;
const HOVER_CLOSE_MS = 100;
const HEADER_FALLBACK_WIDTH = 120;

interface AuditSnapshot<TOptions> {
  readonly declaration: TableProjection<TOptions>;
  readonly options: TOptions;
  readonly settings: TableSettings;
  readonly expanded: Readonly<Record<string, boolean>>;
  readonly exactSources: (hour: number) => Promise<FamilySources | null>;
  readonly width: number;
  readonly window: { readonly first: number; readonly count: number };
  readonly measuredWidths: Readonly<Record<string, number>>;
  readonly countryColors?: CountryColorSource;
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
  readonly staticSources?: { readonly sources: FamilySources; readonly hour: number } | null;
  readonly exactSources?: (hour: number) => Promise<FamilySources | null>;
  readonly offset: number;
  readonly pin: number | null;
  readonly bounds?: readonly [number, number] | null;
  readonly countryColours?: ReadonlyMap<string, string>;
  readonly countryLooks?: CountryLooksMap;
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

type NavEntry = readonly [key: string, name: string, index: number, treeState: number, parent: string, meta?: unknown];

function reducedMotionEnabled(): boolean {
  return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
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
  staticSources = null,
  exactSources,
  offset,
  pin,
  bounds,
  countryColours,
  countryLooks,
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
  const frame = useWidgetFrame();
  useWidgetSkeleton('table');
  const captureActive = useFamilyAuditing();
  const viewportHeight = useViewportHeight();
  const contextMenu = useContextMenu(`${label} actions`);
  const contextMenuRef = useRef(contextMenu);
  contextMenuRef.current = contextMenu;
  const frameRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const accessibleRef = useRef<HTMLTableElement>(null);
  const mirrorBodyRef = useRef<HTMLTableSectionElement>(null);
  const liveRegionRef = useRef<HTMLDivElement>(null);

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
  const pointerDownRef = useRef<{ key: string | null; resolved: boolean; pendingUp: boolean; x: number; y: number; button: number; toggle: boolean; cancelled: boolean } | null>(null);
  const hoverTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const leaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pinnedTipRef = useRef<{ key: string; content: TooltipContent; x: number; y: number } | null>(null);
  const heldRef = useRef(held);
  heldRef.current = held;
  const dimRef = useRef(false);
  const pauseFlushRef = useRef(0);
  const auditStoreRef = useRef<AuditSnapshotStore<AuditSnapshot<TOptions>> | null>(null);
  if (!auditStoreRef.current) auditStoreRef.current = new AuditSnapshotStore<AuditSnapshot<TOptions>>(512);
  const lastAuditRef = useRef<{
    id: number;
    declaration: TableProjection<TOptions>;
    options: TOptions;
    settings: TableSettings;
    expanded: Readonly<Record<string, boolean>>;
    exactSources: (hour: number) => Promise<FamilySources | null>;
    width: number;
    windowFirst: number;
    windowCount: number;
    countryLooks: CountryLooksMap | undefined;
    countryPalette: Readonly<Record<string, string>> | undefined;
  } | null>(null);
  const measuredWidthsRef = useRef<Readonly<Record<string, number>>>({});
  const hoverTipShownRef = useRef(false);
  const hoverFittedTipRef = useRef<{ raw: TooltipContent; subtitle: string; height: number; fitted: TooltipContent } | null>(null);
  const focusFittedTipRef = useRef<{ raw: TooltipContent; subtitle: string; height: number; fitted: TooltipContent } | null>(null);
  const lastMirrorKeyRef = useRef('');
  const lastExtentRef = useRef({ height: -1, width: -1, count: -1 });
  const navRef = useRef<readonly NavEntry[]>([]);
  const lastHourRef = useRef(staticSources?.hour ?? 0);
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

  const theme = useMemo(() => readFamilyTheme(countryColours, countryLooks), [countryColours, countryLooks]);
  const tooltipOwner = `family-table:${widget.id}`;
  const announceSeqRef = useRef(0);
  const lastAnnounceKeyRef = useRef<string | null>(null);
  useEffect(() => () => {
    hideTooltip(tooltipOwner);
    if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current);
    if (leaveTimerRef.current) clearTimeout(leaveTimerRef.current);
    if (frame && lastAnnounceKeyRef.current) frame.clearAnnouncement(lastAnnounceKeyRef.current);
  }, [tooltipOwner, frame]);

  useEffect(() => () => {
    auditStoreRef.current?.clear();
  }, []);

  const announce = useCallback((message: string) => {
    if (frame) {
      const key = `table:${++announceSeqRef.current}`;
      lastAnnounceKeyRef.current = key;
      frame.announce(key, message);
      return;
    }
    if (liveRegionRef.current) liveRegionRef.current.textContent = message;
  }, [frame]);

  const changeSettings = useCallback((update: (next: TableSettings) => void) => {
    if (!onSettings) return;
    const next = updateTableSettings(settings, update);
    if (next.sort.column !== settings.sort.column || next.sort.direction !== settings.sort.direction) {
      const col = meta.columns.find(c => c.key === next.sort.column);
      announce(`Sorted by ${col?.label ?? next.sort.column}, ${next.sort.direction === 'descending' ? 'highest first' : 'lowest first'}.`);
    }
    onSettings(next);
  }, [settings, onSettings, meta.columns, announce]);

  const pipeline = usePipeline(runtime.clock, runtime.client);
  const widgetKey = useWidgetKey();
  const enabled = active && !prepared && !blockedState && (subscriptions.length > 0 || staticSources !== null || allowEmptyFollow);
  const pipeKey = enabled && pipeline ? `${widgetKey}|${offset}|${declaration.kind}` : null;
  const pipe = useFollowedWidget(pipeline, pipeKey, {
    offset,
    pin: pin ?? (subscriptions.length === 0 && staticSources ? staticSources.hour : null),
    bounds,
    widget: widgetKey,
    key: `${widget.kind}|${declaration.kind}`,
  });

  const frameElement = useElement(frameRef);
  useSurfaceLabel(frameElement, enabled && !!exactSources, pipe, 'table');

  const subsKey = useMemo(
    () => JSON.stringify(subscriptions.map(s => [s.name, s.source.id, s.source.revision, s.branch, s.offset, s.channel, s.product ?? '', canonical(s.query)])),
    [subscriptions],
  );
  const subsRef = useRef(subscriptions);
  subsRef.current = subscriptions;
  const sourceIdsRef = useRef<Record<string, number>>({});
  const [subsReady, setSubsReady] = useState(0);

  useLayoutEffect(() => {
    if (!pipeline || pipe === null || !enabled || !subsRef.current.length) {
      sourceIdsRef.current = {};
      return;
    }
    const tokens: object[] = [];
    const ids: Record<string, number> = {};
    for (const sub of subsRef.current) {
      const token = {};
      tokens.push(token);
      const id = pipeline.subscribe(pipe, token, sub.source, sub.branch, sub.offset, sub.query, sub.channel, sub.product);
      if (id !== undefined) ids[sub.name] = id;
    }
    sourceIdsRef.current = ids;
    setSubsReady(n => n + 1);
    return () => {
      sourceIdsRef.current = {};
      for (const token of tokens) pipeline.unsubscribe(pipe, token);
    };
  }, [pipeline, pipe, enabled, subsKey]);

  const reportShown = useShownHour();
  const reportShownRef = useRef(reportShown);
  reportShownRef.current = reportShown;

  const surfaceRef = useRef<{ canvas: HTMLCanvasElement; extent: HTMLDivElement; surface: ReturnType<typeof bitmapSurface> } | null>(null);
  const rowHeight = ROW_HEIGHT[settings.density];

  const latestRef = useRef({
    declaration,
    options,
    settings,
    theme,
    staticSources,
    exactSources,
    selectedKey,
    onActivate,
    contextMenuItems,
    viewportHeight,
    rowHeight,
    offset,
    bounds,
    allowEmptyFollow,
    captureActive,
    columns: initialColumns,
  });
  latestRef.current = {
    declaration,
    options,
    settings,
    theme,
    staticSources,
    exactSources,
    selectedKey,
    onActivate,
    contextMenuItems,
    viewportHeight,
    rowHeight,
    offset,
    bounds,
    allowEmptyFollow,
    captureActive,
    columns: meta.columns.length ? meta.columns : initialColumns,
  };

  const bind = useCallback(() => {
    const held = surfaceRef.current;
    if (!held || !pipeline || pipe === null) return;
    const { width, height, scrollLeft: sl, scrollTop: st } = viewportRef.current;
    const w = Math.max(1, width || 320);
    const h = Math.max(1, height || 200);
    const cur = latestRef.current;
    const staticPanels = cur.staticSources ? Object.fromEntries(cur.staticSources.sources.panels) : undefined;
    const staticAnswers = cur.staticSources
      ? Object.fromEntries(
        [...cur.staticSources.sources.answers].filter((entry): entry is [string, NonNullable<(typeof entry)[1]>] => entry[1] !== undefined),
      )
      : undefined;
    let auditId: number | undefined;
    if (perfLog && cur.captureActive && cur.exactSources) {
      const prev = lastAuditRef.current;
      const win = viewWindowRef.current;
      if (
        prev
        && prev.declaration === cur.declaration
        && prev.options === cur.options
        && prev.settings === cur.settings
        && prev.expanded === expandedRef.current
        && prev.exactSources === cur.exactSources
        && prev.width === w
        && prev.windowFirst === win.first
        && prev.windowCount === win.count
        && prev.countryLooks === cur.theme?.countryLooks
        && prev.countryPalette === cur.theme?.country
      ) {
        auditId = prev.id;
      } else {
        auditId = auditStoreRef.current!.create({
          declaration: cur.declaration,
          options: cur.options,
          settings: cur.settings,
          expanded: expandedRef.current,
          exactSources: cur.exactSources,
          width: w,
          window: { first: win.first, count: win.count },
          measuredWidths: measuredWidthsRef.current,
          countryColors: cur.theme ? { countryLooks: cur.theme.countryLooks, country: cur.theme.country } : undefined,
        });
        lastAuditRef.current = {
          id: auditId,
          declaration: cur.declaration,
          options: cur.options,
          settings: cur.settings,
          expanded: expandedRef.current,
          exactSources: cur.exactSources,
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
      selectedKey: cur.selectedKey,
      focusedKey: focusInsideRef.current ? focusedKeyRef.current : null,
      expanded: expandedRef.current,
      reducedMotion: reducedMotionEnabled(),
      window: viewWindowRef.current,
      pauseFlush: pauseFlushRef.current,
      ...(perfLog && cur.captureActive && auditId !== undefined ? { audit: true, auditId } : {}),
    };
    held.canvas.style.width = `${w}px`;
    held.canvas.style.height = `${h}px`;
    pipeline.present(pipe, 'table');
    const dated = Object.keys(sourceIdsRef.current).length === 0
      ? (cur.staticSources
        ? { h: cur.staticSources.hour, c: cur.staticSources.hour }
        : cur.allowEmptyFollow && cur.bounds
          ? { h: cur.bounds[0], c: cur.bounds[1] }
          : undefined)
      : undefined;
    pipeline.bind(
      pipe,
      [held.surface],
      {
        surfaces: [{ width: w, height: h, dpr: typeof devicePixelRatio === 'number' ? devicePixelRatio : 1 }],
        ...(dated ? { dated } : {}),
      },
      {},
      bindingModel,
    );
  }, [pipeline, pipe]);

  const bindRef = useRef(bind);
  bindRef.current = bind;

  const paintMirror = useCallback((mirror: MirrorModel | null) => {
    const tbody = mirrorBodyRef.current;
    if (!tbody) return;
    if (!mirror) {
      if (lastMirrorKeyRef.current !== '') {
        lastMirrorKeyRef.current = '';
        tbody.replaceChildren();
      }
      return;
    }
    const selected = latestRef.current.selectedKey ?? '';
    const hasFocused = focusedKeyRef.current !== null && mirror.rows.some(r => !r.groupHeader && r.key === focusedKeyRef.current);
    const activeFocus = hasFocused ? focusedKeyRef.current : (mirror.rows.find(r => !r.groupHeader)?.key ?? null);
    const key = JSON.stringify([
      mirror.first,
      mirror.total,
      activeFocus,
      selected,
      mirror.rows.map(r => [
        r.rowIndex,
        r.key,
        r.groupHeader ? 1 : 0,
        r.rank ?? null,
        r.level ?? null,
        r.group ? (r.open ? 1 : 0) : -1,
        r.cells.map(c => c.spoken),
      ]),
    ]);
    if (key === lastMirrorKeyRef.current) return;
    lastMirrorKeyRef.current = key;
    const wasFocused = focusInsideRef.current && document.activeElement instanceof HTMLElement && tbody.contains(document.activeElement);
    const fragment = document.createDocumentFragment();
    let restore: HTMLTableRowElement | null = null;
    mirror.rows.forEach(row => {
      const tr = document.createElement('tr');
      tr.dataset.rowKey = row.key;
      tr.setAttribute('aria-rowindex', String(row.rowIndex + 2));
      if (row.groupHeader) {
        tr.setAttribute('role', 'row');
        const td = document.createElement('td');
        td.textContent = row.name;
        tr.append(td);
        fragment.append(tr);
        return;
      }
      tr.tabIndex = row.key === activeFocus ? 0 : -1;
      if (row.level !== undefined) tr.setAttribute('aria-level', String(row.level));
      if (row.group) tr.setAttribute('aria-expanded', String(row.open === true));
      if (selected && selected === row.key) tr.setAttribute('aria-selected', 'true');
      if (row.rank !== undefined) {
        const rankTd = document.createElement('td');
        rankTd.textContent = String(row.rank);
        tr.append(rankTd);
      }
      for (const cell of row.cells) {
        const td = document.createElement('td');
        td.textContent = cell.spoken;
        tr.append(td);
      }
      if (wasFocused && row.key === activeFocus) restore = tr;
      fragment.append(tr);
    });
    restoringFocusRef.current = true;
    try {
      tbody.replaceChildren(fragment);
      if (restore) (restore as HTMLTableRowElement).focus({ preventScroll: true });
    } finally {
      restoringFocusRef.current = false;
    }
  }, []);

  const toggleTreeKey = useCallback((key: string) => {
    const entry = navRef.current.find(item => item[0] === key);
    if (!entry || entry[3] < 0) return;
    const nextOpen = entry[3] === 0;
    setExpanded(prev => ({ ...prev, [key]: nextOpen }));
    announce(`${entry[1]} ${nextOpen ? 'expanded' : 'collapsed'}.`);
  }, [announce]);

  const scrollRowIntoView = useCallback((index: number) => {
    const node = bodyRef.current;
    if (!node || index < 0) return;
    const rh = latestRef.current.rowHeight;
    const top = index * rh;
    const bottom = top + rh;
    const st = viewportRef.current.scrollTop;
    const vh = viewportRef.current.height;
    if (vh <= 0) return;
    let nextTop = st;
    if (top < st) nextTop = top;
    else if (bottom > st + vh) nextTop = bottom - vh;
    if (nextTop !== st) {
      node.scrollTop = nextTop;
      viewportRef.current.scrollTop = nextTop;
      bindRef.current();
    }
  }, []);

  const refreshFocusAnchor = useCallback(() => {
    const box = frameRef.current?.getBoundingClientRect();
    if (box) focusAnchorRef.current = { x: box.left + Math.min(200, box.width / 2), y: box.top + 48 };
  }, []);

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
    holdElement(frameRef.current, held, STATIC_HOLD);
    if (held && !pinnedTipRef.current) hideTooltip(tooltipOwner);
    return () => holdElement(frameRef.current, false, STATIC_HOLD);
  }, [held, tooltipOwner]);

  useLayoutEffect(() => {
    const node = bodyRef.current;
    if (!node || !pipeline || pipe === null) return;
    lastExtentRef.current = { height: -1, width: -1, count: -1 };
    lastMetaKeyRef.current = null;
    lastWidthsRef.current = null;
    lastMirrorObjRef.current = null;
    lastPhasesRef.current = null;
    lastAuditUpdatedRef.current = null;
    let pendingAuditAck: { targetBinding: number; resolve: () => void } | null = null;
    const settleAuditAck = () => {
      if (!pendingAuditAck) return;
      const done = pendingAuditAck;
      pendingAuditAck = null;
      done.resolve();
    };
    const extent = document.createElement('div');
    const drawn = pipeline.held(pipe, 'table', () => {
      const canvas = document.createElement('canvas');
      canvas.className = 'tf-canvas';
      canvas.setAttribute('aria-hidden', 'true');
      canvas.dataset.surface = 'table';
      canvas.dataset.placement = String(pipe);
      return { canvas, surface: bitmapSurface(canvas) };
    })!;
    extent.className = 'tf-extent';
    extent.append(drawn.canvas);
    node.append(extent);
    surfaceRef.current = { canvas: drawn.canvas, extent, surface: drawn.surface };

    const fitTooltipCached = (
      cacheRef: { current: { raw: TooltipContent; subtitle: string; height: number; fitted: TooltipContent } | null },
      raw: TooltipContent,
      hour: number,
      height: number,
    ): TooltipContent => {
      const subtitle = formatDateTime(Math.floor(hour));
      const cached = cacheRef.current;
      if (cached && cached.raw === raw && cached.subtitle === subtitle && cached.height === height) return cached.fitted;
      const dated = raw.subtitle ? raw : { ...raw, subtitle };
      const fitted = fitTooltip([dated], height);
      cacheRef.current = { raw, subtitle, height, fitted };
      return fitted;
    };

    const updateTooltip = (
      tip: { hour: number; rowKey: string; content: TooltipContent } | null,
      pos: { x: number; y: number } | null,
      cacheRef: { current: { raw: TooltipContent; subtitle: string; height: number; fitted: TooltipContent } | null },
    ) => {
      if (pinnedTipRef.current) return;
      if (!tip || !pos) {
        hoverTipShownRef.current = false;
        hideTooltip(tooltipOwner);
        return;
      }
      hoverTipShownRef.current = true;
      const fitted = fitTooltipCached(cacheRef, tip.content, tip.hour, latestRef.current.viewportHeight);
      showTooltip(tooltipOwner, fitted, pos.x, pos.y, tip.rowKey);
    };

    const activateHit = (rowKey: string, isToggle: boolean) => {
      const entry = navRef.current.find(item => item[0] === rowKey);
      if (isToggle || (entry && entry[3] >= 0)) {
        toggleTreeKey(rowKey);
        return;
      }
      latestRef.current.onActivate?.(rowKey, entry?.[5], lastHourRef.current);
    };

    const install = () => {
      const shown = pipeline.view(pipe);
      const frameNode = frameRef.current;
      dimRef.current = !!shown?.dim;
      holdElement(frameNode, !!shown?.dim);
      const visible = shown?.mode === 'picture';
      node.toggleAttribute('data-table-visible', visible);
      node.toggleAttribute('data-table-pending', !visible);

      const mode = shown?.mode ?? 'reading';
      const reason = shown?.reason ?? null;
      const failureOrigin: FailureOrigin = shown?.failureOrigin ?? 'local';
      if (visible && shown?.through !== undefined) {
        lastHourRef.current = shown.painted ?? shown.through;
        reportShownRef.current(shown.through);
      } else if (!visible) {
        reportShownRef.current(null);
      }

      if (pendingAuditAck) {
        const installedAudit = visible && shown?.fields ? auditRowsOf(shown.fields.audit) : undefined;
        if (!visible || mode !== 'picture' || ((shown?.installedBinding ?? 0) >= pendingAuditAck.targetBinding && installedAudit !== undefined)) {
          settleAuditAck();
        }
      }

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

      if (!visible || !shown?.fields) {
        lastMirrorObjRef.current = null;
        paintMirror(null);
        if (!pinnedTipRef.current) {
          hoverTipShownRef.current = false;
          hideTooltip(tooltipOwner);
        }
        return;
      }

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
          extent.style.width = `${extWidth}px`;
        }
        const sig = JSON.stringify([widths.visible, widths.hidden, widths.scrollable, widths.moreWidth, widths.widths]);
        setWidthsState(prev => prev.signature === sig ? prev : { ...widths, signature: sig });
      }

      const installedAuditId = auditIdOf(shown.fields.audit);
      if (perfLog && installedAuditId !== undefined && widths) {
        const prevUpdated = lastAuditUpdatedRef.current;
        if (!prevUpdated || prevUpdated.id !== installedAuditId || prevUpdated.widths !== widths.widths) {
          lastAuditUpdatedRef.current = { id: installedAuditId, widths: widths.widths };
          auditStoreRef.current!.update(installedAuditId, snap => ({ ...snap, measuredWidths: widths.widths }));
        }
      }

      const navField = shown.fields.nav as readonly NavEntry[] | undefined;
      if (navField) navRef.current = navField;
      const tableInfo = shown.fields.table as { count: number; focusedIndex: number; inspectedPresent: boolean } | undefined;
      if (tableInfo) {
        if (lastExtentRef.current.count !== tableInfo.count) {
          lastExtentRef.current.count = tableInfo.count;
          accessibleRef.current?.setAttribute('aria-rowcount', String(tableInfo.count + 1));
        }
        const extHeight = Math.max(viewportRef.current.height, latestRef.current.rowHeight * tableInfo.count);
        if (lastExtentRef.current.height !== extHeight) {
          lastExtentRef.current.height = extHeight;
          extent.style.height = `${extHeight}px`;
        }
        if (focusInsideRef.current && focusedKeyRef.current) {
          const entry = navRef.current.find(item => item[0] === focusedKeyRef.current);
          if (entry) {
            scrollRowIntoView(entry[2]);
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
          if (!pinnedTipRef.current && !focusInsideRef.current) {
            hoverTipShownRef.current = false;
            hideTooltip(tooltipOwner);
          }
        }
      }

      const mirror = shown.fields.mirror as MirrorModel | undefined;
      if (mirror && mirror !== lastMirrorObjRef.current) {
        lastMirrorObjRef.current = mirror;
        paintMirror(mirror);
      }

      const phases = shown.fields.phases as Record<string, unknown> | undefined;
      if (phases && phases !== lastPhasesRef.current && frameNode && perfLog) {
        lastPhasesRef.current = phases;
        frameNode.dataset.familyPhases = JSON.stringify(phases);
      }

      const hoverTipField = shown.fields.tooltip as { key: string; text: { hour: number; rowKey: string; content: TooltipContent } | null } | undefined;
      const focusTipField = shown.fields.focusTooltip as { key: string; text: { hour: number; rowKey: string; content: TooltipContent } | null } | undefined;
      if (focusInsideRef.current && focusedKeyRef.current && focusTipField?.text && focusTipField.text.rowKey === focusedKeyRef.current) {
        const tr = mirrorBodyRef.current?.querySelector<HTMLTableRowElement>(`tr[data-row-key="${CSS.escape(focusedKeyRef.current)}"]`);
        if (tr) tr.setAttribute('aria-description', tooltipText(fitTooltipCached(focusFittedTipRef, focusTipField.text.content, lastHourRef.current, latestRef.current.viewportHeight)));
      }
      if (!shown.dim && !heldRef.current) {
        if (inspectedKeyRef.current && lastPointerRef.current && hoverTipField?.text && hoverTipField.text.rowKey === inspectedKeyRef.current) {
          updateTooltip(hoverTipField.text, lastPointerRef.current, hoverFittedTipRef);
        } else if (focusInsideRef.current && focusedKeyRef.current && focusTipField?.text && focusTipField.text.rowKey === focusedKeyRef.current) {
          updateTooltip(focusTipField.text, focusAnchorRef.current, focusFittedTipRef);
        } else if (!pinnedTipRef.current) {
          hoverTipShownRef.current = false;
          hideTooltip(tooltipOwner);
        }
      } else if (!pinnedTipRef.current) {
        hoverTipShownRef.current = false;
        hideTooltip(tooltipOwner);
      }
    };

    install();
    const stopInstall = pipeline.onInstall(pipe, install);

    const onPointerMove = (event: PointerEvent) => {
      if (!event.isTrusted) return;
      const down = pointerDownRef.current;
      if (down && !down.cancelled && Math.hypot(event.clientX - down.x, event.clientY - down.y) > CLICK_CANCEL_PX) {
        down.cancelled = true;
      }
      if (heldRef.current || dimRef.current) return;
      const last = lastPointerRef.current;
      if (last && last.x === event.clientX && last.y === event.clientY) return;
      lastPointerRef.current = { x: event.clientX, y: event.clientY };
      if (leaveTimerRef.current) { clearTimeout(leaveTimerRef.current); leaveTimerRef.current = null; }
      const box = node.getBoundingClientRect();
      pipeline.pointer(pipe, 0, [event.clientX - box.left, event.clientY - box.top], hitKey => {
        if (heldRef.current || dimRef.current) return;
        const nextKey = hitKey && (hitKey.startsWith('row:') || hitKey.startsWith('toggle:')) ? hitKey.slice(hitKey.indexOf(':') + 1) : null;
        const changed = nextKey !== inspectedKeyRef.current;
        inspectedKeyRef.current = nextKey;
        pipeline.inspect(pipe, nextKey ? `row:${nextKey}` : null);
        if (!nextKey) {
          if (hoverTimerRef.current) { clearTimeout(hoverTimerRef.current); hoverTimerRef.current = null; }
          if (!pinnedTipRef.current && !focusInsideRef.current) hideTooltip(tooltipOwner);
          return;
        }
        if (changed) {
          if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current);
          const delay = hoverTipShownRef.current ? 0 : HOVER_OPEN_MS;
          hoverTimerRef.current = setTimeout(() => {
            hoverTimerRef.current = null;
            if (heldRef.current || dimRef.current) return;
            const currentView = pipeline.view(pipe);
            const tipField = currentView?.fields?.tooltip as { key: string; text: { hour: number; rowKey: string; content: TooltipContent } | null } | undefined;
            if (tipField?.text && tipField.text.rowKey === inspectedKeyRef.current) updateTooltip(tipField.text, lastPointerRef.current, hoverFittedTipRef);
          }, delay);
        }
      });
    };

    const onPointerLeave = () => {
      lastPointerRef.current = null;
      if (hoverTimerRef.current) { clearTimeout(hoverTimerRef.current); hoverTimerRef.current = null; }
      leaveTimerRef.current = setTimeout(() => {
        leaveTimerRef.current = null;
        inspectedKeyRef.current = null;
        pipeline.pointer(pipe, 0, null);
        pipeline.inspect(pipe, null);
        if (!pinnedTipRef.current) {
          if (focusInsideRef.current && focusedKeyRef.current && !heldRef.current && !dimRef.current) {
            const focusTipField = pipeline.view(pipe)?.fields?.focusTooltip as { key: string; text: { hour: number; rowKey: string; content: TooltipContent } | null } | undefined;
            if (focusTipField?.text && focusTipField.text.rowKey === focusedKeyRef.current) {
              updateTooltip(focusTipField.text, focusAnchorRef.current, focusFittedTipRef);
              return;
            }
          }
          hoverTipShownRef.current = false;
          hideTooltip(tooltipOwner);
        }
      }, HOVER_CLOSE_MS);
    };

    const onPointerDown = (event: PointerEvent) => {
      if (event.button !== 0 && event.button !== 2) return;
      if (heldRef.current || dimRef.current) return;
      const box = node.getBoundingClientRect();
      const x = event.clientX, y = event.clientY;
      if (inspectedKeyRef.current) {
        const down = { key: inspectedKeyRef.current, resolved: true, pendingUp: false, x, y, button: event.button, toggle: false, cancelled: false };
        pointerDownRef.current = down;
        pipeline.pointer(pipe, 0, [x - box.left, y - box.top], hitKey => {
          if (pointerDownRef.current === down && hitKey?.startsWith('toggle:')) down.toggle = true;
        });
      } else {
        const down = { key: null as string | null, resolved: false, pendingUp: false, x, y, button: event.button, toggle: false, cancelled: false };
        pointerDownRef.current = down;
        pipeline.pointer(pipe, 0, [x - box.left, y - box.top], hitKey => {
          if (down.cancelled) return;
          const isToggle = !!hitKey?.startsWith('toggle:');
          const rowKey = hitKey && (hitKey.startsWith('row:') || isToggle) ? hitKey.slice(hitKey.indexOf(':') + 1) : null;
          down.key = rowKey;
          down.toggle = isToggle;
          down.resolved = true;
          if (rowKey) {
            inspectedKeyRef.current = rowKey;
            pipeline.inspect(pipe, `row:${rowKey}`);
          }
          if (down.pendingUp) {
            if (pointerDownRef.current === down) pointerDownRef.current = null;
            if (!heldRef.current && !dimRef.current && down.button === 0 && rowKey) {
              activateHit(rowKey, isToggle);
            }
          }
        });
      }
    };

    const onPointerUp = (event: PointerEvent) => {
      const down = pointerDownRef.current;
      if (!down || down.cancelled || down.button !== 0 || heldRef.current || dimRef.current) {
        pointerDownRef.current = null;
        return;
      }
      if (Math.hypot(event.clientX - down.x, event.clientY - down.y) > CLICK_CANCEL_PX) {
        down.cancelled = true;
        pointerDownRef.current = null;
        return;
      }
      if (!down.resolved) {
        down.pendingUp = true;
        return;
      }
      pointerDownRef.current = null;
      if (!down.key) return;
      activateHit(down.key, down.toggle);
    };

    const onPointerCancel = () => {
      if (pointerDownRef.current) pointerDownRef.current.cancelled = true;
      pointerDownRef.current = null;
    };

    const onContextMenu = (event: MouseEvent) => {
      if (heldRef.current || dimRef.current) return;
      const builder = latestRef.current.contextMenuItems;
      if (!builder) return;
      const key = inspectedKeyRef.current ?? (focusInsideRef.current && accessibleRef.current?.contains(event.target as Node) ? focusedKeyRef.current : null);
      const entry = key ? navRef.current.find(item => item[0] === key) : undefined;
      const items = builder(key, key ? (entry?.[1] ?? key) : null, lastHourRef.current);
      if (!items.length) return;
      event.preventDefault();
      contextMenuRef.current.open(event.clientX, event.clientY, items);
    };

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
      refreshFocusAnchor();
      if (w === viewportRef.current.width && h === viewportRef.current.height) return;
      viewportRef.current.width = w;
      viewportRef.current.height = h;
      syncViewWindow();
      setViewportWidth(w);
      bindRef.current();
    });
    resizeObserver.observe(node);
    viewportRef.current.width = node.clientWidth;
    viewportRef.current.height = node.clientHeight;
    syncViewWindow();
    refreshFocusAnchor();
    setViewportWidth(node.clientWidth);

    const menuHost = frameRef.current ?? node;
    node.addEventListener('pointermove', onPointerMove);
    node.addEventListener('pointerleave', onPointerLeave);
    node.addEventListener('pointerdown', onPointerDown);
    node.addEventListener('pointerup', onPointerUp);
    node.addEventListener('pointercancel', onPointerCancel);
    menuHost.addEventListener('contextmenu', onContextMenu);
    node.addEventListener('scroll', onScroll, { passive: true });
    const stopAuditWatch = subscribeFamilyAuditing(activeAudit => {
      latestRef.current.captureActive = activeAudit;
      if (!activeAudit) {
        settleAuditAck();
        bindRef.current();
        return;
      }
      if (!perfLog || !latestRef.current.exactSources) return;
      bindRef.current();
      const targetBinding = pipeline.binding(pipe);
      const currentView = pipeline.view(pipe);
      if (currentView?.mode !== 'picture') return;
      const installedAudit = currentView.fields ? auditRowsOf(currentView.fields.audit) : undefined;
      if ((currentView.installedBinding ?? 0) >= targetBinding && installedAudit !== undefined) return;
      return new Promise<void>(resolve => {
        settleAuditAck();
        pendingAuditAck = { targetBinding, resolve };
      });
    });
    bindRef.current();

    return () => {
      settleAuditAck();
      stopAuditWatch();
      stopInstall();
      resizeObserver.disconnect();
      node.removeEventListener('pointermove', onPointerMove);
      node.removeEventListener('pointerleave', onPointerLeave);
      node.removeEventListener('pointerdown', onPointerDown);
      node.removeEventListener('pointerup', onPointerUp);
      node.removeEventListener('pointercancel', onPointerCancel);
      menuHost.removeEventListener('contextmenu', onContextMenu);
      node.removeEventListener('scroll', onScroll);
      extent.remove();
      surfaceRef.current = null;
      lastExtentRef.current = { height: -1, width: -1, count: -1 };
      holdElement(frameRef.current, false);
    };
  }, [pipeline, pipe, paintMirror, refreshFocusAnchor, runtime.clock, scrollRowIntoView, toggleTreeKey, tooltipOwner]);

  const auditable = !!exactSources;
  useLayoutEffect(() => {
    if (!perfLog || !pipeline || pipe === null || !auditable) return;
    pipeline.audit(pipe, TABLE_AUDIT, fields => auditRowsOf(fields.audit));
    const stopReader = perfLog.reader(pipe, TABLE_AUDIT, async (hour, auditId) => {
      const snap = auditStoreRef.current!.require(auditId);
      const exact = await snap.exactSources(hour);
      if (!exact) return null;
      const model = snap.declaration.project(exact, hour, snap.options);
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
    }, pipeline.generation, auditId => auditStoreRef.current?.pin(auditId));
    return () => {
      stopReader();
      pipeline.audit(pipe, TABLE_AUDIT, null);
    };
  }, [pipeline, pipe, auditable]);

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
  }, [pipe, enabled, bind, options, settings, theme, staticSources, selectedKey, expanded, subsReady, auditable, exactSources, captureActive]);

  useEffect(() => {
    let wasPlaying = runtime.clock.getSnapshot().playing;
    return runtime.clock.subscribe(() => {
      const nowPlaying = runtime.clock.getSnapshot().playing;
      if (wasPlaying && !nowPlaying && pipeline && pipe !== null) {
        pauseFlushRef.current++;
        bindRef.current();
        const view = pipeline.view(pipe);
        const mirror = view?.fields?.mirror as MirrorModel | undefined;
        if (mirror) paintMirror(mirror);
      }
      wasPlaying = nowPlaying;
    });
  }, [runtime.clock, pipeline, pipe, paintMirror]);

  useEffect(() => {
    if (!onDiagnostics) return;
    const all = [...(extraWarnings ?? []), ...meta.warnings];
    onDiagnostics(all);
    return () => onDiagnostics([]);
  }, [onDiagnostics, extraWarnings, meta.warnings]);

  const handleKeyDown = useCallback((event: React.KeyboardEvent) => {
    const nav = navRef.current;
    if ((event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) && !nav.length) {
      if (heldRef.current || dimRef.current) return;
      const builder = latestRef.current.contextMenuItems;
      if (builder) {
        const items = builder(null, null, lastHourRef.current);
        if (items.length) {
          event.preventDefault();
          refreshFocusAnchor();
          contextMenuRef.current.open(focusAnchorRef.current.x, focusAnchorRef.current.y, items);
        }
      }
      return;
    }
    if (!nav.length) return;
    const currentKey = focusedKeyRef.current ?? nav[0]![0];
    const currentIdx = nav.findIndex(item => item[0] === currentKey);
    const idx = currentIdx >= 0 ? currentIdx : 0;
    const entry = nav[idx]!;
    const pageStep = Math.max(1, Math.floor((viewportRef.current.height || 200) / latestRef.current.rowHeight) - 1);

    if (event.key === 'ArrowDown') {
      event.preventDefault();
      focusRowByKey(nav[Math.min(nav.length - 1, idx + 1)]![0]);
      return;
    }
    if (event.key === 'ArrowUp') {
      event.preventDefault();
      focusRowByKey(nav[Math.max(0, idx - 1)]![0]);
      return;
    }
    if (event.key === 'Home') {
      event.preventDefault();
      focusRowByKey(nav[0]![0]);
      return;
    }
    if (event.key === 'End') {
      event.preventDefault();
      focusRowByKey(nav[nav.length - 1]![0]);
      return;
    }
    if (event.key === 'PageDown') {
      event.preventDefault();
      focusRowByKey(nav[Math.min(nav.length - 1, idx + pageStep)]![0]);
      return;
    }
    if (event.key === 'PageUp') {
      event.preventDefault();
      focusRowByKey(nav[Math.max(0, idx - pageStep)]![0]);
      return;
    }
    if (event.key === 'Enter') {
      event.preventDefault();
      if (entry[3] >= 0) toggleTreeKey(entry[0]);
      else latestRef.current.onActivate?.(entry[0], entry[5], lastHourRef.current);
      return;
    }
    if (event.key === 'ArrowRight' && entry[3] === 0) {
      event.preventDefault();
      toggleTreeKey(entry[0]);
      return;
    }
    if (event.key === 'ArrowLeft') {
      if (entry[3] === 1) {
        event.preventDefault();
        toggleTreeKey(entry[0]);
        return;
      }
      if (entry[4]) {
        event.preventDefault();
        focusRowByKey(entry[4]);
        return;
      }
    }
    if (event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) {
      if (heldRef.current || dimRef.current) return;
      const builder = latestRef.current.contextMenuItems;
      if (builder) {
        const items = builder(entry[0], entry[1], lastHourRef.current);
        if (items.length) {
          event.preventDefault();
          refreshFocusAnchor();
          contextMenuRef.current.open(focusAnchorRef.current.x, focusAnchorRef.current.y, items);
        }
      }
      return;
    }
    if ((event.key === 'p' || event.key === 'P') && !event.ctrlKey && !event.metaKey && !event.altKey) {
      event.preventDefault();
      if (pinnedTipRef.current) {
        pinnedTipRef.current = null;
        hideTooltip(tooltipOwner);
        announce('Tooltip unpinned.');
        return;
      }
      if (pipeline && pipe !== null && !heldRef.current && !dimRef.current) {
        const focusTipField = pipeline.view(pipe)?.fields?.focusTooltip as { text: { hour: number; rowKey: string; content: TooltipContent } | null } | undefined;
        if (focusTipField?.text && focusTipField.text.rowKey === currentKey) {
          const dateText = formatDateTime(Math.floor(lastHourRef.current));
          const pinnedContent: TooltipContent = { ...focusTipField.text.content, subtitle: `Pinned at ${dateText}` };
          refreshFocusAnchor();
          const pos = focusAnchorRef.current;
          pinnedTipRef.current = { key: focusTipField.text.rowKey, content: pinnedContent, x: pos.x, y: pos.y };
          showTooltip(tooltipOwner, fitTooltip([pinnedContent], latestRef.current.viewportHeight), pos.x, pos.y, `${focusTipField.text.rowKey}:pinned`);
          announce('Tooltip pinned.');
        }
      }
      return;
    }
    if (event.key === 'Escape') {
      if (pinnedTipRef.current) {
        event.preventDefault();
        pinnedTipRef.current = null;
        hideTooltip(tooltipOwner);
        announce('Tooltip unpinned.');
      }
      return;
    }
    if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) {
      const now = performance.now();
      const query = now <= typeaheadRef.current.until ? `${typeaheadRef.current.query}${event.key.toLowerCase()}` : event.key.toLowerCase();
      typeaheadRef.current = { query, until: now + TYPEAHEAD_RESET_MS };
      for (let offsetIdx = 1; offsetIdx <= nav.length; offsetIdx++) {
        const candidate = nav[(idx + offsetIdx) % nav.length]!;
        if (candidate[1].toLowerCase().startsWith(query)) {
          event.preventDefault();
          focusRowByKey(candidate[0]);
          break;
        }
      }
    }
  }, [announce, focusRowByKey, pipe, pipeline, toggleTreeKey, tooltipOwner]);

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

  const retryFollow = useCallback(() => {
    if (pipeline) restartPipeline(pipeline);
  }, [pipeline]);

  const retryLocal = useCallback(() => {
    if (pipeline && pipe !== null) {
      pipeline.retryWidget(pipe);
      bindRef.current();
    }
  }, [pipeline, pipe]);

  if (blockedState) return <>{blockedState}</>;

  const summaryPillNode = headerSlot && meta.mode === 'picture' && !meta.empty && meta.summaryPill
    ? createPortal(<span className="pill">{meta.summaryPill}</span>, headerSlot)
    : null;

  const bodyStateNode = (() => {
    if (meta.mode === 'outside') return <WidgetState kind="outside" />;
    if (meta.mode === 'refused') return <WidgetState kind="unavailable" technical={meta.reason ?? undefined} />;
    if (meta.mode === 'failed') {
      const failure = (pipeline && pipe !== null ? pipeline.widgetError(pipe) : undefined)
        ?? asFailure(new Error(meta.reason ?? `Couldn’t load ${subject}.`));
      const onRetry = pipeline ? (meta.failureOrigin === 'stream' ? retryFollow : retryLocal) : undefined;
      return <WidgetState kind="error" what={subject} failure={failure} onRetry={onRetry} />;
    }
    if (meta.mode === 'reading') return <WidgetState kind="loading" label={loadingLabel ?? `Loading ${subject}…`} />;
    if (meta.empty) return <WidgetState kind={meta.emptyKind} message={meta.empty} detail={meta.emptyDetail ?? undefined} />;
    return null;
  })();

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
        if (!pinnedTipRef.current && !inspectedKeyRef.current) hideTooltip(tooltipOwner);
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
    {!frame && <div ref={liveRegionRef} className="tf-live-region" role="status" aria-live="polite" aria-atomic="true" />}
    {contextMenu.menu}
  </div>;
}
