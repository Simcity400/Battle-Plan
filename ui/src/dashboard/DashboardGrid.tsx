import { memo, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { flushSync } from 'react-dom';
import { Responsive, useContainerWidth } from 'react-grid-layout';
import type { Layout, ResponsiveLayouts } from 'react-grid-layout';
import 'react-grid-layout/css/styles.css';
import 'react-resizable/css/styles.css';
import { Icon } from '../components/Icons';
import { StyledIcon } from '../shell/appearance/StyledIcon';
import { WidgetIconsContext } from '../shell/appearance/WidgetIconMenu';
import WidgetState from '../components/WidgetState';
import type { MapOptions } from '../shell/api';
import { createWidget, GRID_COLUMNS, resolveWindow, widgetAtDefault } from './model';
import { resolveWidgetCampaign } from './campaigns';
import { useFactionStylesKey } from './campaignSettings';
import { recordingDisplayName } from './recordingNames';
import type { Breakpoint, DashboardTemplate, GridLayouts, WidgetSpec } from './model';
import { DEFAULT_MAP_OPTIONS, flipFog } from './mapPicker';
import { widgetDefinition } from './definitions';
import type { WidgetRuntime, WidgetSkeletonKind } from './definitions';
import { OptionDraft } from './optionDraft';
import { resolveOptions } from './options';
import type { OptionValue } from './options';
import WidgetSettingsMenu from './WidgetSettingsMenu';
import DefaultCountryChip from './CountryChip';
import WidgetCountryControl from './WidgetCountryControl';
import WidgetTimeChip, { timeChipSummary } from './WidgetTimeChip';
import WidgetViewSwitch from './WidgetViewSwitch';
import { FULL_HEADER_DROP, headerDropStage, WidgetFrameContext, widgetWording } from './widgetFrame';
import type { DynamicViewSwitch, HeaderDropState, WidgetFrameValue } from './widgetFrame';
import { formatDateTime } from './calendar';
import { WidgetBody } from './widgets';
import { HeldFailures, HeldFailuresContext } from './heldFailures';
import HeldFailureNotices from './HeldFailureNotices';
import { MeasuredWidget } from './widgetPerf';
import { FollowedCard, FollowedWidgetKey } from './follow/store.ts';
import { WidgetFrames } from './widgetClock';
import { perfLog } from '../shell/perf';
import { DASHBOARD_SCROLL, NEAR_SCREEN } from './follow/useFollow.ts';
import { preloadAllWidgetRenderers, preloadWidgetRenderers } from '../widgets/rendererLoaders';
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
  const headerRef = useRef<HTMLElement>(null);
  const titleRef = useRef<HTMLHeadingElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const [card, setCard] = useState<HTMLElement | null>(null);
  const attach = useCallback((node: HTMLElement | null) => { element.current = node; setCard(node); }, []);
  const [active, setActive] = useState<boolean | null>(typeof IntersectionObserver === 'undefined' ? true : null);
  const [headerSlot, setHeaderSlot] = useState<HTMLSpanElement | null>(null);
  const [countrySlot, setCountrySlot] = useState<HTMLSpanElement | null>(null);
  const [captionSlot, setCaptionSlot] = useState<HTMLDivElement | null>(null);
  const [headerWidth, setHeaderWidth] = useState<number | null>(null);
  const [titleClipped, setTitleClipped] = useState(false);
  const [summaryText, setSummaryText] = useState('');
  const [subjectText, setSubjectText] = useState('');
  const [drop, setDrop] = useState<HeaderDropState>(FULL_HEADER_DROP);
  const measuredHeader = useRef({
    pillKey: '', pill: 0,
    viewFullKey: '', viewFull: 0,
    viewMenuKey: '', viewMenu: 0,
    subjectFullKey: '', subjectFull: 0,
    subjectCompact: 0,
    timeFullKey: '', timeFull: 0,
    timeCompact: 0,
  });
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
  const [failures] = useState(() => new HeldFailures());
  const [slots] = useState(() => {
    const noted: Record<'header' | 'country', HTMLSpanElement | null> = { header: null, country: null };
    let shown = false;
    const slot = (key: keyof typeof noted, set: (element: HTMLSpanElement | null) => void) => (element: HTMLSpanElement | null) => {
      noted[key] = element;
      if (shown) set(element);
    };
    return { header: slot('header', setHeaderSlot), country: slot('country', setCountrySlot),
      show() { shown = true; setHeaderSlot(noted.header); setCountrySlot(noted.country); } };
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

  useEffect(() => {
    const header = headerRef.current;
    if (!header || typeof ResizeObserver === 'undefined') return;
    const measure = () => {
      const next = Math.floor(header.clientWidth);
      setHeaderWidth(previous => previous === next ? previous : next);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(header);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (!headerSlot || typeof MutationObserver === 'undefined') return;
    const inspect = () => {
      const text = (headerSlot.textContent ?? '').trim();
      setSummaryText(previous => previous === text ? previous : text);
    };
    inspect();
    const observer = new MutationObserver(inspect);
    observer.observe(headerSlot, { childList: true, subtree: true, characterData: true });
    return () => observer.disconnect();
  }, [headerSlot]);

  useEffect(() => {
    const header = headerRef.current;
    if (!header || typeof MutationObserver === 'undefined') return;
    const read = () => {
      const el = header.querySelector<HTMLElement>('.dash-widget-country-button, .dash-widget-controls > .dash-widget-country');
      const text = (el?.textContent ?? '').trim();
      setSubjectText(previous => previous === text ? previous : text);
    };
    read();
    const controls = header.querySelector('.dash-widget-controls');
    if (!controls) return;
    const observer = new MutationObserver(read);
    observer.observe(controls, { childList: true, subtree: true, characterData: true });
    return () => observer.disconnect();
  }, [widget.id]);

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
  const icons = useContext(WidgetIconsContext);
  const icon = icons.style(widget.kind);
  const frames = useMemo(() => new WidgetFrames(runtime.clock, widget.id), [runtime.clock, widget.id]);
  useLayoutEffect(() => { frames.setShown(active === true && !prepared); }, [frames, active, prepared]);
  const bodyRuntime = useMemo(() => ({ ...runtime, clock: frames.clock }), [runtime, frames]);
  const optionDraft = useRef(new OptionDraft(widget.options));
  optionDraft.current.sync(widget.options);
  const saveOptions = useCallback((values: Readonly<Record<string, OptionValue>>) => {
    const options = optionDraft.current.updateAll(definition?.options, values);
    if (options) applyWidget({ ...widget, options }, true);
  }, [definition, widget, applyWidget]);
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
  const CountryChip = definition?.countryChip ?? DefaultCountryChip;
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

  useEffect(() => {
    const content = contentRef.current;
    if (!content || prepared || typeof MutationObserver === 'undefined') return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const clear = () => {
      if (timer !== undefined) { clearTimeout(timer); timer = undefined; }
      setReadingBadge(null);
    };
    const check = () => {
      const playing = runtime.clock.getSnapshot?.().playing ?? false;
      const held = !playing && !!content.querySelector('[data-held="stale"]');
      if (!held) { clear(); return; }
      if (timer !== undefined) return;
      timer = setTimeout(() => {
        timer = undefined;
        if (runtime.clock.getSnapshot?.().playing) { setReadingBadge(null); return; }
        if (!content.querySelector('[data-held="stale"]')) { setReadingBadge(null); return; }
        const target = campaignTrack ? resolveWindow(effective, campaignTrack, runtime.tracks, runtime.clock.getHour?.() ?? 0)?.lastHour : null;
        setReadingBadge(target != null ? `Reading ${formatDateTime(target)}…` : 'Reading…');
      }, READING_BADGE_DELAY_MS);
    };
    check();
    const observer = new MutationObserver(check);
    observer.observe(content, { subtree: true, attributes: true, attributeFilter: ['data-held'], childList: true });
    const unsub = runtime.clock.subscribe?.(check);
    return () => { clear(); observer.disconnect(); unsub?.(); };
  }, [prepared, runtime.clock, runtime.tracks, campaignTrack, effective]);

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

  const declaredSwitch = useMemo<DynamicViewSwitch | null>(() => {
    const spec = definition?.viewSwitch;
    if (!spec || !definition?.options) return null;
    const resolved = resolveOptions(definition.options, widget.options);
    if (spec.when && !spec.when(resolved)) return null;
    const field = definition.options.find(item => item.key === spec.option);
    if (!field || field.kind !== 'choice') return null;
    if (field.visibleWhen && !field.visibleWhen(resolved, widget.countries, widget.time.mode)) return null;
    const options = spec.choices ?? field.choices;
    if (options.length <= 1) return null;
    return {
      label: spec.label ?? `${widget.title} ${field.label.toLowerCase()}`,
      value: String(resolved[spec.option] ?? field.default),
      options,
      onChange: value => saveOption(spec.option, value),
    };
  }, [definition, widget.options, widget.countries, widget.time.mode, widget.title, saveOption]);

  const activeSwitch = dynamicSwitch ?? declaredSwitch;
  const viewMenuOnly = (activeSwitch?.options.length ?? 0) >= 4;
  const viewKey = activeSwitch ? `${activeSwitch.options.map(item => item.label).join('|')}:${activeSwitch.value}` : '';
  const hasSubject = definition?.data !== false && definition?.countrySelection !== false;
  const showTime = definition?.data !== false && (definition?.timeModes.length ?? 0) > 0;
  const timeInfo = showTime ? timeChipSummary(widget, effective, campaignTrack, runtime.tracks, runtime.clock.getHour?.() ?? 0) : null;
  const timeKey = timeInfo?.label ?? '';
  const hasSummary = summaryText.length > 0 && !hasBodyState;
  const effectiveDrop: HeaderDropState = viewMenuOnly && !drop.collapseView
    ? { ...drop, collapseView: true }
    : drop;

  useLayoutEffect(() => {
    const header = headerRef.current;
    if (!header || header.clientWidth <= 0) return;
    const available = header.clientWidth - 16;
    const widthOf = (el: HTMLElement | null) => el ? Math.ceil(el.getBoundingClientRect().width) : 0;
    const iconEl = header.querySelector<HTMLElement>('.dash-widget-icon');
    const gripEl = header.querySelector<HTMLElement>('.dash-drag-grip');
    const settingsEl = header.querySelector<HTMLElement>('[data-widget-settings]');
    const summaryEl = header.querySelector<HTMLElement>('.dash-widget-summary');
    const subjectEl = header.querySelector<HTMLElement>('.dash-widget-country-button, .dash-widget-controls > .dash-widget-country');
    const viewEl = header.querySelector<HTMLElement>('.dash-view-switch');
    const timeEl = header.querySelector<HTMLElement>('.dash-time-chip');
    const heading = titleRef.current;
    const titleFull = heading ? Math.ceil(heading.scrollWidth) : 0;
    const fixed = widthOf(iconEl) + widthOf(settingsEl) + widthOf(gripEl);
    const cache = measuredHeader.current;

    if (hasSummary && !effectiveDrop.dropPill && summaryEl) {
      cache.pill = Math.ceil(summaryEl.scrollWidth);
      cache.pillKey = summaryText;
    }
    if (activeSwitch && viewEl) {
      if (!effectiveDrop.collapseView) {
        cache.viewFull = widthOf(viewEl);
        cache.viewFullKey = viewKey;
      } else {
        cache.viewMenu = widthOf(viewEl);
        cache.viewMenuKey = viewKey;
      }
    }
    if (hasSubject && subjectEl) {
      if (!effectiveDrop.compactSubject) {
        cache.subjectFull = widthOf(subjectEl);
        cache.subjectFullKey = subjectText;
      } else {
        cache.subjectCompact = widthOf(subjectEl);
      }
    }
    if (showTime && timeEl) {
      if (!effectiveDrop.compactTime) {
        cache.timeFull = widthOf(timeEl);
        cache.timeFullKey = timeKey;
      } else {
        cache.timeCompact = widthOf(timeEl);
      }
    }

    const needsFullMeasure = (hasSummary && cache.pillKey !== summaryText && effectiveDrop.dropPill)
      || (!!activeSwitch && !viewMenuOnly && cache.viewFullKey !== viewKey && effectiveDrop.collapseView)
      || (hasSubject && cache.subjectFullKey !== subjectText && effectiveDrop.compactSubject)
      || (showTime && cache.timeFullKey !== timeKey && effectiveDrop.compactTime);

    const next: HeaderDropState = needsFullMeasure
      ? { stage: 0, dropPill: false, collapseView: viewMenuOnly, compactSubject: false, compactTime: false }
      : headerDropStage(available, {
          fixed,
          titleFull,
          hasSummary,
          pill: cache.pillKey === summaryText ? cache.pill : 0,
          hasViewSwitch: !!activeSwitch,
          viewMenuOnly,
          viewFull: cache.viewFullKey === viewKey ? cache.viewFull : 0,
          viewMenu: cache.viewMenuKey === viewKey && cache.viewMenu > 0 ? cache.viewMenu : 84,
          hasSubject,
          subjectFull: cache.subjectFullKey === subjectText ? cache.subjectFull : 0,
          subjectCompact: cache.subjectCompact > 0 ? cache.subjectCompact : 42,
          hasTime: showTime,
          timeFull: cache.timeFullKey === timeKey ? cache.timeFull : 0,
          timeCompact: cache.timeCompact > 0 ? cache.timeCompact : 24,
          editing,
        });

    setDrop(previous => previous.stage === next.stage && previous.dropPill === next.dropPill
      && previous.collapseView === next.collapseView && previous.compactSubject === next.compactSubject
      && previous.compactTime === next.compactTime ? previous : next);
  }, [headerWidth, widget.title, summaryText, hasSummary, subjectText, hasSubject, activeSwitch, viewKey, viewMenuOnly, showTime, timeKey, editing,
    effectiveDrop.dropPill, effectiveDrop.collapseView, effectiveDrop.compactSubject, effectiveDrop.compactTime]);

  const wording = useMemo(() => widgetWording(definition, widget.title), [definition, widget.title]);
  const registerOpenCountries = useCallback((open: (() => void) | undefined) => { openCountriesRef.current = open; }, []);
  const registerOpenTime = useCallback((open: (() => void) | undefined) => { openTimeRef.current = open; }, []);

  const frameValue = useMemo<WidgetFrameValue>(() => ({
    title: widget.title,
    subject: wording.subject,
    things: wording.things,
    timeMode: widget.time.mode,
    pinned: widget.time.mode === 'pin' || widget.time.mode === 'range',
    bounds: campaignTrack ? { firstHour: campaignTrack.branch.firstHour, lastHour: campaignTrack.branch.lastHour } : null,
    hasSubject,
    captionSlot,
    openCountries: hasSubject ? () => openCountriesRef.current?.() : undefined,
    openTime: showTime && (definition?.timeModes.length ?? 0) > 1 ? () => openTimeRef.current?.() : undefined,
    reportState,
    announce,
    setDynamicViewSwitch,
    skeleton: customSkeleton ?? definition?.skeleton ?? 'generic',
    setSkeleton,
  }), [widget.title, wording.subject, wording.things, widget.time.mode, campaignTrack, hasSubject, captionSlot, showTime, definition?.timeModes.length, definition?.skeleton, reportState, announce, setDynamicViewSwitch, customSkeleton, setSkeleton]);

  useLayoutEffect(() => {
    const heading = titleRef.current;
    if (!heading) return;
    const clipped = heading.scrollWidth > heading.clientWidth;
    setTitleClipped(previous => previous === clipped ? previous : clipped);
  }, [headerWidth, widget.title, effectiveDrop.stage]);

  const drawnHere = !prepared || definition?.preview !== false;

  return <article ref={attach} className={`${prepared ? 'dash-widget-prepared' : 'dash-widget'} kind-${widget.kind}`}
    {...(prepared ? {} : { 'data-widget': widget.id, 'data-settings': settings, 'data-fixed-hour': fixedAt === undefined ? undefined : fixedAt - campaignTrack!.axisOffset, 'aria-label': widget.title })}>
    <header ref={headerRef} className="dash-widget-header" data-header-stage={effectiveDrop.stage}>
      {editing && <span className="dash-drag-grip" data-tooltip="Drag to move" aria-hidden="true"><Icon name="grip" /></span>}
      <span className="dash-widget-icon" aria-hidden="true" data-widget-icon={widget.id} style={icon.colour ? { color: icon.colour } : undefined}
        onContextMenu={event => {
          if (!definition) return;
          event.preventDefault();
          icons.openMenu(widget.kind, event.currentTarget, definition.title, event.clientX, event.clientY);
        }}><StyledIcon icon={icon.icon} fallback={definition?.icon ?? 'grid'} /></span>
      <div className="dash-widget-title">
        <h2 ref={titleRef} data-tooltip-overflow={widget.title} tabIndex={titleClipped ? 0 : undefined}>{widget.title}</h2>
        {source && <span className="dash-widget-subtitle">{recordingDisplayName(source.path)}</span>}
      </div>
      <span className="dash-widget-summary" ref={slots.header} hidden={effectiveDrop.dropPill || hasBodyState || undefined} />
      <div className="dash-widget-controls">
        {hasSubject && <WidgetCountryControl widget={widget} template={template} runtime={runtime} onApply={applyWidget}
          compact={effectiveDrop.compactSubject} onRegisterOpen={registerOpenCountries}>
          <CountryChip widget={widget} runtime={runtime} active={active === true} compact={effectiveDrop.compactSubject} onSlot={slots.country} />
        </WidgetCountryControl>}
        {activeSwitch && <WidgetViewSwitch label={activeSwitch.label} value={activeSwitch.value} options={activeSwitch.options}
          compact={effectiveDrop.collapseView} onChange={activeSwitch.onChange} />}
        {showTime && <WidgetTimeChip widget={widget} effective={effective} reference={campaignTrack} runtime={runtime}
          modes={definition!.timeModes} compact={effectiveDrop.compactTime}
          onChange={time => applyWidget({ ...widget, time }, true)} onRegisterOpen={registerOpenTime} />}
        <WidgetSettingsMenu widget={widget} template={template} runtime={runtime} onApply={applyWidget}
          onRemove={editing ? () => onRemove(widget.id) : undefined} onReset={reset}
          resetUndoVisible={!!resetUndo} onUndoReset={undoReset}
          mapOptions={widget.kind === 'map' ? mapOptions : undefined}
          onMapOptions={widget.kind === 'map' ? setMapOptions : undefined} />
      </div>
    </header>
    {missing.length > 0 && runtime.tracks.some((track) => effective.trackIds.includes(track.id)) && <div className="dash-inline-warning"><Icon name="alert" />{missing.map((id) => template.tracks.find((track) => track.id === id)?.label || 'Campaign').join(', ')} unavailable</div>}
    <WidgetFrameContext.Provider value={frameValue}>
      <HeldFailureNotices failures={failures} />
      <div className="dash-widget-caption-slot" ref={setCaptionSlot} hidden={hasBodyState || undefined} />
      <div ref={contentRef} className="dash-widget-content">
        <div className="dash-widget-scaled" style={{ zoom: (widget.contentScale ?? 100) / 100 }}>
          {!drawnHere ? null : active === null ? <WidgetState kind="loading" label={`Loading ${wording.subject}…`} /> : <HeldFailuresContext.Provider value={failures}><MeasuredWidget.Provider value={prepared ? null : widget.id}><FollowedWidgetKey.Provider value={widget.id}><FollowedCard.Provider value={card}>
            <Body widget={widget} runtime={bodyRuntime} active={active} mapOptions={mapOptions} headerSlot={headerSlot} countrySlot={countrySlot} setOption={saveOption} setOptions={saveOptions} />
          </FollowedCard.Provider></FollowedWidgetKey.Provider></MeasuredWidget.Provider></HeldFailuresContext.Provider>}
        </div>
        {readingBadge && <div className="dash-reading-badge" role="status">{readingBadge}</div>}
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
