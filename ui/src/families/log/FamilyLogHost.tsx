import { useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { useContextMenu, type MenuItem } from '../../components/ContextMenu';
import { ChipSet } from '../../components/ChipSet';
import ContentSetPicker from '../../components/ContentSetPicker';
import WidgetState from '../../components/WidgetState';
import { readTokens } from '../../charts/tokens.ts';
import { fitTooltip, hideTooltip, showTooltip, type TooltipContent } from '../../charts/tooltip.ts';
import { useViewportHeight } from '../../charts/useChartTokens';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { canonical } from '../../dashboard/follow/canonical.ts';
import { useElement, useFollowedWidget, usePipeline, useSurfaceLabel } from '../../dashboard/follow/useFollowed.ts';
import { useWidgetKey } from '../../dashboard/follow/useFollow.ts';
import type { WidgetSpec } from '../../dashboard/model';
import { PreparingPage } from '../../dashboard/preparing.ts';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { bitmapSurface, holdElement } from '../../draw/bitmapSurface.ts';
import { perfLog } from '../../shell/perf.ts';
import { AuditSnapshotStore, auditRowsOf } from '../audit.ts';
import type { FamilySources, MirrorModel } from '../contract.ts';
import type { LogControlsSpec, LogExactReader, LogProjection, LogSourceSubscription } from './declaration.ts';
import type { FamilyLogBinding, LogMetaField, LogNavEntry } from './laneView.ts';
import {
  buildLogLayout,
  LOG_MIN_TAIL_LIMIT,
  logicalScrollFromPhysical,
  physicalScrollFromLogical,
  rowIndexAtLogicalY,
  tailLimitForViewport,
  visibleItemRange,
} from './layout.ts';
import { canonicalLogRowAudit, type LogDensity, type LogFacetPill } from './model.ts';
import type { LogTheme } from './painter.ts';
import './logFamily.css';

const LOG_AUDIT = -1;
const STATIC_HOLD = {};
const HOVER_OPEN_MS = 150;
const HOVER_CLOSE_MS = 100;
const CLICK_CANCEL_PX = 4;

interface LogAuditSnapshot<TOptions> {
  readonly projection: LogProjection<TOptions>;
  readonly options: TOptions;
  readonly density: LogDensity;
  readonly months: boolean;
  readonly scrollTop: number;
  readonly height: number;
  readonly limit: number;
  readonly exactSources: LogExactReader;
}

export interface FamilyLogHostProps<TOptions> {
  readonly projection: LogProjection<TOptions>;
  readonly options: TOptions;
  readonly label: string;
  readonly subject: string;
  readonly widget: WidgetSpec;
  readonly runtime: WidgetRuntime;
  readonly active: boolean;
  readonly density: LogDensity;
  readonly months: boolean;
  readonly twoLine?: boolean;
  readonly subscriptions: readonly LogSourceSubscription[];
  readonly staticSources?: { readonly sources: FamilySources; readonly hour: number } | null;
  readonly exactSources?: LogExactReader;
  readonly onLoadScrollPage?: (frozenHour: number, offset: number, pageSize: number, baseOrdinal: number, total: number) => Promise<unknown>;
  readonly onLimitChange?: (limit: number) => void;
  readonly offset: number;
  readonly pin: number | null;
  readonly bounds?: readonly [number, number] | null;
  readonly controls?: LogControlsSpec | null;
  readonly note?: ReactNode;
  readonly selectedKey?: string | null;
  readonly onActivate?: (key: string, meta?: unknown, hour?: number) => void;
  readonly contextMenuItems?: (key: string, hour: number, meta?: unknown) => MenuItem[];
  readonly headerSlot?: HTMLElement | null;
  readonly onDiagnostics?: (warnings: string[]) => void;
  readonly extraWarnings?: readonly string[];
  readonly blockedState?: ReactNode;
  readonly held?: boolean;
  readonly className?: string;
}

function logThemeOf(): LogTheme | undefined {
  if (typeof document === 'undefined') return undefined;
  const tokens = readTokens();
  const rootStyle = getComputedStyle(document.documentElement);
  const hatch = rootStyle.getPropertyValue('--hatch').trim() || tokens.muted;
  const hatchEdge = rootStyle.getPropertyValue('--hatch-edge').trim() || tokens.border;
  const surfaceAlt = rootStyle.getPropertyValue('--surface-raised').trim() || tokens.surface;
  const hover = rootStyle.getPropertyValue('--row-hover').trim() || 'rgba(128,128,128,0.12)';
  const selected = rootStyle.getPropertyValue('--row-selected').trim() || 'rgba(80,140,255,0.18)';
  return {
    surface: tokens.surface,
    surfaceAlt,
    hover,
    selected,
    text: tokens.text,
    muted: tokens.muted,
    line: tokens.border,
    lineSoft: tokens.grid,
    accent: tokens.accent,
    positive: tokens.positive,
    negative: tokens.error,
    warning: tokens.warning,
    hatch,
    hatchEdge,
    font: tokens.font,
    fontSize: { small: tokens.fontSize.small, body: tokens.fontSize.body },
    hatchSpacing: tokens.hatchSpacing,
    categories: tokens.categories,
  };
}

function LogSkeleton() {
  return (
    <div className="family-log-skeleton" data-state="loading" aria-busy="true" aria-label="Loading log rows">
      {Array.from({ length: 8 }, (_, index) => (
        <div key={index} className="family-log-skeleton-row">
          <span className="family-log-skeleton-bar" />
          <span className="family-log-skeleton-bar" />
          <span className="family-log-skeleton-bar" />
          <span className="family-log-skeleton-bar" />
        </div>
      ))}
    </div>
  );
}

export default function FamilyLogHost<TOptions>({
  projection,
  options,
  label,
  subject,
  widget,
  runtime,
  active,
  density,
  months,
  twoLine = false,
  subscriptions,
  staticSources = null,
  exactSources,
  onLoadScrollPage,
  onLimitChange,
  offset,
  pin,
  bounds,
  controls = null,
  note = null,
  selectedKey = null,
  onActivate,
  contextMenuItems,
  headerSlot = null,
  onDiagnostics,
  extraWarnings,
  blockedState = null,
  held = false,
  className,
}: FamilyLogHostProps<TOptions>) {
  const prepared = useContext(PreparingPage);
  const viewportHeight = useViewportHeight();
  const contextMenu = useContextMenu(`${label} actions`);
  const contextMenuRef = useRef(contextMenu);
  contextMenuRef.current = contextMenu;

  const frameRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const mirrorRef = useRef<HTMLDivElement>(null);
  const liveRegionRef = useRef<HTMLDivElement>(null);
  const pillsRef = useRef<HTMLSpanElement>(null);
  const lastPillsSigRef = useRef('');
  const focusAnchorRef = useRef({ x: 0, y: 0 });

  const viewportRef = useRef({ width: 0, height: 0, scrollTop: 0, physicalTop: 0 });
  const limitRef = useRef(LOG_MIN_TAIL_LIMIT);
  const frozenRef = useRef<{ hour: number; baseOrdinal: number; total: number; panels: Record<string, unknown> } | null>(null);
  const [jumpCount, setJumpCount] = useState<number | null>(null);
  const scrollPageFlightRef = useRef<number | null>(null);
  const scrollPageOffsetRef = useRef<number>(0);

  const focusedKeyRef = useRef<string | null>(null);
  const focusInsideRef = useRef(false);
  const inspectedKeyRef = useRef<string | null>(null);
  const lastPointerRef = useRef<{ x: number; y: number } | null>(null);
  const pointerDownRef = useRef<{ key: string | null; x: number; y: number; button: number; cancelled: boolean } | null>(null);
  const hoverTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const leaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pinnedTipRef = useRef<{ key: string; content: TooltipContent; x: number; y: number } | null>(null);
  const hoverTipShownRef = useRef(false);
  const heldRef = useRef(held);
  heldRef.current = held;
  const dimRef = useRef(false);
  const pauseFlushRef = useRef(0);
  const lastMirrorKeyRef = useRef('');
  const lastExtentHeightRef = useRef(-1);
  const navRef = useRef<readonly LogNavEntry[]>([]);
  const activeNavRef = useRef<LogNavEntry | null>(null);
  const lastHourRef = useRef<number | null>(staticSources?.hour ?? null);
  const lastMetaRef = useRef<LogMetaField | null>(null);
  const auditStoreRef = useRef<AuditSnapshotStore<LogAuditSnapshot<TOptions>> | null>(null);
  if (!auditStoreRef.current) auditStoreRef.current = new AuditSnapshotStore<LogAuditSnapshot<TOptions>>(512);
  const lastAuditRef = useRef<{ id: number; key: string; options: TOptions } | null>(null);
  const hasPaintedRef = useRef(false);

  const [meta, setMeta] = useState<{
    mode: LogMetaField['mode'];
    reason: string | null;
    empty: LogMetaField['empty'];
    warnings: readonly string[];
  }>(() => ({
    mode: staticSources ? 'picture' : 'reading',
    reason: null,
    empty: null,
    warnings: [],
  }));
  const metaSigRef = useRef('');

  const paintPills = useCallback((pills: readonly LogFacetPill[]) => {
    if (!pills.length && lastPillsSigRef.current === '') return;
    const host = pillsRef.current;
    if (!host) return;
    let sig = '';
    for (let i = 0; i < pills.length; i++) {
      const p = pills[i]!;
      const c = p.count;
      const valPart = c.kind === 'known' || c.kind === 'zero' || c.kind === 'partial' ? String(c.value) : '';
      const reasonPart = c.kind === 'notRecorded' || c.kind === 'notApplicable' || c.kind === 'partial' ? c.reason : '';
      sig += `${p.key}:${p.label}:${p.tone ?? ''}:${c.kind}:${valPart}:${reasonPart}|`;
    }
    if (sig === lastPillsSigRef.current) return;
    lastPillsSigRef.current = sig;
    if (!pills.length) {
      host.replaceChildren();
      return;
    }
    const fragment = document.createDocumentFragment();
    for (const pill of pills) {
      const span = document.createElement('span');
      const toneClass = pill.tone && pill.tone !== 'default' && pill.tone !== 'muted' ? ` ${pill.tone}` : '';
      const stateClass = pill.count.kind === 'partial' ? ' is-partial' : pill.count.kind === 'notRecorded' ? ' is-missing' : '';
      span.className = `pill${toneClass}${stateClass}`;
      if (pill.count.kind === 'known' || pill.count.kind === 'zero') {
        span.textContent = `${pill.label} ${Number(pill.count.value).toLocaleString()}`;
      } else if (pill.count.kind === 'partial') {
        const prefix = pill.count.atLeast ? '≥ ' : '';
        span.textContent = `${pill.label} ${prefix}${Number(pill.count.value).toLocaleString()} (partial)`;
        span.title = pill.count.reason;
      } else if (pill.count.kind === 'notRecorded') {
        span.textContent = `${pill.label} Not recorded`;
        span.title = pill.count.reason;
      } else {
        span.textContent = `${pill.label} —`;
        span.title = pill.count.reason;
      }
      fragment.append(span);
    }
    host.replaceChildren(fragment);
  }, []);

  const theme = useMemo(() => logThemeOf(), []);
  const tooltipOwner = `family-log:${widget.id}`;
  useEffect(() => () => {
    hideTooltip(tooltipOwner);
    if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current);
    if (leaveTimerRef.current) clearTimeout(leaveTimerRef.current);
  }, [tooltipOwner]);

  const pipeline = usePipeline(runtime.clock, runtime.client);
  const widgetKey = useWidgetKey();
  const enabled = active && !prepared && !blockedState && (subscriptions.length > 0 || staticSources !== null);
  const pipeKey = enabled && pipeline ? `${widgetKey}|${offset}|${projection.kind}` : null;
  const pipe = useFollowedWidget(pipeline, pipeKey, {
    offset,
    pin: pin ?? (subscriptions.length === 0 && staticSources ? staticSources.hour : null),
    bounds,
    widget: widgetKey,
    key: `${widget.kind}|${projection.kind}`,
  });

  const frameElement = useElement(frameRef);
  useSurfaceLabel(frameElement, enabled && !!exactSources, pipe, 'log');

  const subsKey = useMemo(
    () => JSON.stringify(subscriptions.map(s => [s.name, s.source.id, s.source.revision, s.branch, s.offset, s.channel, s.product ?? '', canonical(s.query)])),
    [subscriptions],
  );
  const subsRef = useRef(subscriptions);
  subsRef.current = subscriptions;
  const sourceIdsRef = useRef<Record<string, number>>({});
  const [subsReady, setSubsReady] = useState(0);

  useLayoutEffect(() => {
    hasPaintedRef.current = false;
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

  const latestRef = useRef({
    projection,
    options,
    density,
    months,
    twoLine,
    theme,
    staticSources,
    exactSources,
    onLoadScrollPage,
    onLimitChange,
    selectedKey,
    onActivate,
    contextMenuItems,
    viewportHeight,
    offset,
    bounds,
  });
  latestRef.current = {
    projection,
    options,
    density,
    months,
    twoLine,
    theme,
    staticSources,
    exactSources,
    onLoadScrollPage,
    onLimitChange,
    selectedKey,
    onActivate,
    contextMenuItems,
    viewportHeight,
    offset,
    bounds,
  };

  const bind = useCallback(() => {
    const heldSurface = surfaceRef.current;
    if (!heldSurface || !pipeline || pipe === null) return;
    const { width, height, scrollTop } = viewportRef.current;
    const w = Math.max(1, width || 320);
    const h = Math.max(1, height || 200);
    const cur = latestRef.current;
    const staticPanels = cur.staticSources ? Object.fromEntries(cur.staticSources.sources.panels) : undefined;
    const frozen = frozenRef.current;

    let auditId: number | undefined;
    if (perfLog && cur.exactSources) {
      const auditKey = `${cur.projection.kind}|${cur.density}|${cur.months ? 1 : 0}|${Math.round(scrollTop)}|${h}|${limitRef.current}`;
      const prev = lastAuditRef.current;
      if (prev && prev.key === auditKey && prev.options === cur.options) {
        auditId = prev.id;
      } else {
        auditId = auditStoreRef.current!.create({
          projection: cur.projection,
          options: cur.options,
          density: cur.density,
          months: cur.months,
          scrollTop,
          height: h,
          limit: limitRef.current,
          exactSources: cur.exactSources,
        });
        lastAuditRef.current = { id: auditId, key: auditKey, options: cur.options };
      }
    }

    const bindingModel: FamilyLogBinding<TOptions> = {
      family: 'log',
      kind: cur.projection.kind,
      options: cur.options,
      density: cur.density,
      months: cur.months,
      sources: sourceIdsRef.current,
      ...(staticPanels && Object.keys(staticPanels).length ? { staticPanels, staticHour: cur.staticSources?.hour ?? null } : {}),
      ...(frozen ? { frozenHour: frozen.hour, frozenPanels: frozen.panels } : {}),
      width: w,
      height: h,
      scrollTop,
      theme: cur.theme,
      selectedKey: cur.selectedKey,
      focusedKey: focusInsideRef.current ? focusedKeyRef.current : null,
      pauseFlush: pauseFlushRef.current,
      ...(perfLog && auditId !== undefined ? { audit: true, auditId } : {}),
    };

    heldSurface.canvas.style.width = `${w}px`;
    heldSurface.canvas.style.height = `${h}px`;
    pipeline.present(pipe, 'log');
    const dated = Object.keys(sourceIdsRef.current).length === 0 && cur.staticSources
      ? { h: cur.staticSources.hour, c: cur.staticSources.hour }
      : undefined;
    pipeline.bind(
      pipe,
      [heldSurface.surface],
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
    const box = mirrorRef.current;
    if (!box) return;
    if (!mirror) {
      if (lastMirrorKeyRef.current !== '') {
        lastMirrorKeyRef.current = '';
        box.replaceChildren();
      }
      return;
    }
    const key = `${mirror.first}:${mirror.total}:${mirror.rows.map(r => `${r.key}|${r.name}`).join(';')}`;
    if (key === lastMirrorKeyRef.current) return;
    lastMirrorKeyRef.current = key;
    const fragment = document.createDocumentFragment();
    for (const row of mirror.rows) {
      const div = document.createElement('div');
      div.setAttribute('role', 'option');
      div.id = `log-opt-${widget.id}-${row.key}`;
      div.dataset.rowKey = row.key;
      div.setAttribute('aria-selected', String(row.key === focusedKeyRef.current));
      div.textContent = row.name;
      fragment.append(div);
    }
    box.replaceChildren(fragment);
  }, [widget.id]);

  const jumpToLatest = useCallback(() => {
    frozenRef.current = null;
    scrollPageOffsetRef.current = 0;
    scrollPageFlightRef.current = null;
    setJumpCount(null);
    const node = scrollRef.current;
    if (node) node.scrollTop = 0;
    viewportRef.current.scrollTop = 0;
    viewportRef.current.physicalTop = 0;
    bindRef.current();
  }, []);

  useLayoutEffect(() => {
    holdElement(frameRef.current, held, STATIC_HOLD);
    if (held && !pinnedTipRef.current) hideTooltip(tooltipOwner);
    return () => holdElement(frameRef.current, false, STATIC_HOLD);
  }, [held, tooltipOwner]);

  useLayoutEffect(() => {
    const node = scrollRef.current;
    if (!node || !pipeline || pipe === null) return;
    lastExtentHeightRef.current = -1;
    const extent = document.createElement('div');
    const drawn = pipeline.held(pipe, 'log', () => {
      const canvas = document.createElement('canvas');
      canvas.className = 'family-log-canvas';
      canvas.setAttribute('aria-hidden', 'true');
      canvas.dataset.surface = 'log';
      canvas.dataset.placement = String(pipe);
      return { canvas, surface: bitmapSurface(canvas) };
    })!;
    extent.className = 'family-log-extent';
    extent.append(drawn.canvas);
    node.append(extent);
    surfaceRef.current = { canvas: drawn.canvas, extent, surface: drawn.surface };

    const updateTooltip = (tip: { key: string; content: TooltipContent | null } | null, pos: { x: number; y: number } | null) => {
      if (pinnedTipRef.current) return;
      if (!tip?.content || !pos) {
        hoverTipShownRef.current = false;
        hideTooltip(tooltipOwner);
        return;
      }
      hoverTipShownRef.current = true;
      const fitted = fitTooltip([tip.content], latestRef.current.viewportHeight);
      showTooltip(tooltipOwner, fitted, pos.x, pos.y, tip.key);
    };

    const install = () => {
      const shown = pipeline.view(pipe);
      const frameNode = frameRef.current;
      dimRef.current = !!shown?.dim;
      holdElement(frameNode, !!shown?.dim);
      const rawMeta = shown?.fields?.meta as LogMetaField | undefined;
      lastMetaRef.current = rawMeta ?? null;

      const rawMode = rawMeta?.mode ?? (shown?.mode === 'refused' ? 'refused' : 'reading');
      if (rawMode === 'picture' || rawMode === 'empty') hasPaintedRef.current = true;
      const transientReading = rawMode === 'reading' && hasPaintedRef.current;
      const mode = transientReading ? 'picture' : rawMode;
      const reason = rawMeta?.reason ?? shown?.reason ?? null;
      const paintedHour = rawMode === 'picture' || rawMode === 'empty'
        ? (shown?.through ?? shown?.painted ?? rawMeta?.hour ?? null)
        : (transientReading ? lastHourRef.current : null);
      if (transientReading) holdElement(frameNode, true);
      lastHourRef.current = rawMode === 'picture' || rawMode === 'empty'
        ? (shown?.painted ?? rawMeta?.hour ?? shown?.through ?? null)
        : lastHourRef.current;
      reportShownRef.current(paintedHour);

      if (frozenRef.current && rawMeta) {
        const delta = Math.max(0, rawMeta.liveTotal - frozenRef.current.total);
        if (delta > 0 || runtime.clock.getSnapshot().playing) {
          setJumpCount(prev => (prev === delta ? prev : delta));
        }
      }

      if (rawMeta) paintPills(rawMeta.facetPills);

      const emptySpec = rawMeta?.empty ?? null;
      const warningsList = rawMeta?.warnings ?? [];
      const nextSig = `${mode}|${reason ?? ''}|${emptySpec?.kind ?? ''}|${emptySpec?.message ?? ''}|${warningsList.length > 0 ? warningsList.join('\0') : ''}`;
      if (nextSig !== metaSigRef.current) {
        metaSigRef.current = nextSig;
        setMeta({
          mode,
          reason,
          empty: emptySpec,
          warnings: warningsList,
        });
      }

      if ((mode !== 'picture' && mode !== 'empty') || !shown?.fields) {
        paintMirror(null);
        if (!pinnedTipRef.current) {
          hoverTipShownRef.current = false;
          hideTooltip(tooltipOwner);
        }
        return;
      }

      const nav = shown.fields.nav as readonly LogNavEntry[] | undefined;
      if (nav) navRef.current = nav;
      if ('activeNav' in shown.fields) activeNavRef.current = (shown.fields.activeNav as LogNavEntry | null | undefined) ?? null;

      if (rawMeta) {
        const extH = Math.max(viewportRef.current.height, rawMeta.physicalHeight);
        if (lastExtentHeightRef.current !== extH) {
          lastExtentHeightRef.current = extH;
          extent.style.height = `${extH}px`;
        }
      }

      const mirror = shown.fields.mirror as MirrorModel | undefined;
      if (mirror) paintMirror(mirror);

      const tipField = shown.fields.tooltip as { key: string; content: TooltipContent | null } | null | undefined;
      if (!shown.dim && !heldRef.current) {
        if (inspectedKeyRef.current && lastPointerRef.current && tipField && tipField.key === inspectedKeyRef.current) {
          updateTooltip(tipField, lastPointerRef.current);
        } else if (focusInsideRef.current && focusedKeyRef.current && tipField && tipField.key === focusedKeyRef.current) {
          updateTooltip(tipField, focusAnchorRef.current);
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

    const requestPageIfNeeded = (logicalTop: number) => {
      const frozen = frozenRef.current;
      const loader = latestRef.current.onLoadScrollPage;
      const curMeta = lastMetaRef.current;
      if (!frozen || !loader || !curMeta) return;
      const targetRow = rowIndexAtLogicalY(curMeta.geometry, logicalTop);
      const pageSize = Math.max(limitRef.current, 128);
      if (targetRow < curMeta.offset || targetRow + 24 >= curMeta.offset + pageSize) {
        const aligned = Math.max(0, Math.floor(targetRow / pageSize) * pageSize);
        if (aligned === 0 && curMeta.offset === 0) return;
        if (aligned === scrollPageOffsetRef.current || aligned === scrollPageFlightRef.current) return;
        scrollPageFlightRef.current = aligned;
        void loader(frozen.hour, aligned, pageSize, frozen.baseOrdinal, frozen.total).then(reply => {
          if (scrollPageFlightRef.current !== aligned || !frozenRef.current) return;
          scrollPageFlightRef.current = null;
          scrollPageOffsetRef.current = aligned;
          frozenRef.current = { ...frozenRef.current, panels: { ...frozenRef.current.panels, tail: reply, __offset: aligned } };
          bindRef.current();
        }).catch(() => {
          if (scrollPageFlightRef.current === aligned) scrollPageFlightRef.current = null;
        });
      }
    };

    const onScroll = () => {
      const physTop = node.scrollTop;
      const curMeta = lastMetaRef.current;
      const vh = viewportRef.current.height || 200;
      const logicalTop = curMeta ? logicalScrollFromPhysical(curMeta.geometry, vh, physTop) : physTop;
      viewportRef.current.physicalTop = physTop;
      viewportRef.current.scrollTop = logicalTop;

      if (logicalTop <= 1) {
        if (frozenRef.current) {
          frozenRef.current = null;
          scrollPageOffsetRef.current = 0;
          scrollPageFlightRef.current = null;
          setJumpCount(null);
        }
      } else if (!frozenRef.current && lastHourRef.current !== null && curMeta) {
        const snapHour = lastHourRef.current;
        frozenRef.current = { hour: snapHour, baseOrdinal: curMeta.baseOrdinal, total: curMeta.total, panels: {} };
        if (runtime.clock.getSnapshot().playing) setJumpCount(0);
        requestPageIfNeeded(logicalTop);
      } else if (frozenRef.current) {
        requestPageIfNeeded(logicalTop);
      }
      bindRef.current();
    };

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
        const changed = hitKey !== inspectedKeyRef.current;
        inspectedKeyRef.current = hitKey;
        pipeline.inspect(pipe, hitKey);
        if (!hitKey) {
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
            const tipField = pipeline.view(pipe)?.fields?.tooltip as { key: string; content: TooltipContent | null } | null | undefined;
            if (tipField && tipField.key === inspectedKeyRef.current) updateTooltip(tipField, lastPointerRef.current);
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
        if (!pinnedTipRef.current && !focusInsideRef.current) {
          hoverTipShownRef.current = false;
          hideTooltip(tooltipOwner);
        }
      }, HOVER_CLOSE_MS);
    };

    const onPointerDown = (event: PointerEvent) => {
      if (event.button !== 0 && event.button !== 2) return;
      if (heldRef.current || dimRef.current) return;
      pointerDownRef.current = { key: inspectedKeyRef.current, x: event.clientX, y: event.clientY, button: event.button, cancelled: false };
    };

    const onPointerUp = () => {
      const down = pointerDownRef.current;
      pointerDownRef.current = null;
      if (!down || down.cancelled || down.button !== 0 || !down.key) return;
      const active = activeNavRef.current;
      const entry = active && active.key === down.key ? active : navRef.current.find(item => item.key === down.key);
      if (entry) latestRef.current.onActivate?.(entry.key, entry.meta, entry.hour);
    };

    const onContextMenu = (event: MouseEvent) => {
      const key = inspectedKeyRef.current;
      const builder = latestRef.current.contextMenuItems;
      if (!key || !builder || heldRef.current || dimRef.current) return;
      const active = activeNavRef.current;
      const entry = active && active.key === key ? active : navRef.current.find(item => item.key === key);
      if (!entry) return;
      const items = builder(entry.key, entry.hour, entry.meta);
      if (items.length) {
        event.preventDefault();
        contextMenuRef.current.open(event.clientX, event.clientY, items);
      }
    };

    const updateSize = (w: number, h: number) => {
      viewportRef.current.width = w;
      viewportRef.current.height = h;
      const needed = tailLimitForViewport(h, latestRef.current.density, latestRef.current.twoLine);
      if (needed !== limitRef.current) {
        limitRef.current = needed;
        latestRef.current.onLimitChange?.(needed);
      }
      const box = frameRef.current?.getBoundingClientRect();
      if (box) focusAnchorRef.current = { x: box.left + Math.min(200, box.width / 2), y: box.top + 48 };
      bindRef.current();
    };

    const resizeObserver = new ResizeObserver(entries => {
      const rect = entries[0]?.contentRect;
      if (!rect) return;
      updateSize(Math.round(rect.width), Math.round(rect.height));
    });
    resizeObserver.observe(node);
    updateSize(node.clientWidth, node.clientHeight);

    node.addEventListener('scroll', onScroll, { passive: true });
    node.addEventListener('pointermove', onPointerMove);
    node.addEventListener('pointerleave', onPointerLeave);
    node.addEventListener('pointerdown', onPointerDown);
    node.addEventListener('pointerup', onPointerUp);
    node.addEventListener('contextmenu', onContextMenu);

    return () => {
      stopInstall();
      resizeObserver.disconnect();
      node.removeEventListener('scroll', onScroll);
      node.removeEventListener('pointermove', onPointerMove);
      node.removeEventListener('pointerleave', onPointerLeave);
      node.removeEventListener('pointerdown', onPointerDown);
      node.removeEventListener('pointerup', onPointerUp);
      node.removeEventListener('contextmenu', onContextMenu);
      extent.remove();
      surfaceRef.current = null;
      lastExtentHeightRef.current = -1;
      holdElement(frameRef.current, false);
    };
  }, [pipeline, pipe, paintMirror, runtime.clock, tooltipOwner]);

  const auditable = !!exactSources;
  useLayoutEffect(() => {
    if (!perfLog || !pipeline || pipe === null || !auditable) return;
    pipeline.audit(pipe, LOG_AUDIT, fields => auditRowsOf(fields.audit));
    perfLog.reader(pipe, LOG_AUDIT, async (hour, auditId) => {
      const snap = auditStoreRef.current!.require(auditId);
      const exact = await snap.exactSources(hour, snap.limit);
      if (!exact) return null;
      const model = snap.projection.project(exact, hour, snap.options);
      const layout = buildLogLayout(model, snap.density, snap.months);
      const [start, end] = visibleItemRange(layout.items, snap.scrollTop, snap.height);
      const values = new Map<string, string>();
      if (model.total === 0 && model.rows.length === 0 && layout.items.length === 0) {
        values.set('__empty__', JSON.stringify(['empty', model.empty.kind, model.empty.message]));
      } else {
        for (let i = start; i < end; i++) {
          const item = layout.items[i]!;
          if (item.kind !== 'row') continue;
          values.set(item.row.key, canonicalLogRowAudit(item.row));
        }
      }
      return { values, ordered: true };
    });
    return () => pipeline.audit(pipe, LOG_AUDIT, null);
  }, [pipeline, pipe, auditable]);

  useLayoutEffect(() => {
    if (pipe !== null && enabled) bind();
  }, [pipe, enabled, bind, options, density, months, theme, staticSources, selectedKey, subsReady, auditable, exactSources]);

  useEffect(() => {
    let wasPlaying = runtime.clock.getSnapshot().playing;
    return runtime.clock.subscribe(() => {
      const nowPlaying = runtime.clock.getSnapshot().playing;
      if (wasPlaying && !nowPlaying && pipeline && pipe !== null) {
        pauseFlushRef.current++;
        bindRef.current();
        const mirror = pipeline.view(pipe)?.fields?.mirror as MirrorModel | undefined;
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
    if (!nav.length) return;
    const curKey = focusedKeyRef.current ?? nav[0]!.key;
    const curIdx = nav.findIndex(item => item.key === curKey);
    const idx = curIdx >= 0 ? curIdx : 0;
    const curMeta = lastMetaRef.current;
    const pageStep = Math.max(1, Math.floor((viewportRef.current.height || 200) / Math.max(1, curMeta?.rowHeight ?? 28)) - 1);

    const focusAt = (targetIdx: number) => {
      const entry = nav[Math.max(0, Math.min(nav.length - 1, targetIdx))];
      if (!entry) return;
      focusedKeyRef.current = entry.key;
      const node = scrollRef.current;
      if (node && curMeta) {
        const st = viewportRef.current.scrollTop;
        const vh = viewportRef.current.height || 200;
        let nextTop = st;
        if (entry.top < st) nextTop = entry.top;
        else if (entry.top + entry.height > st + vh) nextTop = entry.top + entry.height - vh;
        if (nextTop !== st) {
          viewportRef.current.scrollTop = nextTop;
          node.scrollTop = physicalScrollFromLogical(curMeta.geometry, vh, nextTop);
        }
      }
      const box = frameRef.current?.getBoundingClientRect();
      if (box) focusAnchorRef.current = { x: box.left + Math.min(200, box.width / 2), y: box.top + 48 };
      bindRef.current();
    };

    if (event.key === 'ArrowDown') { event.preventDefault(); focusAt(idx + 1); return; }
    if (event.key === 'ArrowUp') { event.preventDefault(); focusAt(idx - 1); return; }
    if (event.key === 'Home') { event.preventDefault(); focusAt(0); return; }
    if (event.key === 'End') { event.preventDefault(); focusAt(nav.length - 1); return; }
    if (event.key === 'PageDown') { event.preventDefault(); focusAt(idx + pageStep); return; }
    if (event.key === 'PageUp') { event.preventDefault(); focusAt(idx - pageStep); return; }
    if (event.key === 'Enter') {
      event.preventDefault();
      const entry = nav[idx]!;
      latestRef.current.onActivate?.(entry.key, entry.meta, entry.hour);
      return;
    }
    if (event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) {
      const entry = nav[idx]!;
      const items = latestRef.current.contextMenuItems?.(entry.key, entry.hour, entry.meta) ?? [];
      if (items.length) {
        event.preventDefault();
        const box = frameRef.current?.getBoundingClientRect();
        contextMenuRef.current.open((box?.left ?? 0) + 48, (box?.top ?? 0) + 48, items);
      }
      return;
    }
    if ((event.key === 'p' || event.key === 'P') && !event.ctrlKey && !event.metaKey && !event.altKey) {
      event.preventDefault();
      if (pinnedTipRef.current) {
        pinnedTipRef.current = null;
        hideTooltip(tooltipOwner);
        return;
      }
      const tipField = pipeline && pipe !== null ? pipeline.view(pipe)?.fields?.tooltip as { key: string; content: TooltipContent | null } | null | undefined : null;
      if (tipField?.content) {
        const box = frameRef.current?.getBoundingClientRect();
        const pos = { x: (box?.left ?? 0) + 120, y: (box?.top ?? 0) + 48 };
        const pinned: TooltipContent = { ...tipField.content, subtitle: `Pinned · ${tipField.content.subtitle ?? ''}` };
        pinnedTipRef.current = { key: tipField.key, content: pinned, x: pos.x, y: pos.y };
        showTooltip(tooltipOwner, fitTooltip([pinned], latestRef.current.viewportHeight), pos.x, pos.y, tipField.key);
      }
    }
  }, [pipeline, pipe, tooltipOwner]);

  const pillsNode = headerSlot
    ? createPortal(<span ref={pillsRef} className="family-log-pills" />, headerSlot)
    : null;

  return (
    <div ref={frameRef} className={`family-log${className ? ` ${className}` : ''}`} data-subject={subject}>
      {pillsNode}
      {!headerSlot && <span ref={pillsRef} className="family-log-pills family-log-pills-inline" />}
      {note && <div className="family-log-note">{note}</div>}
      {controls && ((controls.chipGroups?.length ?? 0) > 0 || (controls.pickers?.length ?? 0) > 0) && (
        <div className="family-log-toolbar">
          {controls.chipGroups?.map(group => (
            <ChipSet
              key={group.key}
              label={group.label}
              chips={[...group.chips]}
              selected={[...group.selected]}
              max={group.max}
              full={group.full}
              onToggle={group.onToggle}
            />
          ))}
          {controls.pickers?.map(picker => (
            <ContentSetPicker
              key={picker.key}
              label={picker.label}
              automatic={picker.automatic}
              choices={[...picker.choices]}
              selected={[...picker.selected]}
              onToggle={picker.onToggle}
              onReset={picker.onReset}
            />
          ))}
        </div>
      )}
      <div className="family-log-body">
        {blockedState ?? (
          <>
            {jumpCount !== null && (
              <button type="button" className="family-log-jump" onClick={jumpToLatest}>
                {jumpCount > 0 ? `${jumpCount.toLocaleString()} new · Jump to latest` : 'Jump to latest'}
              </button>
            )}
            <div
              ref={scrollRef}
              className="family-log-scroll"
              role="listbox"
              aria-label={label}
              tabIndex={0}
              onKeyDown={handleKeyDown}
              onFocus={() => {
                focusInsideRef.current = true;
                if (!focusedKeyRef.current && navRef.current.length > 0) {
                  focusedKeyRef.current = navRef.current[0]!.key;
                }
                bindRef.current();
              }}
              onBlur={() => {
                focusInsideRef.current = false;
                if (!pinnedTipRef.current && !inspectedKeyRef.current) hideTooltip(tooltipOwner);
                bindRef.current();
              }}
            />
            {meta.mode === 'reading' && <LogSkeleton />}
            {meta.mode === 'refused' && <WidgetState kind="unavailable" message={meta.reason ?? `Couldn't read ${subject}.`} />}
            {meta.mode === 'empty' && meta.empty && (
              <WidgetState kind={meta.empty.kind} message={meta.empty.message} detail={meta.empty.detail} />
            )}
            <div ref={mirrorRef} className="family-log-mirror" aria-hidden="false" />
            <div ref={liveRegionRef} className="family-log-mirror" aria-live="polite" />
          </>
        )}
      </div>
      {contextMenu.menu}
    </div>
  );
}
