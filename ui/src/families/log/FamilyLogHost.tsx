import { useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { useContextMenu, type MenuItem } from '../../components/ContextMenu';
import { ChipSet } from '../../components/ChipSet';
import ContentSetPicker from '../../components/ContentSetPicker';
import { useViewportHeight } from '../../charts/useChartTokens';
import { dayLabel } from '../../charts/draw';
import { missingNote, missingThrough } from '../../charts/log/logSource.ts';
import '../../charts/log/log.css';
import type { MissingSpan } from '../../generated/dashboard';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { FailureOrigin } from '../../dashboard/follow/page.ts';
import type { WidgetSpec } from '../../dashboard/model';
import { PreparingPage } from '../../dashboard/preparing.ts';
import { asFailure, type Failure } from '../../dashboard/queryError.ts';
import { useWidgetSkeleton } from '../../dashboard/widgetFrame.tsx';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { holdElement } from '../../draw/bitmapSurface.ts';
import { perfLog } from '../../shell/perf.ts';
import type { FamilySources } from '../contract.ts';
import {
  attachAuditWatch,
  bindFamilyPointer,
  handleFamilyKeyDown,
  mountFamilyCanvas,
  optionElementId,
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
  type ListboxMirrorItem,
  type MountedFamilyCanvas,
} from '../host/index.ts';
import type { LogControlsSpec, LogExactReader, LogProjection, LogSourceSubscription } from './declaration.ts';
import type { FamilyLogBinding, LogMetaField, LogNavEntry } from './laneView.ts';
import {
  LOG_MIN_TAIL_LIMIT,
  logicalScrollFromPhysical,
  physicalScrollFromLogical,
  rowIndexAtLogicalY,
  tailLimitForViewport,
} from './layout.ts';
import type { LogDensity, LogFacetPill } from './model.ts';
import { logThemeFromFamily } from './painter.ts';
import {
  reconcileFrozenView,
  verifyLogAuditSnapshot,
  visiblePageIndices,
  type FrozenViewState,
  type LogAuditSnapshot,
} from './frozenView.ts';
import { isLogTailReply } from './tailReply.ts';
import './logFamily.css';

const LOG_AUDIT = -1;

function dedupeMissingSpans(spans: readonly MissingSpan[]): readonly MissingSpan[] {
  if (spans.length <= 1) return spans;
  const byRange = new Map<string, { firstHour: number; lastHour: number; reasons: string[] }>();
  for (const span of spans) {
    const key = `${span.firstHour}:${span.lastHour}`;
    const held = byRange.get(key);
    if (!held) {
      byRange.set(key, { firstHour: span.firstHour, lastHour: span.lastHour, reasons: span.reason ? [span.reason] : [] });
    } else if (span.reason && !held.reasons.includes(span.reason)) {
      held.reasons.push(span.reason);
    }
  }
  return [...byRange.values()].map(entry => ({
    firstHour: entry.firstHour,
    lastHour: entry.lastHour,
    reason: entry.reasons.join('; '),
  }));
}

function logFadeDisabled(): boolean {
  if ((globalThis as { __logNoFade?: boolean }).__logNoFade === true) return true;
  if (typeof localStorage !== 'undefined') {
    try {
      if (localStorage.getItem('bp_log_no_fade') === '1') return true;
    } catch {
      return false;
    }
  }
  return false;
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
  readonly queryKey?: string;
  readonly exactSources?: LogExactReader;
  readonly onLoadScrollPage?: (frozenHour: number, offset: number, pageSize: number, baseOrdinal: number, total: number, signal?: AbortSignal) => Promise<unknown>;
  readonly onLimitChange?: (limit: number) => void;
  readonly onRowTags?: (tags: readonly string[]) => void;
  readonly offset: number;
  readonly pin: number | null;
  readonly bounds?: readonly [number, number] | null;
  readonly controls?: LogControlsSpec | null;
  readonly gapsSubject?: string;
  readonly note?: ReactNode;
  readonly selectedKey?: string | null;
  readonly onActivate?: (key: string, meta?: unknown, hour?: number) => void;
  readonly contextMenuItems?: (key: string, hour: number, meta?: unknown) => MenuItem[];
  readonly headerSlot?: HTMLElement | null;
  readonly onDiagnostics?: (warnings: string[]) => void;
  readonly extraWarnings?: readonly string[];
  readonly blockedState?: ReactNode;
  readonly bodyBlockedState?: ReactNode;
  readonly held?: boolean;
  readonly loadingLabel?: string;
  readonly className?: string;
}

interface LogHostMetaState {
  mode: string;
  reason: string | null;
  failureOrigin: FailureOrigin;
  empty: LogMetaField['empty'];
  warnings: readonly string[];
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
  queryKey,
  exactSources,
  onLoadScrollPage,
  onLimitChange,
  onRowTags,
  offset,
  pin,
  bounds,
  controls = null,
  gapsSubject,
  note = null,
  selectedKey = null,
  onActivate,
  contextMenuItems,
  headerSlot = null,
  onDiagnostics,
  extraWarnings,
  blockedState = null,
  bodyBlockedState = null,
  held = false,
  loadingLabel,
  className,
}: FamilyLogHostProps<TOptions>) {
  const prepared = useContext(PreparingPage);
  useWidgetSkeleton('log');
  const viewportHeight = useViewportHeight();
  const contextMenu = useContextMenu(`${label} actions`);
  const contextMenuRef = useRef(contextMenu);
  contextMenuRef.current = contextMenu;

  const frameRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const mirrorRef = useRef<HTMLDivElement>(null);
  const pillsRef = useRef<HTMLSpanElement>(null);
  const gapsRef = useRef<HTMLDivElement>(null);
  const jumpLabelRef = useRef<HTMLSpanElement>(null);
  const lastPillsSigRef = useRef('');
  const lastGapsSigRef = useRef('');
  const gapsSubjectRef = useRef(gapsSubject);
  gapsSubjectRef.current = gapsSubject;
  const lastRowTagsSigRef = useRef('');
  const focusAnchorRef = useRef({ x: 0, y: 0 });

  const viewportRef = useRef({ width: 0, height: 0, scrollTop: 0, physicalTop: 0 });
  const limitRef = useRef(LOG_MIN_TAIL_LIMIT);
  const frozenGenRef = useRef(0);
  const frozenRef = useRef<FrozenViewState | null>(null);
  const [scrolledBack, setScrolledBack] = useState(false);
  const lastJumpCountRef = useRef(0);
  const [pageError, setPageError] = useState<{ readonly failure: Failure; readonly pageIndex: number; readonly gen: number } | null>(null);
  const pendingFocusOrdinalRef = useRef<number | null>(null);

  const focusedKeyRef = useRef<string | null>(null);
  const focusInsideRef = useRef(false);
  const inspectedKeyRef = useRef<string | null>(null);
  const lastPointerRef = useRef<{ x: number; y: number } | null>(null);
  const heldRef = useRef(held);
  heldRef.current = held;
  const dimRef = useRef(false);
  const pauseFlushRef = useRef(0);
  const lastMirrorKeyRef = useRef('');
  const lastExtentHeightRef = useRef(-1);
  const navRef = useRef<readonly FamilyNavTuple[]>([]);
  const navEntriesRef = useRef<readonly LogNavEntry[]>([]);
  const activeNavRef = useRef<LogNavEntry | null>(null);
  const lastHourRef = useRef<number | null>(staticSources?.hour ?? null);
  const lastMetaRef = useRef<LogMetaField | null>(null);
  const lastMetaKeyRef = useRef<{ mode: string; reason: string | null; failureOrigin: FailureOrigin; rawMeta: unknown } | null>(null);
  const lastAuditRef = useRef<{ id: number; key: string; options: TOptions } | null>(null);
  const lastPhasesRef = useRef<unknown>(null);

  const [meta, setMeta] = useState<LogHostMetaState>(() => ({
    mode: staticSources ? 'picture' : 'reading',
    reason: null,
    failureOrigin: 'local',
    empty: null,
    warnings: [],
  }));
  const metaSigRef = useRef('');

  const { theme: familyTheme, reducedMotion } = useFamilyTheme();
  const theme = useMemo(() => (familyTheme ? logThemeFromFamily(familyTheme) : undefined), [familyTheme]);
  const { announce, hasFrame, liveRegionRef } = useFamilyAnnouncer('log');

  const enabled = active && !prepared && !blockedState && !bodyBlockedState && (subscriptions.length > 0 || staticSources !== null);
  const auditable = !!exactSources;
  const placement = useFamilyPlacement({
    family: 'log',
    declarationKind: projection.kind,
    widgetId: widget.id,
    widgetKind: widget.kind,
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
    surfaceLabel: 'log',
  });
  const { pipeline, pipe, sourceIdsRef, subsReady, instanceId, tooltipOwner, dataContextKey, retryStream, retryLocal } = placement;

  const tooltips = useFamilyTooltip(tooltipOwner, dataContextKey);
  useFamilyHold(frameRef, held, tooltips);

  const audit = useFamilyAudit<LogAuditSnapshot<TOptions>>({
    pipeline,
    pipe,
    ownerId: LOG_AUDIT,
    auditable,
  });

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

  const paintGaps = useCallback((missing: readonly MissingSpan[], until: number | null) => {
    const host = gapsRef.current;
    if (!host) return;
    const subjectName = gapsSubjectRef.current;
    const note = subjectName && missing.length > 0
      ? missingNote(missingThrough(dedupeMissingSpans(missing), until), dayLabel, subjectName)
      : null;
    if (!note) {
      if (lastGapsSigRef.current !== '') {
        lastGapsSigRef.current = '';
        host.hidden = true;
        host.replaceChildren();
      }
      return;
    }
    const sig = `${note.counted}|${note.spans.map(s => `${s.text}\0${s.detail}`).join('\n')}`;
    if (sig === lastGapsSigRef.current) return;
    lastGapsSigRef.current = sig;
    const box = document.createElement('div');
    box.className = 'log-gaps';
    box.setAttribute('role', 'note');
    box.setAttribute('aria-label', 'Hours not recorded');
    const title = document.createElement('b');
    title.textContent = 'Not recorded';
    const list = document.createElement('ul');
    for (const span of note.spans) {
      const li = document.createElement('li');
      li.dataset.tooltip = span.detail;
      li.append(span.text);
      const sr = document.createElement('span');
      sr.className = 'sr-only';
      sr.textContent = ` ${span.detail}`;
      li.append(sr);
      list.append(li);
    }
    const small = document.createElement('small');
    small.textContent = note.counted;
    box.append(title, list, small);
    host.hidden = false;
    host.replaceChildren(box);
  }, []);

  const reportShown = useShownHour();
  const reportShownRef = useRef(reportShown);
  reportShownRef.current = reportShown;

  const boundsKey = bounds ? `${bounds[0]}:${bounds[1]}` : '';
  const isStatic = staticSources !== null;
  const effectiveQueryKey = queryKey ?? `${subsReady}|${pin ?? ''}|${boundsKey}|${isStatic ? 1 : 0}`;

  const surfaceRef = useRef<MountedFamilyCanvas | null>(null);
  const latestRef = useRef({
    projection,
    options,
    density,
    months,
    twoLine,
    theme,
    reducedMotion,
    staticSources,
    queryKey: effectiveQueryKey,
    exactSources,
    onLoadScrollPage,
    onLimitChange,
    onRowTags,
    selectedKey,
    onActivate,
    contextMenuItems,
    viewportHeight,
    captureActive: audit.captureActive,
  });
  latestRef.current = {
    projection,
    options,
    density,
    months,
    twoLine,
    theme,
    reducedMotion,
    staticSources,
    queryKey: effectiveQueryKey,
    exactSources,
    onLoadScrollPage,
    onLimitChange,
    onRowTags,
    selectedKey,
    onActivate,
    contextMenuItems,
    viewportHeight,
    captureActive: audit.captureActive,
  };

  const bind = useCallback(() => {
    const mounted = surfaceRef.current;
    if (!mounted || !pipeline || pipe === null) return;
    const { width, height, scrollTop } = viewportRef.current;
    const w = Math.max(1, width || 360);
    const h = Math.max(1, height || 240);
    const cur = latestRef.current;
    const frozen = frozenRef.current;
    const staticPanels = cur.staticSources ? Object.fromEntries(cur.staticSources.sources.panels) : undefined;

    let auditId: number | undefined;
    if (perfLog && cur.captureActive && cur.exactSources) {
      const pagesKey = frozen ? frozen.loadedPages.map(p => `${p.pageIndex}:${p.offset}:${p.limit}`).join(',') : '';
      const frozenKey = frozen
        ? `${frozen.gen}:${frozen.hour}:${frozen.offset}:${frozen.pageSize}:${frozen.visibleStart}:${frozen.visibleEnd}:${pagesKey}`
        : 'live';
      const snapKey = `${cur.queryKey}|${cur.projection.kind}|${cur.density}|${cur.months ? 1 : 0}|${Math.round(scrollTop)}|${h}|${limitRef.current}|${frozenKey}|${cur.staticSources ? 1 : 0}`;
      if (lastAuditRef.current && lastAuditRef.current.key === snapKey && lastAuditRef.current.options === cur.options) {
        auditId = lastAuditRef.current.id;
      } else {
        auditId = audit.capture({
          queryKey: cur.queryKey,
          projection: cur.projection,
          options: cur.options,
          density: cur.density,
          months: cur.months,
          scrollTop,
          height: h,
          limit: limitRef.current,
          isStatic: cur.staticSources !== null,
          exactSources: cur.exactSources,
          onLoadScrollPage: cur.onLoadScrollPage,
          frozen: frozen ? {
            hour: frozen.hour,
            baseOrdinal: frozen.baseOrdinal,
            total: frozen.total,
            pageSize: frozen.pageSize,
            offset: frozen.offset,
            visibleStart: frozen.visibleStart,
            visibleEnd: frozen.visibleEnd,
            loadedPages: [...frozen.loadedPages],
          } : null,
        }, verifyLogAuditSnapshot);
        lastAuditRef.current = { id: auditId, key: snapKey, options: cur.options };
      }
    }

    const noFade = logFadeDisabled();
    const bindingModel: FamilyLogBinding<TOptions> = {
      family: 'log',
      kind: cur.projection.kind,
      options: cur.options,
      density: cur.density,
      months: cur.months,
      sources: sourceIdsRef.current,
      ...(staticPanels && Object.keys(staticPanels).length ? { staticPanels } : {}),
      ...(cur.staticSources ? { staticHour: cur.staticSources.hour } : {}),
      ...(frozen ? {
        frozenGen: frozen.gen,
        frozenHour: frozen.hour,
        frozenLiveSeq: frozen.liveSeq,
        frozenTotal: frozen.total,
        ...(frozen.panels ? { frozenPanels: frozen.panels } : {}),
      } : {}),
      width: w,
      height: h,
      scrollTop,
      theme: cur.theme,
      selectedKey: cur.selectedKey,
      focusedKey: focusInsideRef.current ? focusedKeyRef.current : null,
      reducedMotion: cur.reducedMotion,
      ...(noFade ? { fadeEnabled: false } : {}),
      pauseFlush: pauseFlushRef.current,
      ...(perfLog && cur.captureActive && auditId !== undefined ? { audit: true, auditId } : {}),
    };
    mounted.canvas.style.width = `${w}px`;
    mounted.canvas.style.height = `${h}px`;
    pipeline.present(pipe, 'log');
    const dated = Object.keys(sourceIdsRef.current).length === 0 && cur.staticSources
      ? { h: cur.staticSources.hour, c: cur.staticSources.hour }
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

  const paintMirror = useCallback((mirror: { readonly total: number; readonly items: readonly ListboxMirrorItem[] } | null) => {
    const mirrorHost = mirrorRef.current;
    const scrollNode = scrollRef.current;
    if (!mirrorHost) return;
    const { activeDescendantId } = syncListboxMirror(mirrorHost, mirror?.items ?? null, {
      optionIdPrefix: `log-${instanceId}`,
      total: mirror?.total ?? 0,
      focusedKey: focusedKeyRef.current,
      lastMirrorKeyRef,
    });
    if (scrollNode) {
      if (activeDescendantId) scrollNode.setAttribute('aria-activedescendant', activeDescendantId);
      else scrollNode.removeAttribute('aria-activedescendant');
    }
  }, [instanceId]);

  const refreshFocusAnchor = useCallback(() => {
    const box = frameRef.current?.getBoundingClientRect();
    if (box) focusAnchorRef.current = { x: box.left + Math.min(240, box.width / 2), y: box.top + 56 };
  }, []);

  const abortFrozenFlights = useCallback((frozen: FrozenViewState | null) => {
    if (!frozen) return;
    for (const controller of frozen.inFlight.values()) controller.abort();
    frozen.inFlight.clear();
  }, []);

  const requestPageIndex = useCallback((frozen: FrozenViewState, pageIndex: number) => {
    const loader = latestRef.current.onLoadScrollPage;
    if (!loader) return;
    if (frozen.pages.has(pageIndex) || frozen.inFlight.has(pageIndex)) return;
    const offset = pageIndex * frozen.pageSize;
    if (offset >= frozen.total) return;
    const controller = new AbortController();
    frozen.inFlight.set(pageIndex, controller);
    frozen.errors.delete(pageIndex);
    const { gen, queryKey: reqQueryKey } = frozen;
    void loader(frozen.hour, offset, frozen.pageSize, frozen.baseOrdinal, frozen.total, controller.signal).then(
      pageTail => {
        if (frozen.inFlight.get(pageIndex) === controller) frozen.inFlight.delete(pageIndex);
        if (controller.signal.aborted) return;
        const curFrozen = frozenRef.current;
        if (!curFrozen || curFrozen.gen !== gen || curFrozen.queryKey !== reqQueryKey) return;
        if (!isLogTailReply(pageTail)) return;
        curFrozen.pages.set(pageIndex, pageTail);
        curFrozen.errors.delete(pageIndex);
        const { activeError } = reconcileFrozenView(curFrozen);
        setPageError(activeError ? { ...activeError, gen: curFrozen.gen } : null);
        bindRef.current();
      },
      error => {
        if (frozen.inFlight.get(pageIndex) === controller) frozen.inFlight.delete(pageIndex);
        if (controller.signal.aborted) return;
        const curFrozen = frozenRef.current;
        if (!curFrozen || curFrozen.gen !== gen || curFrozen.queryKey !== reqQueryKey) return;
        curFrozen.errors.set(pageIndex, asFailure(error));
        const { activeError } = reconcileFrozenView(curFrozen);
        setPageError(activeError ? { ...activeError, gen: curFrozen.gen } : null);
      },
    );
  }, []);

  const ensureRangeLoaded = useCallback((wantStart: number, wantEnd: number) => {
    const frozen = frozenRef.current;
    if (!frozen || frozen.total <= 0) return;
    const clampedStart = Math.max(0, Math.min(frozen.total - 1, wantStart));
    const clampedEnd = Math.max(clampedStart + 1, Math.min(frozen.total, wantEnd));
    frozen.visibleStart = clampedStart;
    frozen.visibleEnd = clampedEnd;
    const needed = visiblePageIndices(clampedStart, clampedEnd, frozen.pageSize, frozen.total);
    for (const pageIndex of needed) {
      if (frozen.pages.has(pageIndex)) continue;
      if (pageIndex === 0 && clampedEnd <= frozen.tailLoadedCount) continue;
      requestPageIndex(frozen, pageIndex);
    }
    const { activeError } = reconcileFrozenView(frozen);
    setPageError(activeError ? { ...activeError, gen: frozen.gen } : null);
  }, [requestPageIndex]);

  const jumpToLatest = useCallback(() => {
    abortFrozenFlights(frozenRef.current);
    frozenGenRef.current++;
    frozenRef.current = null;
    pendingFocusOrdinalRef.current = null;
    lastJumpCountRef.current = 0;
    setPageError(null);
    setScrolledBack(false);
    const node = scrollRef.current;
    if (node) node.scrollTop = 0;
    viewportRef.current.scrollTop = 0;
    viewportRef.current.physicalTop = 0;
    bindRef.current();
  }, [abortFrozenFlights]);

  const prevResetScopeRef = useRef(effectiveQueryKey);
  useLayoutEffect(() => {
    if (prevResetScopeRef.current === effectiveQueryKey) return;
    prevResetScopeRef.current = effectiveQueryKey;
    abortFrozenFlights(frozenRef.current);
    frozenGenRef.current++;
    frozenRef.current = null;
    pendingFocusOrdinalRef.current = null;
    lastJumpCountRef.current = 0;
    setPageError(null);
    setScrolledBack(false);
    if (scrollRef.current) scrollRef.current.scrollTop = 0;
    viewportRef.current.scrollTop = 0;
    viewportRef.current.physicalTop = 0;
  }, [effectiveQueryKey, abortFrozenFlights]);

  const createFrozenFromMeta = useCallback((m: LogMetaField): FrozenViewState => {
    const pageSize = Math.max(limitRef.current, 128);
    return {
      gen: ++frozenGenRef.current,
      queryKey: latestRef.current.queryKey,
      hour: m.hour!,
      liveSeq: m.liveSeq,
      baseOrdinal: m.baseOrdinal,
      total: m.total,
      tailLoadedCount: m.loadedCount,
      pageSize,
      pages: new Map(),
      inFlight: new Map(),
      errors: new Map(),
      visibleStart: 0,
      visibleEnd: Math.max(1, m.loadedCount),
      offset: 0,
      loadedPages: [],
      panels: null,
    };
  }, []);

  const scrollToLogicalTop = useCallback((logicalTop: number) => {
    const node = scrollRef.current;
    const geom = lastMetaRef.current?.geometry;
    if (!node || !geom) return;
    const clamped = Math.max(0, Math.min(Math.max(0, geom.logicalHeight - viewportRef.current.height), logicalTop));
    const physical = physicalScrollFromLogical(geom, viewportRef.current.height, clamped);
    node.scrollTop = physical;
    viewportRef.current.physicalTop = physical;
    viewportRef.current.scrollTop = clamped;
    if (clamped > 0 && !frozenRef.current && lastMetaRef.current && lastMetaRef.current.hour !== null) {
      frozenRef.current = createFrozenFromMeta(lastMetaRef.current);
      lastJumpCountRef.current = 0;
      setScrolledBack(true);
    } else if (clamped === 0 && frozenRef.current) {
      abortFrozenFlights(frozenRef.current);
      frozenGenRef.current++;
      frozenRef.current = null;
      lastJumpCountRef.current = 0;
      setPageError(null);
      setScrolledBack(false);
    }
  }, [abortFrozenFlights, createFrozenFromMeta]);

  const focusRowByKey = useCallback((key: string) => {
    focusedKeyRef.current = key;
    const entry = navEntriesRef.current.find(item => item.key === key);
    const node = scrollRef.current;
    const geom = lastMetaRef.current?.geometry;
    if (entry && node && geom) {
      const curTop = viewportRef.current.scrollTop;
      const vh = viewportRef.current.height;
      let nextLogical = curTop;
      if (entry.top < curTop) nextLogical = entry.top;
      else if (entry.top + entry.height > curTop + vh) nextLogical = entry.top + entry.height - vh;
      if (nextLogical !== curTop) scrollToLogicalTop(nextLogical);
    }
    if (node) {
      node.setAttribute('aria-activedescendant', optionElementId(`log-${instanceId}`, key));
    }
    bindRef.current();
  }, [instanceId, scrollToLogicalTop]);

  const paintMirrorRef = useRef(paintMirror);
  paintMirrorRef.current = paintMirror;
  const paintPillsRef = useRef(paintPills);
  paintPillsRef.current = paintPills;
  const paintGapsRef = useRef(paintGaps);
  paintGapsRef.current = paintGaps;
  const ensureRangeLoadedRef = useRef(ensureRangeLoaded);
  ensureRangeLoadedRef.current = ensureRangeLoaded;
  const refreshFocusAnchorRef = useRef(refreshFocusAnchor);
  refreshFocusAnchorRef.current = refreshFocusAnchor;

  const focusByOrdinal = useCallback((ordinal: number) => {
    const match = navEntriesRef.current.find(item => item.ordinal >= 0 && item.ordinal === ordinal);
    if (match) {
      pendingFocusOrdinalRef.current = null;
      focusRowByKey(match.key);
      return;
    }
    const metaNow = lastMetaRef.current;
    if (!metaNow || metaNow.total <= 0) return;
    const clamped = Math.max(0, Math.min(metaNow.total - 1, ordinal));
    pendingFocusOrdinalRef.current = clamped;
    const targetTop = clamped * metaNow.rowHeight;
    scrollToLogicalTop(Math.max(0, targetTop - Math.floor(viewportRef.current.height / 3)));
    ensureRangeLoaded(clamped, clamped + 1);
    bindRef.current();
  }, [ensureRangeLoaded, focusRowByKey, scrollToLogicalTop]);

  useLayoutEffect(() => {
    const node = scrollRef.current;
    if (!node || !pipeline || pipe === null) return;
    lastExtentHeightRef.current = -1;
    lastMetaKeyRef.current = null;
    lastPhasesRef.current = null;

    const mounted = mountFamilyCanvas(node, pipeline, pipe, 'log', 'family-log-canvas', 'family-log-extent');
    surfaceRef.current = mounted;

    const auditWatch = attachAuditWatch(pipeline, pipe, {
      isAuditable: () => !!latestRef.current.exactSources,
      onCaptureChange: activeAudit => { latestRef.current.captureActive = activeAudit; },
      bindNow: () => bindRef.current(),
    });

    const install = () => {
      const shown = pipeline.view(pipe);
      const frameNode = frameRef.current;
      dimRef.current = !!shown?.dim;
      holdElement(frameNode, !!shown?.dim);
      const viewMode = shown?.mode ?? 'reading';
      const visible = viewMode === 'picture';
      const rawMeta = shown?.fields?.meta as LogMetaField | undefined;
      const effectiveMode = !visible
        ? viewMode
        : (rawMeta?.mode === 'empty' ? 'picture' : (rawMeta?.mode ?? 'picture'));
      const reason = shown?.reason ?? rawMeta?.reason ?? null;
      const failureOrigin: FailureOrigin = shown?.failureOrigin ?? 'local';
      const isPictureWithModel = visible && rawMeta !== undefined && rawMeta.mode !== 'reading';

      node.toggleAttribute('data-log-visible', isPictureWithModel && rawMeta.mode !== 'empty');
      node.toggleAttribute('data-log-pending', !isPictureWithModel);

      if (isPictureWithModel && rawMeta.hour !== null) {
        lastHourRef.current = rawMeta.hour;
        reportFamilyShownHour(reportShownRef.current, true, shown, rawMeta.hour);
      } else {
        reportFamilyShownHour(reportShownRef.current, false, shown, null);
      }

      auditWatch.settleIfReady(visible, viewMode, shown?.installedBinding, shown?.fields);

      if (rawMeta) lastMetaRef.current = rawMeta;
      paintGapsRef.current(rawMeta?.missing ?? [], isPictureWithModel && rawMeta.hour !== null ? rawMeta.hour : lastHourRef.current);
      const prevMetaKey = lastMetaKeyRef.current;
      if (
        !prevMetaKey
        || prevMetaKey.mode !== effectiveMode
        || prevMetaKey.reason !== reason
        || prevMetaKey.failureOrigin !== failureOrigin
        || prevMetaKey.rawMeta !== rawMeta
      ) {
        lastMetaKeyRef.current = { mode: effectiveMode, reason, failureOrigin, rawMeta };
        const empty = rawMeta?.mode === 'empty' ? rawMeta.empty : null;
        const warnings = rawMeta?.warnings ?? [];
        const nextMetaSig = JSON.stringify([effectiveMode, reason, failureOrigin, empty?.kind ?? '', empty?.message ?? '', empty?.detail ?? '', warnings]);
        if (nextMetaSig !== metaSigRef.current) {
          metaSigRef.current = nextMetaSig;
          setMeta({ mode: effectiveMode, reason, failureOrigin, empty, warnings });
        }
      }

      if (!isPictureWithModel || !shown?.fields) {
        paintMirrorRef.current(null);
        paintPillsRef.current([]);
        tooltips.hideIfUnpinned();
        return;
      }

      if (rawMeta.facetPills) paintPillsRef.current(rawMeta.facetPills);
      if (rawMeta.rowTags && latestRef.current.onRowTags) {
        const sig = rawMeta.rowTags.join(',');
        if (sig !== lastRowTagsSigRef.current) {
          lastRowTagsSigRef.current = sig;
          latestRef.current.onRowTags(rawMeta.rowTags);
        }
      }

      const extH = Math.max(viewportRef.current.height, rawMeta.physicalHeight);
      if (extH !== lastExtentHeightRef.current) {
        lastExtentHeightRef.current = extH;
        mounted.extent.style.height = `${extH}px`;
      }

      const liveNewCount = typeof shown.fields.liveNewCount === 'number' ? shown.fields.liveNewCount : 0;
      if (liveNewCount !== lastJumpCountRef.current || (jumpLabelRef.current && !jumpLabelRef.current.textContent)) {
        lastJumpCountRef.current = liveNewCount;
        if (jumpLabelRef.current) {
          jumpLabelRef.current.textContent = liveNewCount > 0
            ? `${liveNewCount.toLocaleString()} new · Jump to latest`
            : 'Jump to latest';
        }
      }

      const navTuples = shown.fields.nav as readonly FamilyNavTuple[] | undefined;
      if (navTuples) navRef.current = navTuples;
      const navEntries = shown.fields.navEntries as readonly LogNavEntry[] | undefined;
      if (navEntries) {
        navEntriesRef.current = navEntries;
        if (pendingFocusOrdinalRef.current !== null) {
          const wanted = navEntries.find(item => item.ordinal === pendingFocusOrdinalRef.current);
          if (wanted) {
            pendingFocusOrdinalRef.current = null;
            focusedKeyRef.current = wanted.key;
            queueMicrotask(() => bindRef.current());
          }
        }
      }
      const activeNav = shown.fields.activeNav as LogNavEntry | null | undefined;
      if (activeNav !== undefined) activeNavRef.current = activeNav;

      const mirror = shown.fields.mirror as { readonly total: number; readonly items: readonly ListboxMirrorItem[] } | undefined;
      if (mirror) paintMirrorRef.current(mirror);

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
        const opt = mirrorRef.current?.querySelector<HTMLElement>(`[data-row-key="${CSS.escape(focusedKeyRef.current)}"]`);
        if (opt) opt.setAttribute('aria-description', tooltips.describeFocused(focusTipField.text, focusTipField.text.hour, latestRef.current.viewportHeight));
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
      accessibleRef: scrollRef,
      isBlocked: () => heldRef.current || dimRef.current,
      getViewportHeight: () => latestRef.current.viewportHeight,
      onActivate: key => {
        const entry = (activeNavRef.current?.key === key ? activeNavRef.current : null)
          ?? navEntriesRef.current.find(item => item.key === key);
        latestRef.current.onActivate?.(key, entry?.meta, entry?.hour ?? lastHourRef.current ?? undefined);
      },
      onContextMenu: (key, clientX, clientY) => {
        const builder = latestRef.current.contextMenuItems;
        if (!builder || !key) return false;
        const entry = (activeNavRef.current?.key === key ? activeNavRef.current : null)
          ?? navEntriesRef.current.find(item => item.key === key);
        const items = builder(key, entry?.hour ?? lastHourRef.current ?? 0, entry?.meta);
        if (!items.length) return false;
        contextMenuRef.current.open(clientX, clientY, items);
        return true;
      },
    });

    const onScroll = () => {
      const physicalTop = node.scrollTop;
      const geom = lastMetaRef.current?.geometry;
      const logicalTop = geom ? logicalScrollFromPhysical(geom, viewportRef.current.height, physicalTop) : physicalTop;
      viewportRef.current.physicalTop = physicalTop;
      viewportRef.current.scrollTop = logicalTop;

      if (logicalTop === 0) {
        if (frozenRef.current) {
          abortFrozenFlights(frozenRef.current);
          frozenGenRef.current++;
          frozenRef.current = null;
          pendingFocusOrdinalRef.current = null;
          lastJumpCountRef.current = 0;
          setPageError(null);
          setScrolledBack(false);
        }
      } else {
        if (!frozenRef.current && lastMetaRef.current && lastMetaRef.current.hour !== null) {
          frozenRef.current = createFrozenFromMeta(lastMetaRef.current);
          lastJumpCountRef.current = 0;
          setScrolledBack(true);
        }
        if (frozenRef.current && geom) {
          const wantStart = rowIndexAtLogicalY(geom, logicalTop);
          const wantEnd = rowIndexAtLogicalY(geom, logicalTop + viewportRef.current.height) + 2;
          ensureRangeLoadedRef.current(wantStart, wantEnd);
        }
      }
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
      const nextLimit = tailLimitForViewport(h, latestRef.current.density, latestRef.current.twoLine);
      if (nextLimit !== limitRef.current) {
        limitRef.current = nextLimit;
        latestRef.current.onLimitChange?.(nextLimit);
      }
      bindRef.current();
    });
    resizeObserver.observe(node);
    viewportRef.current.width = node.clientWidth;
    viewportRef.current.height = node.clientHeight;
    const initLimit = tailLimitForViewport(node.clientHeight || 240, latestRef.current.density, latestRef.current.twoLine);
    if (initLimit !== limitRef.current) {
      limitRef.current = initLimit;
      latestRef.current.onLimitChange?.(initLimit);
    }
    refreshFocusAnchorRef.current();

    node.addEventListener('scroll', onScroll, { passive: true });
    bindRef.current();

    return () => {
      abortFrozenFlights(frozenRef.current);
      auditWatch.dispose();
      stopInstall();
      stopPointer();
      resizeObserver.disconnect();
      node.removeEventListener('scroll', onScroll);
      mounted.unmount();
      surfaceRef.current = null;
      lastExtentHeightRef.current = -1;
      holdElement(frameRef.current, false);
    };
  }, [pipeline, pipe, tooltips, abortFrozenFlights, createFrozenFromMeta]);

  useLayoutEffect(() => {
    paintGaps(lastMetaRef.current?.missing ?? [], lastHourRef.current);
  }, [gapsSubject, paintGaps]);

  useLayoutEffect(() => {
    if (pipe !== null && enabled) bind();
  }, [pipe, enabled, bind, options, density, months, theme, reducedMotion, staticSources, effectiveQueryKey, selectedKey, subsReady, auditable, exactSources, audit.captureActive]);

  useFamilyPauseFlush(runtime.clock, () => {
    if (!pipeline || pipe === null) return;
    pauseFlushRef.current++;
    bindRef.current();
  });

  useEffect(() => {
    if (!onDiagnostics) return;
    const all = [...(extraWarnings ?? []), ...meta.warnings];
    onDiagnostics(all);
    return () => onDiagnostics([]);
  }, [onDiagnostics, extraWarnings, meta.warnings]);

  const handleKeyDown = useCallback((event: React.KeyboardEvent) => {
    const rowHeight = lastMetaRef.current?.rowHeight ?? 28;
    const pageStep = Math.max(1, Math.floor((viewportRef.current.height || 240) / rowHeight) - 1);
    const totalCount = lastMetaRef.current?.total ?? navRef.current.length;
    handleFamilyKeyDown(event, {
      nav: navRef.current,
      totalCount,
      focusedKey: focusedKeyRef.current,
      focusedIndex: pendingFocusOrdinalRef.current ?? undefined,
      fallbackToFirstOnActivate: false,
      pageStep,
      blocked: heldRef.current || dimRef.current,
      isPinnedTooltip: () => tooltips.isPinned(),
      onFocusKey: focusRowByKey,
      onFocusOrdinal: focusByOrdinal,
      onActivate: entry => {
        const navEntry = navEntriesRef.current.find(item => item.key === entry[0]);
        const builder = latestRef.current.contextMenuItems;
        if (builder) {
          const items = builder(entry[0], navEntry?.hour ?? lastHourRef.current ?? 0, entry[5]);
          if (items.length) {
            refreshFocusAnchor();
            contextMenuRef.current.open(focusAnchorRef.current.x, focusAnchorRef.current.y, items);
            return;
          }
        }
        latestRef.current.onActivate?.(entry[0], entry[5], navEntry?.hour ?? lastHourRef.current ?? undefined);
      },
      onContextMenu: entry => {
        const builder = latestRef.current.contextMenuItems;
        if (!builder || !entry) return false;
        const navEntry = navEntriesRef.current.find(item => item.key === entry[0]);
        const items = builder(entry[0], navEntry?.hour ?? lastHourRef.current ?? 0, entry[5]);
        if (!items.length) return false;
        refreshFocusAnchor();
        contextMenuRef.current.open(focusAnchorRef.current.x, focusAnchorRef.current.y, items);
        return true;
      },
      onTogglePinTooltip: currentKey => {
        if (!pipeline || pipe === null || heldRef.current || dimRef.current || !currentKey) return;
        const focusTipField = pipeline.view(pipe)?.fields?.focusTooltip as FamilyTooltipField | undefined;
        if (focusTipField?.text && focusTipField.text.rowKey === currentKey) {
          refreshFocusAnchor();
          tooltips.pinFocused(focusTipField.text, focusTipField.text.hour, focusAnchorRef.current, latestRef.current.viewportHeight, announce);
        }
      },
      onUnpinTooltip: () => tooltips.unpin(announce),
    });
  }, [announce, focusByOrdinal, focusRowByKey, pipe, pipeline, refreshFocusAnchor, tooltips]);

  const onRetryLocal = useCallback(() => {
    if (pageError) {
      const frozen = frozenRef.current;
      if (frozen && frozen.gen === pageError.gen) {
        const idx = pageError.pageIndex;
        frozen.errors.delete(idx);
        setPageError(null);
        requestPageIndex(frozen, idx);
        return;
      }
      setPageError(null);
    }
    retryLocal(() => bindRef.current());
  }, [pageError, requestPageIndex, retryLocal]);

  if (blockedState) return <>{blockedState}</>;

  const hasControls = !!controls && ((controls.chipGroups?.length ?? 0) > 0 || (controls.pickers?.length ?? 0) > 0);
  const pillsNode = <span ref={pillsRef} className="family-log-pills" />;

  const bodyStateNode = bodyBlockedState ?? renderFamilyBodyState({
    mode: meta.mode,
    reason: meta.reason,
    failureOrigin: meta.failureOrigin,
    empty: meta.empty?.message ?? null,
    emptyDetail: meta.empty?.detail ?? null,
    emptyKind: meta.empty?.kind ?? 'empty',
    subject,
    loadingLabel: loadingLabel ?? `Loading ${subject}…`,
    pipeline,
    pipe,
    onRetryStream: retryStream,
    onRetryLocal,
    localError: pageError?.failure ?? null,
  });

  return (
    <div ref={frameRef} className={`family-log${className ? ` ${className}` : ''}`} data-density={density}>
      {headerSlot && createPortal(pillsNode, headerSlot)}
      {!headerSlot && <div className="family-log-pills-inline">{pillsNode}</div>}
      {hasControls && (
        <div className="family-log-toolbar">
          {controls!.chipGroups?.map(group => (
            <ChipSet
              key={group.key}
              label={group.label}
              chips={group.chips}
              selected={group.selected}
              max={group.max}
              full={group.full}
              onToggle={group.onToggle}
            />
          ))}
          {controls!.pickers?.map(picker => (
            <ContentSetPicker
              key={picker.key}
              label={picker.label}
              automatic={picker.automatic}
              choices={picker.choices}
              selected={picker.selected}
              onToggle={picker.onToggle}
              onReset={picker.onReset}
            />
          ))}
        </div>
      )}
      {note && <p className="family-log-note">{note}</p>}
      <div ref={gapsRef} className="family-log-gaps-slot" hidden />
      <div className="family-log-body">
        <div
          ref={scrollRef}
          className="family-log-scroll"
          tabIndex={0}
          role="listbox"
          aria-label={label}
          onFocus={() => {
            focusInsideRef.current = true;
            refreshFocusAnchor();
            if (!focusedKeyRef.current && navRef.current.length > 0) {
              focusedKeyRef.current = navRef.current[0]![0];
            }
            bindRef.current();
          }}
          onBlur={event => {
            if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
            focusInsideRef.current = false;
            if (!inspectedKeyRef.current) tooltips.hideIfUnpinned();
            bindRef.current();
          }}
          onKeyDown={handleKeyDown}
        >
          <div ref={mirrorRef} className="family-log-mirror" />
        </div>
        {bodyStateNode && <div className="family-log-state-overlay">{bodyStateNode}</div>}
        {scrolledBack && !bodyStateNode && (
          <button type="button" className="family-log-jump" onClick={jumpToLatest}>
            <span ref={jumpLabelRef}>
              {lastJumpCountRef.current > 0 ? `${lastJumpCountRef.current.toLocaleString()} new · Jump to latest` : 'Jump to latest'}
            </span>
          </button>
        )}
      </div>
      {!hasFrame && <div ref={liveRegionRef} className="family-log-mirror" role="status" aria-live="polite" aria-atomic="true" />}
      {contextMenu.menu}
    </div>
  );
}
