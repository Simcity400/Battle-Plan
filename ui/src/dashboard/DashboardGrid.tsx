import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { flushSync } from 'react-dom';
import { Responsive, useContainerWidth } from 'react-grid-layout';
import type { Layout, ResponsiveLayouts } from 'react-grid-layout';
import 'react-grid-layout/css/styles.css';
import 'react-resizable/css/styles.css';
import { Icon } from '../components/Icons';
import WidgetState from '../components/WidgetState';
import type { MapOptions } from '../shell/api';
import { createWidget, GRID_COLUMNS, resolveWindow, widgetAtDefault } from './model';
import { resolveWidgetCampaign } from './campaigns';
import { useFactionStylesKey } from './campaignSettings';
import type { Breakpoint, DashboardTemplate, GridLayouts, WidgetSpec } from './model';
import { DEFAULT_MAP_OPTIONS, flipFog } from './mapPicker';
import { widgetDefinition } from './definitions';
import type { WidgetRuntime, WidgetSkeletonKind } from './definitions';
import { OptionDraft } from './optionDraft';
import type { OptionValue } from './options';
import WidgetHeader from './WidgetHeader';
import { resolveDeclaredViewSwitch, WidgetFrameContext, widgetWording } from './widgetFrame';
import type { DynamicViewSwitch, WidgetFrameValue } from './widgetFrame';
import { formatDateTime } from './calendar';
import { WidgetBody } from './widgets';
import { HeldFailures, HeldFailuresContext } from './heldFailures';
import HeldFailureNotices from './HeldFailureNotices';
import { MeasuredWidget } from './widgetPerf';
import { FollowedCard, FollowedWidgetKey, FollowedWidgetKind } from './follow/store.ts';
import { usePipelineGeneration } from './follow/useFollowed.ts';
import { WidgetFrames } from './widgetClock';
import { perfLog } from '../shell/perf';
import { DASHBOARD_SCROLL, NEAR_SCREEN } from './follow/useFollow.ts';
import { preloadAllWidgetRenderers, preloadWidgetRenderers } from '../widgets/rendererLoaders';
import { widgetOptionFields } from '../widgets/manifest.ts';
import { commitZone } from '../shell/commitCount';
import { engine } from '../engine/frame.ts';

const RESET_UNDO_MS = 6000;
const READING_BADGE_DELAY_MS = 400;

interface Props {
  prepared?: boolean;
  template: DashboardTemplate;
  runtime: WidgetRuntime;
  editing: boolean;
  onLayouts: (layouts: GridLayouts) => void;
  onApply: (widget: WidgetSpec, view?: boolean) => void;
  onRemove: (id: string) => void;
}

function DashboardGrid({ prepared = false, template, runtime, editing, onLayouts, onApply, onRemove }: Props) {
  const { width, containerRef, mounted } = useContainerWidth({ initialWidth: 1200 });
  const kinds = [...new Set(template.widgets.map(widget => widget.kind))].join(' ');
  useEffect(() => { preloadWidgetRenderers(kinds.split(' ')); preloadAllWidgetRenderers(); }, [kinds]);
  const changed = useCallback((_layout: Layout, layouts: ResponsiveLayouts<Breakpoint>) => {
    runtime.maps.schedule();
    const next: GridLayouts = {};
    for (const breakpoint of Object.keys(GRID_COLUMNS) as Breakpoint[]) {
      const layout = layouts[breakpoint];
      if (layout) next[breakpoint] = layout.map(({ i, x, y, w, h, minW, minH }) => ({ i, x, y, w, h, minW, minH }));
    }
    onLayouts(next);
  }, [onLayouts, runtime.maps]);
  return <div className={`dash-grid${editing ? ' is-editing' : ''}`} ref={containerRef}>
    {mounted && <Responsive<Breakpoint> width={width} layouts={template.layouts}
      breakpoints={{ lg: 1280, md: 850, sm: 520, xs: 0 }} cols={GRID_COLUMNS} rowHeight={26} margin={[6, 6]} containerPadding={[0, 0]}
      dragConfig={{ enabled: editing, handle: '.dash-drag-grip', cancel: 'input,select,textarea,a' }}
      resizeConfig={{ enabled: editing, handles: ['se'] }} onLayoutChange={changed}
      onDrag={runtime.maps.schedule} onDragStop={runtime.maps.schedule} onResize={runtime.maps.schedule} onResizeStop={runtime.maps.schedule}>
      {template.widgets.map((widget) => <div key={widget.id} className="dash-grid-cell">
        <WidgetFrame prepared={prepared} widget={widget} template={template} runtime={runtime} editing={editing} onApply={onApply} onRemove={onRemove} />
      </div>)}
    </Responsive>}
  </div>;
}

const MOUNT_BUDGET = 4;
type Place = Pick<IntersectionObserverEntry, 'isIntersecting' | 'boundingClientRect'>;
const waiting = new Map<() => void, Place>();
let mountFrame = false;

function mountInTurn(mount: () => void, entry: Place): () => void {
  waiting.set(mount, entry);
  if (!mountFrame) { mountFrame = true; engine.next('commit', mountWaiting); }
  return () => { waiting.delete(mount); };
}

function mountWaiting() {
  const start = performance.now();
  const order = [...waiting].sort(([, a], [, b]) => Number(b.isIntersecting) - Number(a.isIntersecting)
    || a.boundingClientRect.top - b.boundingClientRect.top || a.boundingClientRect.left - b.boundingClientRect.left);
  for (const [mount] of order) {
    if (performance.now() - start >= MOUNT_BUDGET) break;
    waiting.delete(mount);
    flushSync(mount);
  }
  mountFrame = waiting.size > 0;
  if (mountFrame) engine.next('commit', mountWaiting);
}

export const bodiesWaiting = (): number => waiting.size;

const onScreen = (box: DOMRect, viewport: DOMRect) =>
  box.width > 0 && box.height > 0 && box.bottom > viewport.top && box.top < viewport.bottom && box.right > viewport.left && box.left < viewport.right;

const Body = memo(WidgetBody);

function WidgetFrame({ prepared = false, widget, template, runtime, editing, onApply, onRemove }: Omit<Props, 'onLayouts'> & { widget: WidgetSpec }) {
  const element = useRef<HTMLElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const [card, setCard] = useState<HTMLElement | null>(null);
  const attach = useCallback((node: HTMLElement | null) => { element.current = node; setCard(node); }, []);
  const [active, setActive] = useState<boolean | null>(typeof IntersectionObserver === 'undefined' ? true : null);
  const [headerSlot, setHeaderSlot] = useState<HTMLSpanElement | null>(null);
  const [countrySlot, setCountrySlot] = useState<HTMLSpanElement | null>(null);
  const [filterSlot, setFilterSlot] = useState<HTMLSpanElement | null>(null);
  const [captionSlot, setCaptionSlot] = useState<HTMLDivElement | null>(null);
  const [hasBodyState, setHasBodyState] = useState(false);
  const stateOwners = useRef(new Set<object>());
  const [dynamicSwitch, setDynamicSwitch] = useState<DynamicViewSwitch | null>(null);
  const switchOwners = useRef(new Map<object, DynamicViewSwitch>());
  const [customSkeleton, setCustomSkeleton] = useState<WidgetSkeletonKind | (() => ReactNode) | null>(null);
  const skeletonOwners = useRef(new Map<object, WidgetSkeletonKind | (() => ReactNode)>());
  const [announcement, setAnnouncement] = useState('');
  const lastAnnounced = useRef('');
  const [readingBadge, setReadingBadge] = useState<string | null>(null);
  const openCountriesRef = useRef<(() => void) | undefined>(undefined);
  const openTimeRef = useRef<(() => void) | undefined>(undefined);
  const [hasCountryOpener, setHasCountryOpener] = useState(false);
  const [hasTimeOpener, setHasTimeOpener] = useState(false);
  const [failures] = useState(() => new HeldFailures());
  const [slots] = useState(() => {
    const noted: Record<'header' | 'country' | 'filter', HTMLSpanElement | null> = { header: null, country: null, filter: null };
    let shown = false;
    const slot = (key: keyof typeof noted, set: (element: HTMLSpanElement | null) => void) => (element: HTMLSpanElement | null) => {
      noted[key] = element;
      if (shown) set(element);
    };
    return { header: slot('header', setHeaderSlot), country: slot('country', setCountrySlot), filter: slot('filter', setFilterSlot),
      show() { shown = true; setHeaderSlot(noted.header); setCountrySlot(noted.country); setFilterSlot(noted.filter); } };
  });

  useLayoutEffect(() => {
    const target = element.current;
    if (!target) return;
    if (prepared) return mountInTurn(() => { slots.show(); setActive(true); }, { isIntersecting: false, boundingClientRect: target.getBoundingClientRect() });
    const root = target.closest(DASHBOARD_SCROLL);
    let mounted = false, visible = false, cancel = () => {};
    const mount = () => { mounted = true; slots.show(); setActive(visible); };
    if (root && onScreen(target.getBoundingClientRect(), root.getBoundingClientRect())) { visible = true; mount(); }
    const near = new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting;
      if (mounted) setActive(visible); else cancel = mountInTurn(mount, entry);
    }, { root, rootMargin: NEAR_SCREEN });
    const seen = new IntersectionObserver(([entry]) => {
      if (mounted || !entry.isIntersecting) return;
      cancel();
      visible = true;
      flushSync(mount);
    }, { root });
    near.observe(target);
    seen.observe(target);
    return () => { near.disconnect(); seen.disconnect(); cancel(); };
  }, [slots, prepared]);

  useEffect(() => { if (!prepared) perfLog?.widget(widget.id, widget.kind, widget.title); }, [widget.id, widget.kind, widget.title, prepared]);
  const [mapOptions, setMapOptions] = useState<MapOptions>(DEFAULT_MAP_OPTIONS);
  useEffect(() => prepared ? undefined : runtime.maps.onFog(widget.id, () => setMapOptions(flipFog)), [runtime.maps, widget.id, prepared]);

  const [resetUndo, setResetUndo] = useState<{ widget: WidgetSpec; mapOptions: MapOptions } | null>(null);
  useEffect(() => {
    if (!resetUndo) return;
    const timer = setTimeout(() => setResetUndo(null), RESET_UNDO_MS);
    return () => clearTimeout(timer);
  }, [resetUndo]);

  const applyWidget = useCallback((next: WidgetSpec, view?: boolean) => {
    setResetUndo(null);
    onApply(next, view);
  }, [onApply]);

  const definition = widgetDefinition(widget.kind);
  const frames = useMemo(() => new WidgetFrames(runtime.clock, widget.id), [runtime.clock, widget.id]);
  useLayoutEffect(() => { frames.setShown(active === true && !prepared); }, [frames, active, prepared]);
  const bodyRuntime = useMemo(() => ({ ...runtime, clock: frames.clock }), [runtime, frames]);
  const optionDraft = useRef(new OptionDraft(widget.options));
  optionDraft.current.sync(widget.options);
  const saveOptions = useCallback((values: Readonly<Record<string, OptionValue>>) => {
    const options = optionDraft.current.updateAll(widgetOptionFields(widget.kind, widget.options), values);
    if (options) applyWidget({ ...widget, options }, true);
  }, [widget, applyWidget]);
  const saveOption = useCallback((key: string, value: OptionValue) => saveOptions({ [key]: value }), [saveOptions]);

  const defaultTrack = template.tracks[0]?.id;
  const reset = definition && defaultTrack !== undefined
    && !(widgetAtDefault(widget, defaultTrack) && mapOptions.country === null && mapOptions.fog === DEFAULT_MAP_OPTIONS.fog) ? () => {
    setResetUndo({ widget, mapOptions });
    onApply(createWidget(widget.kind, defaultTrack, widget.id), true);
    setMapOptions(DEFAULT_MAP_OPTIONS);
  } : undefined;
  const undoReset = useCallback(() => {
    if (!resetUndo) return;
    onApply(resetUndo.widget, true);
    setMapOptions(resetUndo.mapOptions);
    setResetUndo(null);
  }, [resetUndo, onApply]);

  const effective = resolveWidgetCampaign(widget, { tracks: runtime.campaignTracks }, runtime.defaultRoleId);
  const missing = effective.trackIds.filter((id) => !runtime.tracks.some((track) => track.id === id));
  const campaignTrack = runtime.tracks.find(track => track.id === effective.trackIds[0]);
  const fixedAt = perfLog && campaignTrack && effective.time.mode !== 'follow'
    ? resolveWindow(effective, campaignTrack, runtime.tracks, 0)?.lastHour : undefined;
  const source = campaignTrack && definition?.data !== false && campaignTrack.roleId !== runtime.defaultRoleId ? campaignTrack.source : undefined;
  const mapRecording = campaignTrack ? `${campaignTrack.source.id}:${campaignTrack.source.revision}` : null;

  useEffect(() => {
    if (!mapRecording || prepared) return;
    const owners = new Map<string, object>();
    const stop = runtime.maps.onToggleFailure(widget.id, mapRecording, (setting, failure) => {
      let owner = owners.get(setting);
      if (!owner) owners.set(setting, owner = {});
      failures.put(owner, failure ? [{ error: failure }] : []);
    });
    return () => { stop(); for (const owner of owners.values()) failures.put(owner, []); };
  }, [runtime.maps, widget.id, failures, mapRecording, prepared]);

  const readingContextRef = useRef({ effective, campaignTrack, tracks: runtime.tracks });
  readingContextRef.current = { effective, campaignTrack, tracks: runtime.tracks };

  useEffect(() => {
    const content = contentRef.current;
    if (!content || prepared || typeof MutationObserver === 'undefined') return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let shown = false;
    const clear = () => {
      if (timer !== undefined) { clearTimeout(timer); timer = undefined; }
      if (shown) { shown = false; setReadingBadge(null); }
    };
    const readHeld = () => {
      if (content.querySelector('[data-held="behind"]')) return true;
      const playing = runtime.clock.getSnapshot?.().playing ?? false;
      return !playing && !!content.querySelector('[data-held="stale"]');
    };
    const label = () => {
      if (runtime.clock.getSnapshot?.().playing) return 'Catching up…';
      const { effective: current, campaignTrack: track, tracks } = readingContextRef.current;
      const target = track ? resolveWindow(current, track, tracks, runtime.clock.getHour?.() ?? 0)?.lastHour : null;
      return target != null ? `Reading ${formatDateTime(target)}…` : 'Reading…';
    };
    const check = () => {
      if (!readHeld()) { clear(); return; }
      if (shown) {
        const next = label();
        setReadingBadge(previous => previous === next ? previous : next);
        return;
      }
      if (timer !== undefined) return;
      timer = setTimeout(() => {
        timer = undefined;
        if (!readHeld()) { clear(); return; }
        shown = true;
        setReadingBadge(label());
      }, READING_BADGE_DELAY_MS);
    };
    check();
    const observer = new MutationObserver(check);
    observer.observe(content, { subtree: true, attributes: true, attributeFilter: ['data-held'], childList: true });
    const unsub = runtime.clock.subscribe?.(check);
    return () => { clear(); observer.disconnect(); unsub?.(); };
  }, [prepared, runtime.clock, widget.id]);

  const factionStyles = useFactionStylesKey(definition?.factionColours && campaignTrack ? campaignTrack.source.metadata.campaignKey : null);
  const settings = useMemo(() => settingsSignature(widget, factionStyles), [widget, factionStyles]);
  useEffect(() => {
    if (prepared) return;
    const frame = requestAnimationFrame(() => { if (element.current) element.current.dataset.drawn = settings; });
    return () => cancelAnimationFrame(frame);
  }, [settings, prepared]);

  const reportState = useCallback((owner: object, isState: boolean) => {
    if (isState) stateOwners.current.add(owner); else stateOwners.current.delete(owner);
    const next = stateOwners.current.size > 0;
    setHasBodyState(previous => previous === next ? previous : next);
  }, []);

  const announce = useCallback((key: string, text: string) => {
    if (!text || lastAnnounced.current === key) return;
    lastAnnounced.current = key;
    setAnnouncement(text);
  }, []);

  const clearAnnouncement = useCallback((key: string) => {
    if (lastAnnounced.current !== key) return;
    lastAnnounced.current = '';
    setAnnouncement(previous => previous === '' ? previous : '');
  }, []);

  useEffect(() => {
    if (!readingBadge) return;
    const key = `reading:${readingBadge}`;
    announce(key, readingBadge);
    return () => clearAnnouncement(key);
  }, [readingBadge, announce, clearAnnouncement]);

  const setDynamicViewSwitch = useCallback((owner: object, spec: DynamicViewSwitch | null) => {
    if (spec) switchOwners.current.set(owner, spec); else switchOwners.current.delete(owner);
    const latest = [...switchOwners.current.values()].at(-1) ?? null;
    setDynamicSwitch(latest);
  }, []);

  const setSkeleton = useCallback((owner: object, skeleton: WidgetSkeletonKind | (() => ReactNode) | null) => {
    if (skeleton) skeletonOwners.current.set(owner, skeleton); else skeletonOwners.current.delete(owner);
    const latest = [...skeletonOwners.current.values()].at(-1) ?? null;
    setCustomSkeleton(latest);
  }, []);

  const declaredSwitch = useMemo<DynamicViewSwitch | null>(() => resolveDeclaredViewSwitch(definition, widget, saveOption),
    [definition, widget, saveOption]);
  const activeSwitch = dynamicSwitch ?? declaredSwitch;
  const hasSubject = definition?.data !== false && definition?.countrySelection !== false;
  const showTime = definition?.data !== false && (definition?.timeModes.length ?? 0) > 0;

  const wording = useMemo(() => widgetWording(definition, widget.title), [definition, widget.title]);
  const registerOpenCountries = useCallback((open: (() => void) | undefined) => {
    openCountriesRef.current = open;
    setHasCountryOpener(Boolean(open));
  }, []);
  const registerOpenTime = useCallback((open: (() => void) | undefined) => {
    openTimeRef.current = open;
    setHasTimeOpener(Boolean(open));
  }, []);

  const frameValue = useMemo<WidgetFrameValue>(() => ({
    title: widget.title,
    subject: wording.subject,
    things: wording.things,
    timeMode: widget.time.mode,
    pinned: widget.time.mode === 'pin' || widget.time.mode === 'range',
    bounds: campaignTrack ? { firstHour: campaignTrack.branch.firstHour, lastHour: campaignTrack.branch.lastHour } : null,
    hasSubject,
    captionSlot,
    openCountries: hasSubject && hasCountryOpener ? () => openCountriesRef.current?.() : undefined,
    openTime: showTime && hasTimeOpener ? () => openTimeRef.current?.() : undefined,
    reportState,
    announce,
    clearAnnouncement,
    setDynamicViewSwitch,
    skeleton: customSkeleton ?? definition?.skeleton ?? 'generic',
    setSkeleton,
  }), [widget.title, wording.subject, wording.things, widget.time.mode, campaignTrack, hasSubject, hasCountryOpener, captionSlot, showTime, hasTimeOpener, definition?.skeleton, reportState, announce, clearAnnouncement, setDynamicViewSwitch, customSkeleton, setSkeleton]);

  const drawnHere = !prepared || definition?.preview !== false;
  const pipelineGen = usePipelineGeneration(prepared ? null : runtime.clock);

  return <article ref={attach} className={`${prepared ? 'dash-widget-prepared' : 'dash-widget'} kind-${widget.kind}`}
    {...(prepared ? {} : { 'data-widget': widget.id, 'data-settings': settings, 'data-fixed-hour': fixedAt === undefined ? undefined : fixedAt - campaignTrack!.axisOffset, 'aria-label': widget.title })}>
    <WidgetHeader
      widget={widget}
      definition={definition}
      effective={effective}
      campaignTrack={campaignTrack}
      runtime={runtime}
      template={template}
      editing={editing}
      active={active === true}
      activeSwitch={activeSwitch}
      hasBodyState={hasBodyState}
      readingBadge={widget.kind === 'map' ? readingBadge : null}
      summarySlotRef={slots.header}
      countrySlotRef={slots.country}
      filterSlotRef={slots.filter}
      headerSlotElement={headerSlot}
      source={source}
      onApply={applyWidget}
      onRemove={editing ? () => onRemove(widget.id) : undefined}
      onReset={reset}
      resetUndoVisible={!!resetUndo}
      onUndoReset={undoReset}
      mapOptions={widget.kind === 'map' ? mapOptions : undefined}
      onMapOptions={widget.kind === 'map' ? setMapOptions : undefined}
      onRegisterOpenCountries={registerOpenCountries}
      onRegisterOpenTime={registerOpenTime}
    />
    {missing.length > 0 && runtime.tracks.some((track) => effective.trackIds.includes(track.id)) && <div className="dash-inline-warning"><Icon name="alert" />{missing.map((id) => template.tracks.find((track) => track.id === id)?.label || 'Campaign').join(', ')} unavailable</div>}
    <WidgetFrameContext.Provider value={frameValue}>
      <HeldFailureNotices failures={failures} />
      <div className="dash-widget-caption-slot" ref={setCaptionSlot} hidden={hasBodyState || undefined} />
      <div ref={contentRef} className="dash-widget-content">
        <div className="dash-widget-scaled" style={{ zoom: (widget.contentScale ?? 100) / 100 }}>
          {!drawnHere ? null : active === null ? <WidgetState kind="loading" label={`Loading ${wording.subject}…`} /> : <HeldFailuresContext.Provider value={failures}><MeasuredWidget.Provider value={prepared ? null : widget.id}><FollowedWidgetKey.Provider value={widget.id}><FollowedWidgetKind.Provider value={widget.kind}><FollowedCard.Provider value={card}>
            <Body key={widget.kind === 'map' ? 0 : pipelineGen} widget={widget} runtime={bodyRuntime} active={active} mapOptions={mapOptions} headerSlot={headerSlot} countrySlot={countrySlot} filterSlot={filterSlot} setOption={saveOption} setOptions={saveOptions} />
          </FollowedCard.Provider></FollowedWidgetKind.Provider></FollowedWidgetKey.Provider></MeasuredWidget.Provider></HeldFailuresContext.Provider>}
        </div>
        {widget.kind !== 'map' && readingBadge && <div className="dash-reading-badge">{readingBadge}</div>}
      </div>
    </WidgetFrameContext.Provider>
    <div className="sr-only" aria-live="polite" aria-atomic="true">{announcement}</div>
  </article>;
}

export default memo(DashboardGrid);

commitZone(DashboardGrid, 'grid');
commitZone(WidgetFrame, 'widget frame');

function settingsSignature(widget: WidgetSpec, factionStyles = ''): string {
  const text = JSON.stringify(widget) + factionStyles;
  let hash = 2166136261;
  for (let index = 0; index < text.length; index++) hash = Math.imul(hash ^ text.charCodeAt(index), 16777619);
  return (hash >>> 0).toString(36);
}
