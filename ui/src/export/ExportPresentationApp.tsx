import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { nativeDashboardClient } from '../dashboard/nativeClient';
import { bindTracks, clockBounds, resolveWindow, type DashboardTemplate, type WidgetSpec } from '../dashboard/model';
import { includeCampaigns, resolveWidgetCampaign } from '../dashboard/campaigns';
import { PlaybackController } from '../dashboard/playback';
import { MapViewsStore } from '../dashboard/geometry';
import { widgetDefinition } from '../dashboard/definitions';
import type { DashboardSource } from '../dashboard/client';
import type { CountryChoice } from '../dashboard/mapPicker';
import DashboardGrid from '../dashboard/DashboardGrid';
import WidgetHeader from '../dashboard/WidgetHeader';
import { WidgetBody } from '../dashboard/widgets';
import { WidgetFrameContext, widgetWording, type WidgetFrameValue } from '../dashboard/widgetFrame';
import { HeldFailures, HeldFailuresContext } from '../dashboard/heldFailures';
import { FollowedCard, FollowedWidgetKey, FollowedWidgetKind } from '../dashboard/follow/store';
import { resolveWidgetActiveView } from '../widgets/manifest';
import { loadWidgetRenderer } from '../widgets/rendererLoaders';
import { LiveMotion } from '../charts/liveMotion';
import { useAppearance } from '../shell/appearance/useAppearance';
import { useWidgetIconMenu, WidgetIconsContext } from '../shell/appearance/WidgetIconMenu';
import { ExportSurfaceContext, type ExportWidgetStatus } from './exportMotion';
import { sendVideoExportPageAck, type ExportClipRect, type ExportMapRect } from './videoExportApi';
import { exportGlideDurationMs } from './videoExportModel';
import type { LaneMotion } from '../families/laneRuntime';
import '../widgets';
import '../dashboard/dashboard.css';
import './exportPresentation.css';

interface MountPayload {
  seq: number;
  kind: 'widget' | 'page';
  framing: 'full' | 'visible';
  reflowForPortrait: boolean;
  width: number;
  height: number;
  cssWidth: number;
  cssHeight: number;
  dpr: number;
  scale: number;
  sourceWidth: number;
  sourceHeight: number;
  scrollTop: number;
  fps: number;
  hoursPerSecond: number;
  dateStamp: boolean;
  followPinned: boolean;
  initialHour: number;
  payload: SurfaceMountData;
}

interface MetricsPayload {
  seq: number;
  cssWidth: number;
  cssHeight: number;
  dpr: number;
  scale: number;
}

interface StepPayload {
  seq: number;
  frameIndex: number;
  virtualNowMs: number;
  mode: 'recorded' | 'card';
  hour: number | null;
  fraction: number;
  resetContinuity: boolean;
  dateText: string | null;
  cardHeading: string | null;
  cardDates: string | null;
  cardDetail: string | null;
  cutBanner: string | null;
  cssWidth: number;
  cssHeight: number;
  dpr: number;
  scale: number;
  measureText?: boolean;
}

export interface SurfaceMountData {
  template: DashboardTemplate;
  widget?: WidgetSpec;
  campaigns: { id: string; path: string; label: string; revision?: string }[];
  selectedCountries: Record<string, CountryChoice | null>;
  selectedBranches: Record<string, string>;
  observed: Record<string, number>;
  defaultRoleId: string | null;
}

interface MountedSession {
  kind: 'widget' | 'page';
  framing: 'full' | 'visible';
  reflowForPortrait: boolean;
  outputWidth: number;
  outputHeight: number;
  cssWidth: number;
  cssHeight: number;
  dpr: number;
  scale: number;
  sourceWidth: number;
  sourceHeight: number;
  scrollTop: number;
  fps: number;
  hoursPerSecond: number;
  template: DashboardTemplate;
  widget: WidgetSpec | null;
  sources: Map<string, DashboardSource>;
  selectedBranches: Record<string, string>;
  defaultRoleId: string | null;
  campaigns: { id: string; path: string; label: string; loading: boolean }[];
}

interface StepOverlay {
  mode: 'recorded' | 'card';
  dateText: string | null;
  cutBanner: string | null;
  cardHeading: string | null;
  cardDates: string | null;
  cardDetail: string | null;
}

const noop = () => undefined;

function scheduleFrameTick(run: () => void): () => void {
  const hidden = typeof document !== 'undefined' && document.visibilityState === 'hidden';
  if (typeof requestAnimationFrame !== 'function' || hidden) {
    const timer = window.setTimeout(run, 16);
    return () => window.clearTimeout(timer);
  }
  let done = false;
  let rafId = 0;
  const finish = () => {
    if (done) return;
    done = true;
    window.clearTimeout(fallbackTimer);
    if (rafId) cancelAnimationFrame(rafId);
    run();
  };
  const fallbackTimer = window.setTimeout(finish, 32);
  rafId = requestAnimationFrame(finish);
  return () => {
    if (done) return;
    done = true;
    window.clearTimeout(fallbackTimer);
    if (rafId) cancelAnimationFrame(rafId);
  };
}

function ancestorZoom(element: HTMLElement | null, root: HTMLElement): number {
  let factor = 1;
  let cur: HTMLElement | null = element;
  while (cur && cur !== root) {
    const rawZoom = cur.style.zoom;
    if (rawZoom) {
      const parsed = parseFloat(rawZoom);
      if (Number.isFinite(parsed) && parsed > 0) factor *= parsed;
    }
    cur = cur.parentElement;
  }
  return factor;
}

function measureMinTextPx(root: HTMLElement | null, pixelFactor: number): number | null {
  if (!root) return null;
  let minSize = Infinity;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  let inspected = 0;
  while (inspected < 400) {
    const node = walker.nextNode();
    if (!node) break;
    if (!node.textContent || !node.textContent.trim()) continue;
    const parent = node.parentElement;
    if (!parent) continue;
    const style = window.getComputedStyle(parent);
    if (style.display === 'none' || style.visibility === 'hidden') continue;
    const size = parseFloat(style.fontSize);
    if (Number.isFinite(size) && size > 0) {
      const zoom = ancestorZoom(parent, root);
      minSize = Math.min(minSize, size * zoom * pixelFactor);
    }
    inspected += 1;
  }
  if (!Number.isFinite(minSize)) {
    return null;
  }
  return Math.round(minSize * 10) / 10;
}

function measureViewportClip(root: HTMLElement | null, dpr: number): ExportClipRect | null {
  if (!root) return null;
  const clipEl = root.querySelector<HTMLElement>('.export-visible-clip');
  if (!clipEl) return null;
  const box = clipEl.getBoundingClientRect();
  const x = Math.round(box.left * dpr);
  const y = Math.round(box.top * dpr);
  const width = Math.max(0, Math.round(box.width * dpr));
  const height = Math.max(0, Math.round(box.height * dpr));
  return { x, y, width, height };
}

function measureBgRgb(root: HTMLElement | null): [number, number, number] | null {
  if (!root) return null;
  const raw = window.getComputedStyle(root).backgroundColor;
  const match = raw.match(/rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)/i);
  if (!match) return null;
  return [
    Math.min(255, Math.max(0, Number(match[1]))),
    Math.min(255, Math.max(0, Number(match[2]))),
    Math.min(255, Math.max(0, Number(match[3]))),
  ];
}

function measureOverlayRects(root: HTMLElement | null, dpr: number): ExportMapRect[] {
  if (!root) return [];
  const nodes = root.querySelectorAll<HTMLElement>('.export-date-pill, .export-cut-banner');
  const out: ExportMapRect[] = [];
  for (const node of nodes) {
    const box = node.getBoundingClientRect();
    const x = Math.floor(box.left * dpr);
    const y = Math.floor(box.top * dpr);
    const width = Math.ceil(box.right * dpr) - x;
    const height = Math.ceil(box.bottom * dpr) - y;
    if (width > 0 && height > 0) {
      out.push({ widgetId: node.className, x, y, width, height });
    }
  }
  return out;
}

function measureMapRects(root: HTMLElement | null, dpr: number): ExportMapRect[] {
  if (!root) return [];
  const slots = root.querySelectorAll<HTMLElement>('[data-export-map-id]');
  const out: ExportMapRect[] = [];
  for (const slot of slots) {
    const widgetId = slot.dataset.exportMapId;
    if (!widgetId) continue;
    const box = slot.getBoundingClientRect();
    const x = Math.round(box.left * dpr);
    const y = Math.round(box.top * dpr);
    const width = Math.max(0, Math.round(box.width * dpr));
    const height = Math.max(0, Math.round(box.height * dpr));
    if (width > 0 && height > 0) {
      out.push({ widgetId, x, y, width, height });
    }
  }
  return out;
}

function applyFollowPinned(template: DashboardTemplate, followPinned: boolean): DashboardTemplate {
  if (!followPinned) return template;
  return {
    ...template,
    widgets: template.widgets.map(widget =>
      widget.time.mode === 'pin' ? { ...widget, time: { mode: 'follow' as const } } : widget,
    ),
  };
}

function SingleWidgetCard({
  widget,
  template,
  runtime,
}: {
  widget: WidgetSpec;
  template: DashboardTemplate;
  runtime: import('../dashboard/definitions').WidgetRuntime;
}) {
  const [headerSlot, setHeaderSlot] = useState<HTMLSpanElement | null>(null);
  const [countrySlot, setCountrySlot] = useState<HTMLSpanElement | null>(null);
  const [filterSlot, setFilterSlot] = useState<HTMLSpanElement | null>(null);
  const [card, setCard] = useState<HTMLElement | null>(null);
  const [failures] = useState(() => new HeldFailures());
  const definition = widgetDefinition(widget.kind);
  const wording = useMemo(() => widgetWording(definition, widget.title), [definition, widget.title]);
  const resolvedWidget = useMemo(
    () => resolveWidgetCampaign(widget, template, runtime.defaultRoleId),
    [widget, template, runtime.defaultRoleId],
  );
  const campaignTrack = runtime.tracks.find(track => track.id === resolvedWidget.trackIds[0]) ?? runtime.tracks[0];
  const frameValue = useMemo<WidgetFrameValue>(
    () => ({
      title: widget.title,
      subject: wording.subject,
      things: wording.things,
      timeMode: widget.time.mode,
      pinned: widget.time.mode === 'pin' || widget.time.mode === 'range',
      bounds: campaignTrack
        ? { firstHour: campaignTrack.branch.firstHour, lastHour: campaignTrack.branch.lastHour }
        : null,
      hasSubject: definition?.data !== false && definition?.countrySelection !== false,
      reportState: noop,
      announce: noop,
      clearAnnouncement: noop,
      setDynamicViewSwitch: noop,
      skeleton: definition?.skeleton ?? 'generic',
      setSkeleton: noop,
    }),
    [widget.title, widget.time.mode, wording.subject, wording.things, campaignTrack, definition],
  );
  return (
    <article ref={setCard} className={`dash-widget kind-${widget.kind}`} data-widget={widget.id}>
      <WidgetHeader
        widget={widget}
        definition={definition}
        effective={widget}
        campaignTrack={campaignTrack}
        runtime={runtime}
        template={template}
        editing={false}
        readOnly
        active
        activeSwitch={null}
        summarySlotRef={setHeaderSlot}
        countrySlotRef={setCountrySlot}
        filterSlotRef={setFilterSlot}
        headerSlotElement={headerSlot}
      />
      <WidgetFrameContext.Provider value={frameValue}>
        <div className="dash-widget-content">
          <div className="dash-widget-scaled" style={{ zoom: (widget.contentScale ?? 100) / 100 }}>
            <HeldFailuresContext.Provider value={failures}>
              <FollowedWidgetKey.Provider value={widget.id}>
                <FollowedWidgetKind.Provider value={widget.kind}>
                  <FollowedCard.Provider value={card}>
                    <WidgetBody
                      widget={widget}
                      runtime={runtime}
                      active
                      headerSlot={headerSlot}
                      countrySlot={countrySlot}
                      filterSlot={filterSlot}
                    />
                  </FollowedCard.Provider>
                </FollowedWidgetKind.Provider>
              </FollowedWidgetKey.Provider>
            </HeldFailuresContext.Provider>
          </div>
        </div>
      </WidgetFrameContext.Provider>
    </article>
  );
}

export default function ExportPresentationApp() {
  const appearance = useAppearance();
  const widgetIconMenu = useWidgetIconMenu(appearance);
  const rootRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const clockRef = useRef<PlaybackController | null>(null);
  if (!clockRef.current) {
    clockRef.current = new PlaybackController(undefined, 'export-clock');
  }
  const clock = clockRef.current;
  const mapsRef = useRef<MapViewsStore | null>(null);
  if (!mapsRef.current) {
    mapsRef.current = new MapViewsStore(nativeDashboardClient, clock);
  }
  const maps = mapsRef.current;

  const [session, setSession] = useState<MountedSession | null>(null);
  const [motion, setMotion] = useState<LaneMotion | undefined>(undefined);
  const [overlay, setOverlay] = useState<StepOverlay>({
    mode: 'recorded',
    dateText: null,
    cutBanner: null,
    cardHeading: null,
    cardDates: null,
    cardDetail: null,
  });

  const sessionRef = useRef<MountedSession | null>(null);
  sessionRef.current = session;
  const openedSourcesRef = useRef<string[]>([]);
  const expectedFamilyIdsRef = useRef<string[]>([]);
  const widgetStatusesRef = useRef(new Map<string, ExportWidgetStatus>());
  const statusWaitersRef = useRef(new Set<() => void>());
  const opGenRef = useRef(0);
  const lastSignatureRef = useRef<string>('');
  const lastModeRef = useRef<'recorded' | 'card'>('card');

  const wakeWaiters = useCallback(() => {
    for (const wake of [...statusWaitersRef.current]) wake();
  }, []);

  const nextAnimationFrames = useCallback(async (count = 2, opGen?: number): Promise<void> => {
    let remaining = Math.max(1, count);
    while (remaining > 0) {
      if (opGen !== undefined && opGenRef.current !== opGen) {
        throw new Error('cancelled');
      }
      await new Promise<void>(resolve => {
        let settled = false;
        const finish = () => {
          if (settled) return;
          settled = true;
          cancelTick();
          statusWaitersRef.current.delete(onWake);
          resolve();
        };
        const onWake = () => {
          if (opGen !== undefined && opGenRef.current !== opGen) {
            finish();
          }
        };
        const cancelTick = scheduleFrameTick(finish);
        if (opGen !== undefined) {
          statusWaitersRef.current.add(onWake);
        }
      });
      remaining -= 1;
    }
    if (opGen !== undefined && opGenRef.current !== opGen) {
      throw new Error('cancelled');
    }
  }, []);

  const releaseOpenedSources = useCallback(async () => {
    const ids = openedSourcesRef.current.splice(0, openedSourcesRef.current.length);
    for (const id of ids) {
      await nativeDashboardClient.releaseSource(id).catch(() => undefined);
    }
  }, []);

  const reportWidget = useCallback((key: string, status: ExportWidgetStatus | null) => {
    if (status === null) widgetStatusesRef.current.delete(key);
    else widgetStatusesRef.current.set(key, status);
    wakeWaiters();
  }, [wakeWaiters]);

  const waitForWidgetsSettled = useCallback(
    async (
      expectedBindings: Map<string, number>,
      expectedTargets: Map<string, number | null>,
      opGen: number,
      timeoutMs = 15000,
    ) => {
      const start = performance.now();
      const artBudgetMs = 2500;
      const expectedIds = expectedFamilyIdsRef.current;
      const unsettled = (allowPendingArt: boolean): string[] => {
        const missing: string[] = [];
        const presentByWidget = new Map<string, ExportWidgetStatus[]>();
        for (const status of widgetStatusesRef.current.values()) {
          const list = presentByWidget.get(status.widgetId) ?? [];
          list.push(status);
          presentByWidget.set(status.widgetId, list);
        }
        for (const widgetId of expectedIds) {
          if (!presentByWidget.has(widgetId)) {
            missing.push(`${widgetId}:unmounted`);
          }
        }
        for (const [key, status] of widgetStatusesRef.current) {
          if (status.mode === 'reading') {
            missing.push(`${key}:reading`);
            continue;
          }
          if (status.mode === 'unavailable' || status.mode === 'failed' || status.mode === 'refused') {
            continue;
          }
          const expectedHour = expectedTargets.get(status.widgetId);
          if (status.mode === 'outside') {
            if (
              expectedHour !== undefined &&
              expectedHour !== null &&
              status.target !== null &&
              status.target !== expectedHour
            ) {
              missing.push(`${key}:hour(${status.target}!=${expectedHour})`);
            }
            continue;
          }
          const minBinding = expectedBindings.get(key) ?? 0;
          if (status.binding > 0 && status.installedBinding < Math.max(status.binding, minBinding)) {
            missing.push(`${key}:binding(${status.installedBinding}<${Math.max(status.binding, minBinding)})`);
            continue;
          }
          if (status.mode === 'picture' && status.expired) {
            missing.push(`${key}:expired`);
            continue;
          }
          if (
            expectedHour !== undefined &&
            expectedHour !== null &&
            status.mode === 'picture' &&
            status.target !== null &&
            status.target !== expectedHour
          ) {
            missing.push(`${key}:hour(${status.target}!=${expectedHour})`);
            continue;
          }
          if (!allowPendingArt && (status.pendingArt || status.pendingIdentities)) {
            missing.push(`${key}:art`);
          }
        }
        return missing;
      };
      while (performance.now() - start < timeoutMs) {
        if (opGenRef.current !== opGen) {
          throw new Error('cancelled');
        }
        const allowArtTimeout = performance.now() - start >= artBudgetMs;
        if (unsettled(allowArtTimeout).length === 0) return;
        await new Promise<void>(resolve => {
          const timer = window.setTimeout(() => {
            statusWaitersRef.current.delete(done);
            resolve();
          }, 25);
          const done = () => {
            window.clearTimeout(timer);
            statusWaitersRef.current.delete(done);
            resolve();
          };
          statusWaitersRef.current.add(done);
        });
      }
      if (opGenRef.current !== opGen) {
        throw new Error('cancelled');
      }
      const remaining = unsettled(true);
      if (remaining.length > 0) {
        throw new Error(`Export timed out waiting for widgets to settle: ${remaining.join(', ')}`);
      }
    },
    [],
  );

  const computeSignature = useCallback((step: StepPayload) => {
    const parts: string[] = [
      step.mode,
      step.dateText ?? '',
      step.cutBanner ?? '',
      `${step.cssWidth}x${step.cssHeight}@${step.dpr}:${step.scale}`,
    ];
    const keys = [...widgetStatusesRef.current.keys()].sort();
    for (const key of keys) {
      const s = widgetStatusesRef.current.get(key)!;
      parts.push(`${key}=${s.mode}:${s.installedId ?? 'n'}:${s.installedBinding}:${s.target ?? 'n'}`);
    }
    return parts.join('|');
  }, []);

  useEffect(() => {
    let alive = true;
    const unlisteners: (() => void)[] = [];
    let serial = Promise.resolve();

    const enqueue = (task: () => Promise<void>) => {
      serial = serial.then(task).catch(() => undefined);
    };

    const expectedTargetsFor = (curSession: MountedSession, clockHour: number | null): Map<string, number | null> => {
      const map = new Map<string, number | null>();
      if (clockHour === null) return map;
      const bound = bindTracks(curSession.template, curSession.sources, curSession.selectedBranches);
      const widgets = curSession.kind === 'widget' && curSession.widget ? [curSession.widget] : curSession.template.widgets;
      for (const rawWidget of widgets) {
        const widget = resolveWidgetCampaign(rawWidget, curSession.template, curSession.defaultRoleId);
        const track = bound.find(t => t.id === widget.trackIds[0]);
        if (!track) {
          map.set(widget.id, null);
          continue;
        }
        const win = resolveWindow(widget, track, bound, clockHour);
        const inBranch = win !== null && win.lastHour >= track.branch.firstHour && win.lastHour <= track.branch.lastHour;
        map.set(widget.id, inBranch ? Math.floor(win.lastHour) : null);
      }
      return map;
    };

    const handleMount = async (payload: MountPayload) => {
      const opGen = ++opGenRef.current;
      const openedThisMount: string[] = [];
      try {
        setSession(null);
        await nextAnimationFrames(1, opGen);
        await releaseOpenedSources();
        if (opGenRef.current !== opGen) throw new Error('cancelled');
        widgetStatusesRef.current.clear();
        expectedFamilyIdsRef.current = [];
        lastSignatureRef.current = '';
        lastModeRef.current = 'card';
        const data = payload.payload;
        const openedSources = new Map<string, DashboardSource>();
        for (const campaign of data.campaigns) {
          if (opGenRef.current !== opGen) throw new Error('cancelled');
          const source = await nativeDashboardClient.openSource(campaign.path, new AbortController().signal);
          openedThisMount.push(source.id);
          if (campaign.revision && source.revision !== campaign.revision) {
            throw new Error(
              'The recording changed on disk while preparing this export. Re-open the export dialog to retry.',
            );
          }
          openedSources.set(campaign.id, source);
        }
        openedSourcesRef.current = openedThisMount;
        const campaigns = data.campaigns.map(c => ({ ...c, loading: false }));
        const baseTemplate = applyFollowPinned(data.template, payload.followPinned);
        const merged = includeCampaigns(baseTemplate, campaigns);
        const template: DashboardTemplate = {
          ...merged,
          roles: merged.roles.map(role => ({
            ...role,
            selectedTag: data.selectedCountries[role.id]?.tag ?? null,
            selectedSlot: data.selectedCountries[role.id]?.slot ?? null,
            observed: data.observed,
          })),
        };
        const targetWidget = payload.kind === 'widget' && data.widget
          ? (payload.followPinned && data.widget.time.mode === 'pin'
              ? { ...data.widget, time: { mode: 'follow' as const } }
              : data.widget)
          : null;
        const activeWidgets = targetWidget ? [targetWidget] : template.widgets;
        const tracks = bindTracks(template, openedSources, data.selectedBranches);
        const boundTrackIds = new Set(tracks.map(t => t.id));
        const loadedRenderers = await Promise.all(
          activeWidgets.map(async w => {
            if (w.kind === 'map') {
              await loadWidgetRenderer('map');
              const resolved = resolveWidgetCampaign(w, template, data.defaultRoleId);
              return {
                id: w.id,
                expected: resolved.trackIds.some(id => boundTrackIds.has(id)),
              };
            }
            const activeView = resolveWidgetActiveView(w.kind, w.options);
            const renderer =
              (await loadWidgetRenderer(w.kind, activeView)) ??
              (await loadWidgetRenderer(w.kind));
            return {
              id: w.id,
              expected: Boolean(renderer && 'declaration' in renderer && renderer.declaration),
            };
          }),
        );
        if (opGenRef.current !== opGen) throw new Error('cancelled');
        expectedFamilyIdsRef.current = loadedRenderers.filter(r => r.expected).map(r => r.id);
        clock.setBounds(clockBounds(tracks), true);
        clock.stepExport(payload.initialHour, payload.hoursPerSecond, true);

        const nextSession: MountedSession = {
          kind: payload.kind,
          framing: payload.framing,
          reflowForPortrait: payload.reflowForPortrait,
          outputWidth: payload.width,
          outputHeight: payload.height,
          cssWidth: payload.cssWidth,
          cssHeight: payload.cssHeight,
          dpr: payload.dpr,
          scale: payload.scale,
          sourceWidth: payload.sourceWidth,
          sourceHeight: payload.sourceHeight,
          scrollTop: payload.scrollTop,
          fps: payload.fps,
          hoursPerSecond: payload.hoursPerSecond,
          template,
          widget: targetWidget,
          sources: openedSources,
          selectedBranches: data.selectedBranches,
          defaultRoleId: data.defaultRoleId,
          campaigns,
        };
        setOverlay({
          mode: 'recorded',
          dateText: null,
          cutBanner: null,
          cardHeading: null,
          cardDates: null,
          cardDetail: null,
        });
        setMotion({
          easeValues: true,
          glideDurationMs: exportGlideDurationMs(payload.hoursPerSecond, payload.fps),
          nowMs: 0,
          speed: payload.hoursPerSecond,
        });
        setSession(nextSession);
        await nextAnimationFrames(4, opGen);
        await waitForWidgetsSettled(new Map(), expectedTargetsFor(nextSession, payload.initialHour), opGen, 15000);
        await nextAnimationFrames(2, opGen);
        const contentNode = contentRef.current;
        const contentWidth = contentNode ? contentNode.scrollWidth : payload.cssWidth;
        const contentHeight = contentNode ? contentNode.scrollHeight : payload.cssHeight;
        const pixelFactor = payload.dpr * payload.scale;
        await sendVideoExportPageAck({
          seq: payload.seq,
          kind: 'mount',
          dirty: true,
          contentWidth,
          contentHeight,
          minTextPx: measureMinTextPx(rootRef.current, pixelFactor),
          mapRects: measureMapRects(rootRef.current, payload.dpr),
          overlayRects: measureOverlayRects(rootRef.current, payload.dpr),
          viewportClip: measureViewportClip(rootRef.current, payload.dpr),
          bgRgb: measureBgRgb(rootRef.current),
          error: null,
        });
      } catch (cause) {
        setSession(null);
        await nextAnimationFrames(1);
        for (const id of openedThisMount) {
          await nativeDashboardClient.releaseSource(id).catch(() => undefined);
        }
        openedSourcesRef.current = openedSourcesRef.current.filter(id => !openedThisMount.includes(id));
        await sendVideoExportPageAck({
          seq: payload.seq,
          kind: 'mount',
          dirty: true,
          contentWidth: null,
          contentHeight: null,
          minTextPx: null,
          mapRects: [],
          overlayRects: [],
          viewportClip: null,
          bgRgb: measureBgRgb(rootRef.current),
          error: cause instanceof Error ? cause.message : String(cause),
        });
      }
    };

    const handleMetrics = async (payload: MetricsPayload) => {
      const opGen = ++opGenRef.current;
      try {
        setSession(prev =>
          prev
            ? {
                ...prev,
                cssWidth: payload.cssWidth,
                cssHeight: payload.cssHeight,
                dpr: payload.dpr,
                scale: payload.scale,
              }
            : null,
        );
        await nextAnimationFrames(3, opGen);
        const pixelFactor = payload.dpr * payload.scale;
        await sendVideoExportPageAck({
          seq: payload.seq,
          kind: 'metrics',
          dirty: true,
          contentWidth: contentRef.current?.scrollWidth ?? null,
          contentHeight: contentRef.current?.scrollHeight ?? null,
          minTextPx: measureMinTextPx(rootRef.current, pixelFactor),
          mapRects: measureMapRects(rootRef.current, payload.dpr),
          overlayRects: measureOverlayRects(rootRef.current, payload.dpr),
          viewportClip: measureViewportClip(rootRef.current, payload.dpr),
          bgRgb: measureBgRgb(rootRef.current),
          error: null,
        });
      } catch (cause) {
        await sendVideoExportPageAck({
          seq: payload.seq,
          kind: 'metrics',
          dirty: true,
          contentWidth: null,
          contentHeight: null,
          minTextPx: null,
          mapRects: [],
          overlayRects: [],
          viewportClip: null,
          bgRgb: measureBgRgb(rootRef.current),
          error: cause instanceof Error ? cause.message : String(cause),
        });
      }
    };

    const handleStep = async (payload: StepPayload) => {
      const opGen = ++opGenRef.current;
      try {
        const curSession = sessionRef.current;
        if (!curSession) {
          throw new Error('Export page received a step before mounting.');
        }
        if (
          curSession.cssWidth !== payload.cssWidth ||
          curSession.cssHeight !== payload.cssHeight ||
          Math.abs(curSession.dpr - payload.dpr) > 1e-4 ||
          Math.abs(curSession.scale - payload.scale) > 1e-4
        ) {
          setSession(prev =>
            prev
              ? {
                  ...prev,
                  cssWidth: payload.cssWidth,
                  cssHeight: payload.cssHeight,
                  dpr: payload.dpr,
                  scale: payload.scale,
                }
              : null,
          );
        }
        setOverlay({
          mode: payload.mode,
          dateText: payload.dateText,
          cutBanner: payload.cutBanner,
          cardHeading: payload.cardHeading,
          cardDates: payload.cardDates,
          cardDetail: payload.cardDetail,
        });
        if (payload.mode === 'card') {
          lastModeRef.current = 'card';
          lastSignatureRef.current = '';
          await nextAnimationFrames(2, opGen);
          await sendVideoExportPageAck({
            seq: payload.seq,
            kind: 'step',
            dirty: true,
            contentWidth: null,
            contentHeight: null,
            minTextPx: null,
            mapRects: [],
            overlayRects: [],
            viewportClip: null,
            bgRgb: measureBgRgb(rootRef.current),
            error: null,
          });
          return;
        }
        const expectedBindings = new Map<string, number>();
        for (const [key, st] of widgetStatusesRef.current) {
          expectedBindings.set(key, st.binding);
        }
        const glideDurationMs = exportGlideDurationMs(curSession.hoursPerSecond, curSession.fps);
        setMotion({
          easeValues: true,
          glideDurationMs,
          nowMs: payload.virtualNowMs,
          speed: curSession.hoursPerSecond,
        });
        if (payload.hour !== null) {
          clock.stepExport(
            payload.hour + payload.fraction,
            curSession.hoursPerSecond,
            payload.resetContinuity,
          );
        }
        await nextAnimationFrames(1, opGen);
        await waitForWidgetsSettled(expectedBindings, expectedTargetsFor(curSession, payload.hour), opGen, 15000);
        await nextAnimationFrames(2, opGen);

        const sig = computeSignature(payload);
        const mapRects = measureMapRects(rootRef.current, payload.dpr);
        const overlayRects = measureOverlayRects(rootRef.current, payload.dpr);
        const viewportClip = measureViewportClip(rootRef.current, payload.dpr);
        const dirty =
          payload.frameIndex === 0 ||
          lastModeRef.current !== 'recorded' ||
          sig !== lastSignatureRef.current ||
          mapRects.length > 0;
        lastSignatureRef.current = sig;
        lastModeRef.current = 'recorded';

        await sendVideoExportPageAck({
          seq: payload.seq,
          kind: 'step',
          dirty,
          contentWidth: contentRef.current?.scrollWidth ?? null,
          contentHeight: contentRef.current?.scrollHeight ?? null,
          minTextPx:
            payload.measureText || payload.frameIndex === 0
              ? measureMinTextPx(rootRef.current, payload.dpr * payload.scale)
              : null,
          mapRects,
          overlayRects,
          viewportClip,
          bgRgb: measureBgRgb(rootRef.current),
          error: null,
        });
      } catch (cause) {
        await sendVideoExportPageAck({
          seq: payload.seq,
          kind: 'step',
          dirty: true,
          contentWidth: null,
          contentHeight: null,
          minTextPx: null,
          mapRects: [],
          overlayRects: [],
          viewportClip: null,
          bgRgb: measureBgRgb(rootRef.current),
          error: cause instanceof Error ? cause.message : String(cause),
        });
      }
    };

    const handleUnmount = async (payload?: { seq?: number } | null) => {
      opGenRef.current += 1;
      wakeWaiters();
      widgetStatusesRef.current.clear();
      expectedFamilyIdsRef.current = [];
      lastSignatureRef.current = '';
      setSession(null);
      setMotion(undefined);
      await nextAnimationFrames(1);
      await releaseOpenedSources();
      if (payload && typeof payload.seq === 'number') {
        await sendVideoExportPageAck({
          seq: payload.seq,
          kind: 'unmount',
          dirty: false,
          contentWidth: null,
          contentHeight: null,
          minTextPx: null,
          mapRects: [],
          overlayRects: [],
          viewportClip: null,
          bgRgb: null,
          error: null,
        });
      }
    };

    const onBeforeUnload = () => {
      opGenRef.current += 1;
      wakeWaiters();
      void releaseOpenedSources();
    };
    window.addEventListener('beforeunload', onBeforeUnload);

    void Promise.all([
      listen<MountPayload>('video-export-mount', event => enqueue(() => handleMount(event.payload))),
      listen<MetricsPayload>('video-export-metrics', event => enqueue(() => handleMetrics(event.payload))),
      listen<StepPayload>('video-export-step', event => enqueue(() => handleStep(event.payload))),
      listen<{ seq?: number } | null>('video-export-unmount', event => {
        opGenRef.current += 1;
        wakeWaiters();
        enqueue(() => handleUnmount(event.payload));
      }),
    ]).then(stops => {
      if (!alive) {
        for (const stop of stops) stop();
        return;
      }
      unlisteners.push(...stops);
      void sendVideoExportPageAck({
        seq: 0,
        kind: 'boot',
        dirty: false,
        contentWidth: null,
        contentHeight: null,
        minTextPx: null,
        mapRects: [],
        overlayRects: [],
        viewportClip: null,
        bgRgb: measureBgRgb(rootRef.current),
        error: null,
      });
    });

    return () => {
      alive = false;
      opGenRef.current += 1;
      wakeWaiters();
      window.removeEventListener('beforeunload', onBeforeUnload);
      for (const stop of unlisteners) stop();
      void releaseOpenedSources();
    };
  }, [clock, waitForWidgetsSettled, computeSignature, releaseOpenedSources, wakeWaiters, nextAnimationFrames]);

  const tracks = useMemo(() => {
    if (!session) return [];
    return bindTracks(session.template, session.sources, session.selectedBranches);
  }, [session]);

  const runtime = useMemo(() => {
    if (!session) return null;
    return {
      onMapBranch: noop,
      tracks,
      clock,
      maps,
      client: nativeDashboardClient,
      campaignTracks: session.template.tracks,
      defaultRoleId: session.defaultRoleId,
      alignment: session.template.alignment,
      nativeHidden: true,
      campaigns: session.campaigns,
      recordings: [],
      assignCampaign: async () => undefined,
      requestSeek: () => false,
      addWidgetBelow: noop,
    };
  }, [session, tracks, clock, maps]);

  const exportContext = useMemo(
    () => ({
      active: true,
      motion,
      reportWidget,
    }),
    [motion, reportWidget],
  );

  const isPortraitReflow = Boolean(
    session &&
      session.framing === 'visible' &&
      session.reflowForPortrait &&
      session.outputHeight > session.outputWidth,
  );
  const pageWidth = useMemo(() => {
    if (!session) return 1280;
    if (isPortraitReflow) {
      return Math.round(session.cssWidth / Math.max(1e-4, session.scale));
    }
    return session.sourceWidth;
  }, [session, isPortraitReflow]);

  return (
    <ExportSurfaceContext.Provider value={exportContext}>
      <WidgetIconsContext.Provider value={widgetIconMenu.icons}>
        <div ref={rootRef} className="export-presentation-root">
          {session && runtime && (
            <LiveMotion.Provider value={clock.getMotion}>
              {session.kind === 'widget' && session.widget ? (
                <div className="export-single-widget-shell" ref={contentRef}>
                  <SingleWidgetCard widget={session.widget} template={session.template} runtime={runtime} />
                </div>
              ) : session.framing === 'visible' && !isPortraitReflow ? (
                <div className="export-presentation-stage">
                  <div
                    className="export-visible-clip"
                    style={{
                      width: `${Math.round(session.sourceWidth * session.scale)}px`,
                      height: `${Math.round(session.sourceHeight * session.scale)}px`,
                    }}
                  >
                    <div
                      ref={contentRef}
                      className="export-page-content is-visible-clipped"
                      style={{
                        width: `${pageWidth}px`,
                        transform: `scale(${session.scale}) translateY(${-Math.max(0, session.scrollTop)}px)`,
                      }}
                    >
                      <DashboardGrid
                        template={session.template}
                        runtime={runtime}
                        editing={false}
                        onLayouts={noop}
                        onApply={noop}
                        onRemove={noop}
                      />
                    </div>
                  </div>
                </div>
              ) : (
                <div
                  className={`export-presentation-stage${session.framing === 'visible' ? ' is-visible-mode' : ''}`}
                >
                  <div
                    ref={contentRef}
                    className="export-page-content"
                    style={{
                      width: `${pageWidth}px`,
                      transform:
                        session.framing === 'visible'
                          ? `scale(${session.scale}) translateY(${-Math.max(0, session.scrollTop)}px)`
                          : `scale(${session.scale})`,
                    }}
                  >
                    <DashboardGrid
                      template={session.template}
                      runtime={runtime}
                      editing={false}
                      onLayouts={noop}
                      onApply={noop}
                      onRemove={noop}
                    />
                  </div>
                </div>
              )}
            </LiveMotion.Provider>
          )}

          {overlay.mode === 'recorded' && overlay.dateText && (
            <div className="export-date-pill">{overlay.dateText}</div>
          )}

          {overlay.mode === 'recorded' && overlay.cutBanner && (
            <div className="export-cut-banner">{overlay.cutBanner}</div>
          )}

          {overlay.mode === 'card' && (
            <div className="export-card-overlay">
              <h1>{overlay.cardHeading ?? 'Unrecorded stretch'}</h1>
              {overlay.cardDates && <p className="export-card-dates">{overlay.cardDates}</p>}
              {overlay.cardDetail && <p className="export-card-detail">{overlay.cardDetail}</p>}
            </div>
          )}
        </div>
      </WidgetIconsContext.Provider>
    </ExportSurfaceContext.Provider>
  );
}
