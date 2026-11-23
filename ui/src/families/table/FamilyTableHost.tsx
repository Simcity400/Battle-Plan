import { useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { useContextMenu, type MenuItem } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { readTokens } from '../../charts/tokens.ts';
import { fitTooltip, hideTooltip, showTooltip, tooltipText, type TooltipContent } from '../../charts/tooltip.ts';
import { useViewportHeight } from '../../charts/useChartTokens';
import type { DashboardSource } from '../../dashboard/client';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { canonical } from '../../dashboard/follow/canonical.ts';
import type { Channel } from '../../dashboard/follow/facts';
import type { FollowQuery } from '../../dashboard/follow/pipeline.ts';
import { useElement, useFollowedWidget, usePipeline, useSurfaceLabel } from '../../dashboard/follow/useFollowed.ts';
import { useWidgetKey } from '../../dashboard/follow/useFollow.ts';
import type { WidgetSpec } from '../../dashboard/model';
import { PreparingPage } from '../../dashboard/preparing.ts';
import { asFailure } from '../../dashboard/queryError.ts';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { bitmapSurface, holdElement } from '../../draw/bitmapSurface.ts';
import type { AuditRow } from '../../draw/notifications';
import { ROW_HEIGHT, updateTableSettings, type TableSettings, type TableTheme } from './settings.ts';
import { columnWidthVar, DISPLAY_LABELS, FamilyTableHeader, type TableHeaderColumn } from './TableHeader.tsx';
import { perfLog } from '../../shell/perf.ts';
import type { FamilySources, MirrorModel } from '../contract.ts';
import type { FamilyTableBinding } from './laneView.ts';
import { buildColumnLayouts, layoutFamilyTable } from './layout.ts';
import { deriveTableAudit, resolveTableRows, type TableColumnSpec, type TableProjection } from './model.ts';
import './tableFamily.css';

const TABLE_AUDIT = -1;
const OVERSCAN = 4;
const CLICK_CANCEL_PX = 4;
const TOOLTIP_THROTTLE_MS = 250;
const MIRROR_THROTTLE_MS = 1000;
const TYPEAHEAD_RESET_MS = 700;
const HOVER_OPEN_MS = 150;
const HOVER_CLOSE_MS = 100;
const HEADER_FALLBACK_WIDTH = 120;

export interface FamilySourceSubscription {
  readonly name: string;
  readonly source: DashboardSource;
  readonly branch: string;
  readonly offset: number;
  readonly query: FollowQuery;
  readonly channel: Channel;
  readonly product?: string;
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
  readonly subscriptions: readonly FamilySourceSubscription[];
  readonly staticSources?: { readonly sources: FamilySources; readonly hour: number } | null;
  readonly exactSources?: (hour: number) => Promise<FamilySources | null>;
  readonly offset: number;
  readonly pin: number | null;
  readonly bounds?: readonly [number, number] | null;
  readonly countryColours?: ReadonlyMap<string, string>;
  readonly selectedKey?: string | null;
  readonly onActivate?: (key: string, meta?: unknown, hour?: number) => void;
  readonly contextMenuItems?: (key: string, name: string, hour: number) => MenuItem[];
  readonly headerSlot?: HTMLElement | null;
  readonly onDiagnostics?: (warnings: string[]) => void;
  readonly extraWarnings?: readonly string[];
  readonly blockedState?: ReactNode;
  readonly loadingLabel?: string;
  readonly className?: string;
}

interface TableMetaState {
  mode: string;
  reason: string | null;
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

function themeOf(countryColours?: ReadonlyMap<string, string>): TableTheme | undefined {
  if (typeof document === 'undefined') return undefined;
  const tokens = readTokens();
  const rootStyle = getComputedStyle(document.documentElement);
  const hatch = rootStyle.getPropertyValue('--hatch').trim() || tokens.muted;
  const hatchEdge = rootStyle.getPropertyValue('--hatch-edge').trim() || tokens.border;
  return {
    surface: tokens.surface,
    text: tokens.text,
    muted: tokens.muted,
    line: tokens.border,
    lineSoft: tokens.grid,
    accent: tokens.accent,
    blue: tokens.accent,
    red: tokens.error,
    amber: tokens.warning,
    green: tokens.positive,
    grey: tokens.muted,
    hatch,
    hatchEdge,
    font: tokens.font,
    fontSize: { small: tokens.fontSize.small, body: tokens.fontSize.body },
    lineWidth: tokens.lineWidth.grid,
    hatchSpacing: tokens.hatchSpacing,
    country: countryColours ? Object.fromEntries(countryColours) : undefined,
  };
}

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
  subscriptions,
  staticSources = null,
  exactSources,
  offset,
  pin,
  bounds,
  countryColours,
  selectedKey = null,
  onActivate,
  contextMenuItems,
  headerSlot = null,
  onDiagnostics,
  extraWarnings,
  blockedState = null,
  loadingLabel,
  className,
}: FamilyTableHostProps<TOptions>) {
  const prepared = useContext(PreparingPage);
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
  const restoringFocusRef = useRef(false);
  const inspectedKeyRef = useRef<string | null>(null);
  const lastPointerRef = useRef<{ x: number; y: number } | null>(null);
  const pointerDownRef = useRef<{ key: string | null; x: number; y: number; button: number; toggle: boolean } | null>(null);
  const hoverTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const leaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pinnedTipRef = useRef<{ key: string; content: TooltipContent; x: number; y: number } | null>(null);
  const lastTipAtRef = useRef(0);
  const lastMirrorAtRef = useRef(0);
  const lastMirrorKeyRef = useRef('');
  const lastExtentRef = useRef({ height: -1, width: -1, count: -1 });
  const navRef = useRef<readonly NavEntry[]>([]);
  const lastHourRef = useRef(staticSources?.hour ?? 0);
  const typeaheadRef = useRef<{ query: string; until: number }>({ query: '', until: 0 });
  const viewWindowRef = useRef({ first: 0, count: 24 });

  const [meta, setMeta] = useState<TableMetaState>(() => ({
    mode: staticSources ? 'picture' : 'reading',
    reason: null,
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

  const [widthsState, setWidthsState] = useState<WidthsState>({
    widths: {},
    visible: initialColumns.map(c => c.key),
    hidden: [],
    totalWidth: 0,
    scrollable: false,
    moreWidth: 0,
    signature: '',
  });

  const theme = useMemo(() => themeOf(countryColours), [countryColours]);
  const tooltipOwner = `family-table:${widget.id}`;
  useEffect(() => () => {
    hideTooltip(tooltipOwner);
    if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current);
    if (leaveTimerRef.current) clearTimeout(leaveTimerRef.current);
  }, [tooltipOwner]);

  const announce = useCallback((message: string) => {
    if (liveRegionRef.current) liveRegionRef.current.textContent = message;
  }, []);

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
  const enabled = active && !prepared && !blockedState && (subscriptions.length > 0 || staticSources !== null);
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
      focusedKey: focusedKeyRef.current,
      expanded: expandedRef.current,
      reducedMotion: reducedMotionEnabled(),
      window: viewWindowRef.current,
      ...(perfLog ? { audit: true } : {}),
    };
    held.canvas.style.width = `${w}px`;
    held.canvas.style.height = `${h}px`;
    pipeline.present(pipe, 'table');
    pipeline.bind(
      pipe,
      [held.surface],
      {
        surfaces: [{ width: w, height: h, dpr: typeof devicePixelRatio === 'number' ? devicePixelRatio : 1 }],
        ...(cur.staticSources && Object.keys(sourceIdsRef.current).length === 0 ? { dated: { h: cur.staticSources.hour, c: cur.staticSources.hour } } : {}),
      },
      {},
      bindingModel,
    );
  }, [pipeline, pipe]);

  const bindRef = useRef(bind);
  bindRef.current = bind;

  const paintMirror = useCallback((mirror: MirrorModel | null, force: boolean) => {
    const tbody = mirrorBodyRef.current;
    if (!tbody) return;
    const now = performance.now();
    const playing = runtime.clock.getSnapshot().playing;
    if (!force && playing && now - lastMirrorAtRef.current < MIRROR_THROTTLE_MS) return;
    const key = mirror ? `${mirror.first}:${mirror.total}:${focusedKeyRef.current ?? ''}:${mirror.rows.map(r => `${r.key}|${r.cells.map(c => c.spoken).join(',')}`).join(';')}` : '';
    if (!force && key === lastMirrorKeyRef.current) return;
    lastMirrorAtRef.current = now;
    lastMirrorKeyRef.current = key;
    if (!mirror) {
      tbody.replaceChildren();
      return;
    }
    const activeFocus = focusedKeyRef.current ?? mirror.rows.find(r => !r.groupHeader)?.key ?? null;
    const wasFocused = focusInsideRef.current && document.activeElement instanceof HTMLElement && tbody.contains(document.activeElement);
    const fragment = document.createDocumentFragment();
    let restore: HTMLTableRowElement | null = null;
    mirror.rows.forEach((row, idx) => {
      const tr = document.createElement('tr');
      tr.dataset.rowKey = row.key;
      tr.setAttribute('aria-rowindex', String(mirror.first + idx + 2));
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
      if (latestRef.current.selectedKey === row.key) tr.setAttribute('aria-selected', 'true');
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
      if (wasFocused && row.key === focusedKeyRef.current) restore = tr;
      fragment.append(tr);
    });
    tbody.replaceChildren(fragment);
    if (restore) {
      restoringFocusRef.current = true;
      (restore as HTMLTableRowElement).focus({ preventScroll: true });
      restoringFocusRef.current = false;
    }
  }, [runtime.clock]);

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
    if (pipeline && pipe !== null) pipeline.inspect(pipe, `row:${key}`);
    bindRef.current();
  }, [pipeline, pipe, scrollRowIntoView]);

  useLayoutEffect(() => {
    const node = bodyRef.current;
    if (!node || !pipeline || pipe === null) return;
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

    const updateTooltip = (tip: { hour: number; rowKey: string; content: TooltipContent } | null, force: boolean) => {
      if (pinnedTipRef.current) return;
      const point = lastPointerRef.current;
      if (!tip || (!point && !focusInsideRef.current)) {
        hideTooltip(tooltipOwner);
        return;
      }
      const now = performance.now();
      const playing = runtime.clock.getSnapshot().playing;
      if (!force && playing && now - lastTipAtRef.current < TOOLTIP_THROTTLE_MS) return;
      lastTipAtRef.current = now;
      const fitted = fitTooltip([tip.content], latestRef.current.viewportHeight);
      const pos = point ?? (() => {
        const box = frameRef.current?.getBoundingClientRect();
        return box ? { x: box.left + Math.min(200, box.width / 2), y: box.top + 48 } : { x: 0, y: 0 };
      })();
      showTooltip(tooltipOwner, fitted, pos.x, pos.y, tip.rowKey);
      if (focusInsideRef.current && focusedKeyRef.current === tip.rowKey) {
        const tr = mirrorBodyRef.current?.querySelector<HTMLTableRowElement>(`tr[data-row-key="${CSS.escape(tip.rowKey)}"]`);
        if (tr) tr.setAttribute('aria-description', tooltipText(fitted));
      }
    };

    const install = () => {
      const shown = pipeline.view(pipe);
      const frameNode = frameRef.current;
      holdElement(frameNode, !!shown?.dim);
      const visible = shown?.mode === 'picture';
      node.toggleAttribute('data-table-visible', visible);
      node.toggleAttribute('data-table-pending', !visible);

      const mode = shown?.mode ?? 'reading';
      const reason = shown?.reason ?? null;
      if (visible && shown?.through !== undefined) {
        lastHourRef.current = shown.painted ?? shown.through;
        reportShownRef.current(shown.through);
      } else if (!visible) {
        reportShownRef.current(null);
      }

      const rawMeta = shown?.fields?.meta as Omit<TableMetaState, 'mode' | 'reason'> | undefined;
      const nextMetaSig = JSON.stringify([mode, reason, rawMeta?.empty ?? null, rawMeta?.emptyDetail ?? null, rawMeta?.summaryPill ?? null, rawMeta?.notes ?? [], rawMeta?.warnings ?? [], rawMeta?.hasTree ?? false, rawMeta?.unsorted ?? false, rawMeta?.rankShown ?? false, rawMeta?.columns?.map(c => c.key) ?? []]);
      if (nextMetaSig !== metaSigRef.current) {
        metaSigRef.current = nextMetaSig;
        setMeta(prev => ({
          mode,
          reason,
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

      if (!visible || !shown?.fields) {
        paintMirror(null, true);
        if (!pinnedTipRef.current) hideTooltip(tooltipOwner);
        return;
      }

      const widths = shown.fields.widths as Omit<WidthsState, 'signature'> | undefined;
      if (widths) {
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
          if (tableInfo.focusedIndex >= 0) {
            scrollRowIntoView(tableInfo.focusedIndex);
          } else if (navRef.current.length > 0) {
            focusedKeyRef.current = navRef.current[0]![0];
            queueMicrotask(() => bindRef.current());
          }
        }
        if (inspectedKeyRef.current && !tableInfo.inspectedPresent) {
          inspectedKeyRef.current = null;
          pipeline.inspect(pipe, null);
          if (!pinnedTipRef.current) hideTooltip(tooltipOwner);
        }
      }

      const mirror = shown.fields.mirror as MirrorModel | undefined;
      if (mirror) paintMirror(mirror, false);

      const phases = shown.fields.phases as Record<string, unknown> | undefined;
      if (phases && frameNode && perfLog) frameNode.dataset.familyPhases = JSON.stringify(phases);

      const tipField = shown.fields.tooltip as { key: string; text: { hour: number; rowKey: string; content: TooltipContent } | null } | undefined;
      if (tipField?.text && !shown.dim) {
        updateTooltip(tipField.text, false);
      } else if (!pinnedTipRef.current && !inspectedKeyRef.current && !focusInsideRef.current) {
        hideTooltip(tooltipOwner);
      }
    };

    install();
    const stopInstall = pipeline.onInstall(pipe, install);

    const onPointerMove = (event: PointerEvent) => {
      if (!event.isTrusted) return;
      const last = lastPointerRef.current;
      if (last && last.x === event.clientX && last.y === event.clientY) return;
      lastPointerRef.current = { x: event.clientX, y: event.clientY };
      if (leaveTimerRef.current) { clearTimeout(leaveTimerRef.current); leaveTimerRef.current = null; }
      const box = node.getBoundingClientRect();
      pipeline.pointer(pipe, 0, [event.clientX - box.left, event.clientY - box.top], hitKey => {
        const nextKey = hitKey && (hitKey.startsWith('row:') || hitKey.startsWith('toggle:')) ? hitKey.slice(hitKey.indexOf(':') + 1) : null;
        const changed = nextKey !== inspectedKeyRef.current;
        inspectedKeyRef.current = nextKey;
        pipeline.inspect(pipe, nextKey ? `row:${nextKey}` : null);
        if (!nextKey) {
          if (hoverTimerRef.current) { clearTimeout(hoverTimerRef.current); hoverTimerRef.current = null; }
          if (!pinnedTipRef.current) hideTooltip(tooltipOwner);
          return;
        }
        if (changed) {
          if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current);
          const delay = lastTipAtRef.current > 0 ? 0 : HOVER_OPEN_MS;
          hoverTimerRef.current = setTimeout(() => {
            hoverTimerRef.current = null;
            const currentView = pipeline.view(pipe);
            const tipField = currentView?.fields?.tooltip as { key: string; text: { hour: number; rowKey: string; content: TooltipContent } | null } | undefined;
            if (tipField?.text && tipField.text.rowKey === inspectedKeyRef.current) updateTooltip(tipField.text, true);
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
        pipeline.inspect(pipe, focusInsideRef.current && focusedKeyRef.current ? `row:${focusedKeyRef.current}` : null);
        if (!pinnedTipRef.current && !focusInsideRef.current) hideTooltip(tooltipOwner);
      }, HOVER_CLOSE_MS);
    };

    const onPointerDown = (event: PointerEvent) => {
      if (event.button !== 0 && event.button !== 2) return;
      const box = node.getBoundingClientRect();
      const x = event.clientX, y = event.clientY;
      if (inspectedKeyRef.current) {
        pointerDownRef.current = { key: inspectedKeyRef.current, x, y, button: event.button, toggle: false };
        pipeline.pointer(pipe, 0, [x - box.left, y - box.top], hitKey => {
          if (pointerDownRef.current && hitKey?.startsWith('toggle:')) pointerDownRef.current.toggle = true;
        });
      } else {
        const hitIdx = Math.floor((y - box.top + viewportRef.current.scrollTop) / latestRef.current.rowHeight);
        const fallbackKey = navRef.current.find(item => item[2] === hitIdx)?.[0] ?? null;
        pointerDownRef.current = { key: fallbackKey, x, y, button: event.button, toggle: false };
        pipeline.pointer(pipe, 0, [x - box.left, y - box.top], hitKey => {
          if (!pointerDownRef.current || !hitKey) return;
          const isToggle = hitKey.startsWith('toggle:');
          const rowKey = hitKey.startsWith('row:') || isToggle ? hitKey.slice(hitKey.indexOf(':') + 1) : null;
          pointerDownRef.current.key = rowKey;
          pointerDownRef.current.toggle = isToggle;
          if (rowKey) {
            inspectedKeyRef.current = rowKey;
            pipeline.inspect(pipe, `row:${rowKey}`);
          }
        });
      }
    };

    const onPointerUp = (event: PointerEvent) => {
      const down = pointerDownRef.current;
      pointerDownRef.current = null;
      if (!down || down.button !== 0 || !down.key) return;
      if (Math.hypot(event.clientX - down.x, event.clientY - down.y) > CLICK_CANCEL_PX) return;
      const entry = navRef.current.find(item => item[0] === down.key);
      if (down.toggle || (entry && entry[3] >= 0)) {
        toggleTreeKey(down.key);
        return;
      }
      latestRef.current.onActivate?.(down.key, entry?.[5], lastHourRef.current);
    };

    const onContextMenu = (event: MouseEvent) => {
      const builder = latestRef.current.contextMenuItems;
      if (!builder) return;
      const key = inspectedKeyRef.current ?? focusedKeyRef.current;
      if (!key) return;
      const entry = navRef.current.find(item => item[0] === key);
      const items = builder(key, entry?.[1] ?? key, lastHourRef.current);
      if (!items.length) return;
      event.preventDefault();
      contextMenuRef.current.open(event.clientX, event.clientY, items);
    };

    const onScroll = () => {
      const sl = node.scrollLeft, st = node.scrollTop;
      const prevSl = viewportRef.current.scrollLeft;
      viewportRef.current.scrollLeft = sl;
      viewportRef.current.scrollTop = st;
      if (sl !== prevSl) setScrollLeft(sl);
      const rh = latestRef.current.rowHeight;
      const first = Math.max(0, Math.floor(st / rh) - OVERSCAN);
      const count = Math.ceil(Math.max(1, viewportRef.current.height) / rh) + OVERSCAN * 2;
      viewWindowRef.current = { first, count };
      bindRef.current();
    };

    const resizeObserver = new ResizeObserver(entries => {
      const rect = entries[0]?.contentRect;
      const w = Math.round(rect ? rect.width : node.clientWidth);
      const h = Math.round(rect ? rect.height : node.clientHeight);
      if (w === viewportRef.current.width && h === viewportRef.current.height) return;
      viewportRef.current.width = w;
      viewportRef.current.height = h;
      const rh = latestRef.current.rowHeight;
      viewWindowRef.current = { first: Math.max(0, Math.floor(viewportRef.current.scrollTop / rh) - OVERSCAN), count: Math.ceil(Math.max(1, h) / rh) + OVERSCAN * 2 };
      setViewportWidth(w);
      bindRef.current();
    });
    resizeObserver.observe(node);
    viewportRef.current.width = node.clientWidth;
    viewportRef.current.height = node.clientHeight;
    setViewportWidth(node.clientWidth);

    node.addEventListener('pointermove', onPointerMove);
    node.addEventListener('pointerleave', onPointerLeave);
    node.addEventListener('pointerdown', onPointerDown);
    node.addEventListener('pointerup', onPointerUp);
    node.addEventListener('contextmenu', onContextMenu);
    node.addEventListener('scroll', onScroll, { passive: true });
    bindRef.current();

    return () => {
      stopInstall();
      resizeObserver.disconnect();
      node.removeEventListener('pointermove', onPointerMove);
      node.removeEventListener('pointerleave', onPointerLeave);
      node.removeEventListener('pointerdown', onPointerDown);
      node.removeEventListener('pointerup', onPointerUp);
      node.removeEventListener('contextmenu', onContextMenu);
      node.removeEventListener('scroll', onScroll);
      extent.remove();
      surfaceRef.current = null;
      holdElement(frameRef.current, false);
    };
  }, [pipeline, pipe, paintMirror, runtime.clock, scrollRowIntoView, toggleTreeKey, tooltipOwner]);

  useLayoutEffect(() => {
    if (pipe !== null && enabled) bind();
  }, [pipe, enabled, bind, options, settings, theme, staticSources, selectedKey, expanded, subsReady]);

  useEffect(() => {
    let wasPlaying = runtime.clock.getSnapshot().playing;
    return runtime.clock.subscribe(() => {
      const nowPlaying = runtime.clock.getSnapshot().playing;
      if (wasPlaying && !nowPlaying && pipeline && pipe !== null) {
        const view = pipeline.view(pipe);
        const mirror = view?.fields?.mirror as MirrorModel | undefined;
        if (mirror) paintMirror(mirror, true);
      }
      wasPlaying = nowPlaying;
    });
  }, [runtime.clock, pipeline, pipe, paintMirror]);

  const auditable = !!exactSources;
  useLayoutEffect(() => {
    if (!perfLog || !pipeline || pipe === null || !auditable) return;
    pipeline.audit(pipe, TABLE_AUDIT, fields => fields.audit as readonly AuditRow[] | undefined);
    perfLog.reader(pipe, TABLE_AUDIT, async hour => {
      const cur = latestRef.current;
      const exact = await cur.exactSources?.(hour);
      if (!exact) return null;
      const model = cur.declaration.project(exact, hour, cur.options);
      const flatRows = resolveTableRows(model, cur.settings, expandedRef.current);
      const allLayouts = buildColumnLayouts(model.columns, cur.settings);
      const enabledSpecs = allLayouts.filter(col => col.shown).map(col => model.columns.find(spec => spec.key === col.key)!).filter(Boolean);
      const rows = deriveTableAudit(flatRows, enabledSpecs, model.hour, model.empty);
      return { values: new Map(rows.map(row => [row.key, row.value ?? ''])), ordered: true };
    });
    return () => pipeline.audit(pipe, TABLE_AUDIT, null);
  }, [pipeline, pipe, auditable]);

  useEffect(() => {
    if (!onDiagnostics) return;
    const all = [...(extraWarnings ?? []), ...meta.warnings];
    onDiagnostics(all);
    return () => onDiagnostics([]);
  }, [onDiagnostics, extraWarnings, meta.warnings]);

  const handleKeyDown = useCallback((event: React.KeyboardEvent) => {
    const nav = navRef.current;
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
      const builder = latestRef.current.contextMenuItems;
      if (builder) {
        const items = builder(entry[0], entry[1], lastHourRef.current);
        if (items.length) {
          event.preventDefault();
          const box = frameRef.current?.getBoundingClientRect();
          contextMenuRef.current.open(box ? box.left + 60 : 0, box ? box.top + 48 : 0, items);
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
      if (pipeline && pipe !== null) {
        const tipField = pipeline.view(pipe)?.fields?.tooltip as { text: { hour: number; rowKey: string; content: TooltipContent } | null } | undefined;
        if (tipField?.text) {
          const pinnedContent: TooltipContent = { ...tipField.text.content, subtitle: `Pinned at ${tipField.text.content.subtitle ?? ''}` };
          const box = frameRef.current?.getBoundingClientRect();
          const pos = lastPointerRef.current ?? (box ? { x: box.left + Math.min(200, box.width / 2), y: box.top + 48 } : { x: 0, y: 0 });
          pinnedTipRef.current = { key: tipField.text.rowKey, content: pinnedContent, x: pos.x, y: pos.y };
          showTooltip(tooltipOwner, fitTooltip([pinnedContent], latestRef.current.viewportHeight), pos.x, pos.y, `${tipField.text.rowKey}:pinned`);
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
    const protectedKeys = new Set<string>();
    if (!meta.unsorted && settings.sort.column) protectedKeys.add(settings.sort.column);
    return layoutFamilyTable(base, Math.max(1, viewportWidth || 320), settings.overflow, {
      rankShown: meta.rankShown,
      rankWidth: meta.rankWidth || 36,
      protectedKeys,
    });
  }, [activeColumns, settings, widthsState.widths, resizing, meta.unsorted, meta.rankShown, meta.rankWidth, viewportWidth]);

  const headerColumns = useMemo<TableHeaderColumn[]>(() => layout.columns.map(col => {
    const spec = specMap.get(col.key);
    return {
      ...col,
      text: spec?.text,
      hideable: spec?.hideable,
      hint: spec?.hint,
      displays: spec && spec.offered.length > 1 ? spec.offered.map(display => [display, DISPLAY_LABELS[display]] as const) : [],
    };
  }), [layout.columns, specMap]);

  if (blockedState) return <>{blockedState}</>;

  const summaryPillNode = headerSlot && meta.mode === 'picture' && !meta.empty && meta.summaryPill
    ? createPortal(<span className="pill">{meta.summaryPill}</span>, headerSlot)
    : null;

  const bodyStateNode = (() => {
    if (meta.mode === 'outside') return <WidgetState kind="outside" />;
    if (meta.mode === 'refused') return <WidgetState kind="unavailable" message={meta.reason ?? `${subject} isn’t in this recording.`} />;
    if (meta.mode === 'failed') return <WidgetState kind="error" what={subject} failure={asFailure(new Error(meta.reason ?? `Couldn’t load ${subject}.`))} />;
    if (meta.mode === 'reading') return <WidgetState kind="loading" label={loadingLabel ?? `Loading ${subject}…`} />;
    if (meta.empty) return <WidgetState kind={meta.emptyKind} message={meta.empty} detail={meta.emptyDetail ?? undefined} />;
    return null;
  })();

  const hiddenCols = settings.overflow === 'tooltip' ? (widthsState.hidden.length ? widthsState.hidden : layout.hidden) : [];

  return <div ref={frameRef} className={`tf-frame${className ? ` ${className}` : ''}`} data-density={settings.density} data-overflow={settings.overflow}>
    {summaryPillNode}
    {meta.notes.length > 0 && !bodyStateNode && <div className="tf-notes">{meta.notes.map((note, idx) => <span key={idx}>{note}</span>)}</div>}
    <div className="tf-head-clip" style={bodyStateNode ? { display: 'none' } : undefined}>
      <FamilyTableHeader
        columns={headerColumns}
        allColumns={activeColumns}
        hiddenColumns={hiddenCols}
        settings={settings}
        unsorted={meta.unsorted}
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
        const target = (event.target as HTMLElement | null)?.closest<HTMLTableRowElement>('tr[data-row-key]');
        if (target?.dataset.rowKey && (!wasInside || focusedKeyRef.current !== target.dataset.rowKey)) {
          focusedKeyRef.current = target.dataset.rowKey;
          if (pipeline && pipe !== null) pipeline.inspect(pipe, `row:${target.dataset.rowKey}`);
          bindRef.current();
        }
      }}
      onBlur={event => {
        if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
        focusInsideRef.current = false;
        if (!inspectedKeyRef.current && pipeline && pipe !== null) pipeline.inspect(pipe, null);
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
    <div ref={liveRegionRef} className="tf-live-region" role="status" aria-live="polite" aria-atomic="true" />
    {contextMenu.menu}
  </div>;
}
