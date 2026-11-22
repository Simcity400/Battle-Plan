import { memo, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
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
import type { WidgetRuntime } from './definitions';
import { OptionDraft } from './optionDraft';
import type { OptionValue } from './options';
import WidgetSettingsMenu from './WidgetSettingsMenu';
import DefaultCountryChip from './CountryChip';
import WidgetCountryControl from './WidgetCountryControl';
import { HeaderActionsSlot } from '../charts/ZoomReset';
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
  const [card, setCard] = useState<HTMLElement | null>(null);
  const attach = useCallback((node: HTMLElement | null) => { element.current = node; setCard(node); }, []);
  const [active, setActive] = useState<boolean | null>(typeof IntersectionObserver === 'undefined' ? true : null);
  const [headerSlot, setHeaderSlot] = useState<HTMLSpanElement | null>(null);
  const [countrySlot, setCountrySlot] = useState<HTMLSpanElement | null>(null);
  const [actionsSlot, setActionsSlot] = useState<HTMLSpanElement | null>(null);
  const [failures] = useState(() => new HeldFailures());
  const [slots] = useState(() => {
    const noted: Record<'header' | 'country' | 'actions', HTMLSpanElement | null> = { header: null, country: null, actions: null };
    let shown = false;
    const slot = (key: keyof typeof noted, set: (element: HTMLSpanElement | null) => void) => (element: HTMLSpanElement | null) => {
      noted[key] = element;
      if (shown) set(element);
    };
    return { header: slot('header', setHeaderSlot), country: slot('country', setCountrySlot), actions: slot('actions', setActionsSlot),
      show() { shown = true; setHeaderSlot(noted.header); setCountrySlot(noted.country); setActionsSlot(noted.actions); } };
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
    if (options) onApply({ ...widget, options }, true);
  }, [definition, widget, onApply]);
  const saveOption = useCallback((key: string, value: OptionValue) => saveOptions({ [key]: value }), [saveOptions]);
  const defaultTrack = template.tracks[0]?.id;
  const reset = definition && defaultTrack !== undefined
    && !(widgetAtDefault(widget, defaultTrack) && mapOptions.country === null && mapOptions.fog === DEFAULT_MAP_OPTIONS.fog) ? () => {
    onApply(createWidget(widget.kind, defaultTrack, widget.id), true);
    setMapOptions(DEFAULT_MAP_OPTIONS);
  } : undefined;
  const effective = resolveWidgetCampaign(widget, { tracks: runtime.campaignTracks }, runtime.defaultRoleId);
  const missing = effective.trackIds.filter((id) => !runtime.tracks.some((track) => track.id === id));
  const HeaderControls = definition?.headerControls;
  const CountryChip = definition?.countryChip ?? DefaultCountryChip;
  const canFollow = !!definition?.history && definition.timeModes.includes('follow') && definition.timeModes.includes('campaign');
  const live = widget.time.mode === 'follow';
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
  const factionStyles = useFactionStylesKey(definition?.factionColours && campaignTrack ? campaignTrack.source.metadata.campaignKey : null);
  const settings = useMemo(() => settingsSignature(widget, factionStyles), [widget, factionStyles]);
  useEffect(() => {
    if (prepared) return;
    const frame = requestAnimationFrame(() => { if (element.current) element.current.dataset.drawn = settings; });
    return () => cancelAnimationFrame(frame);
  }, [settings, prepared]);
  const drawnHere = !prepared || definition?.preview !== false;
  return <article ref={attach} className={`${prepared ? 'dash-widget-prepared' : 'dash-widget'} kind-${widget.kind}`}
    {...(prepared ? {} : { 'data-widget': widget.id, 'data-settings': settings, 'data-fixed-hour': fixedAt === undefined ? undefined : fixedAt - campaignTrack!.axisOffset, 'aria-label': widget.title })}>
    <header className="dash-widget-header">
      {editing && <span className="dash-drag-grip" data-tooltip="Drag to move" aria-hidden="true"><Icon name="grip" /></span>}
      <span className="dash-widget-icon" aria-hidden="true" data-widget-icon={widget.id} style={icon.colour ? { color: icon.colour } : undefined}
        onContextMenu={event => {
          if (!definition) return;
          event.preventDefault();
          icons.openMenu(widget.kind, event.currentTarget, definition.title, event.clientX, event.clientY);
        }}><StyledIcon icon={icon.icon} fallback={definition?.icon ?? 'grid'} /></span>
      <div className="dash-widget-title"><h2>{widget.title}<span className="dash-widget-summary" ref={slots.header} /></h2>
        {source && <span className="dash-widget-subtitle">{recordingDisplayName(source.path)}</span>}</div>
      {definition?.data !== false && definition?.countrySelection !== false && <WidgetCountryControl widget={widget} template={template} runtime={runtime} onApply={onApply}>
        <CountryChip widget={widget} runtime={runtime} active={active === true} onSlot={slots.country} /></WidgetCountryControl>}
      <div className="dash-widget-actions">
      <span className="dash-widget-actions-slot" ref={slots.actions} />
      {canFollow && <button type="button" role="switch" aria-checked={live} className="live-toggle" aria-label={`${widget.title}: follow playback`}
        data-tooltip={live ? 'Show whole campaign' : 'Follow playback'}
        onClick={() => onApply({ ...widget, time: { mode: live ? 'campaign' : 'follow' } }, true)}>
        <span className="live-toggle-track" aria-hidden="true"><span className="live-toggle-knob" /></span><span className="live-toggle-label">Live</span>
      </button>}
      {HeaderControls
        ? <HeaderControls widget={widget} runtime={runtime} template={template} editing={editing} onApply={onApply} onRemove={() => onRemove(widget.id)} onReset={reset}
          mapOptions={mapOptions} onMapOptions={setMapOptions} />
        : <WidgetSettingsMenu widget={widget} template={template} runtime={runtime} onApply={onApply} onRemove={editing ? () => onRemove(widget.id) : undefined} onReset={reset} />}
      </div>
    </header>
    {missing.length > 0 && runtime.tracks.some((track) => effective.trackIds.includes(track.id)) && <div className="dash-inline-warning"><Icon name="alert" />{missing.map((id) => template.tracks.find((track) => track.id === id)?.label || 'Campaign').join(', ')} unavailable</div>}
    <HeldFailureNotices failures={failures} />
    <div className="dash-widget-content"><div className="dash-widget-scaled" style={{ zoom: (widget.contentScale ?? 100) / 100 }}>
      {!drawnHere ? null : active === null ? <WidgetState kind="loading" label={`Loading ${widget.title}…`} /> : <HeaderActionsSlot.Provider value={actionsSlot}>
        <HeldFailuresContext.Provider value={failures}><MeasuredWidget.Provider value={prepared ? null : widget.id}><FollowedWidgetKey.Provider value={widget.id}><FollowedCard.Provider value={card}>
          <Body widget={widget} runtime={bodyRuntime} active={active} mapOptions={mapOptions} headerSlot={headerSlot} countrySlot={countrySlot} setOption={saveOption} setOptions={saveOptions} />
        </FollowedCard.Provider></FollowedWidgetKey.Provider></MeasuredWidget.Provider></HeldFailuresContext.Provider>
      </HeaderActionsSlot.Provider>}
    </div></div>
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
