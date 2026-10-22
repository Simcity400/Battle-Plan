import { memo, useCallback, useContext, useEffect, useRef, useState } from 'react';
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
import { createWidget, GRID_COLUMNS, widgetAtDefault } from './model';
import { resolveWidgetCampaign } from './campaigns';
import { recordingDisplayName, useRecordingNames } from './recordingNames';
import type { Breakpoint, DashboardTemplate, GridLayouts, WidgetSpec } from './model';
import { flipFog } from './mapPicker';
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

interface Props {
  template: DashboardTemplate;
  runtime: WidgetRuntime;
  editing: boolean;
  onLayouts: (layouts: GridLayouts) => void;
  onApply: (widget: WidgetSpec, view?: boolean) => void;
  onRemove: (id: string) => void;
}

function DashboardGrid({ template, runtime, editing, onLayouts, onApply, onRemove }: Props) {
  const { width, containerRef, mounted } = useContainerWidth({ initialWidth: 1200 });
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
        <WidgetFrame widget={widget} template={template} runtime={runtime} editing={editing} onApply={onApply} onRemove={onRemove} />
      </div>)}
    </Responsive>}
  </div>;
}

/** Milliseconds of an animation frame spent mounting widget bodies, leaving the rest of a
 * 120 Hz frame to layout and paint. */
const MOUNT_BUDGET = 4;
/** Bodies waiting to mount, with where their widget last was. */
const waiting = new Map<() => void, IntersectionObserverEntry>();
let mountFrame = 0;

/** Queues a widget body to mount, or updates its place in the queue. Bodies near the viewport
 * mount first, then in reading order, as many each frame as the budget allows (at least one),
 * so opening a page never mounts all its widgets in one long frame. */
function mountInTurn(mount: () => void, entry: IntersectionObserverEntry): () => void {
  waiting.set(mount, entry);
  if (!mountFrame) mountFrame = requestAnimationFrame(mountWaiting);
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
  mountFrame = waiting.size ? requestAnimationFrame(mountWaiting) : 0;
}

/** Frame-only changes (editing, layout moves) leave the body alone. */
const Body = memo(WidgetBody);

/** Common chrome for every widget: header, drag grip, settings and the scaled body. */
function WidgetFrame({ widget, template, runtime, editing, onApply, onRemove }: Omit<Props, 'onLayouts'> & { widget: WidgetSpec }) {
  const names = useRecordingNames();
  const element = useRef<HTMLElement>(null);
  // Null until the body mounts in its turn, then whether the widget is near the viewport.
  // Server rendering has no observer and renders the body at once.
  const [active, setActive] = useState<boolean | null>(typeof IntersectionObserver === 'undefined' ? true : null);
  const [headerSlot, setHeaderSlot] = useState<HTMLSpanElement | null>(null);
  const [countrySlot, setCountrySlot] = useState<HTMLSpanElement | null>(null);
  const [actionsSlot, setActionsSlot] = useState<HTMLSpanElement | null>(null);
  const [failures] = useState(() => new HeldFailures());
  // The header elements the body portals into are only noted until the body mounts, so
  // attaching them never renders the frame again; the body's first render has them all.
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
  useEffect(() => {
    const target = element.current;
    if (!target) return;
    let mounted = false, visible = false, cancel = () => {};
    const mount = () => { mounted = true; slots.show(); setActive(visible); };
    const observer = new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting;
      if (mounted) setActive(visible); else cancel = mountInTurn(mount, entry);
    }, { root: target.closest('.dash-grid-scroll'), rootMargin: '240px 0px' });
    observer.observe(target);
    return () => { observer.disconnect(); cancel(); };
  }, [slots]);
  const [mapOptions, setMapOptions] = useState<MapOptions>({ country: null, fog: true });
  // The native picker's fog click flips this widget's switch, as its control does.
  useEffect(() => runtime.maps.onFog(widget.id, () => setMapOptions(flipFog)), [runtime.maps, widget.id]);
  const definition = widgetDefinition(widget.kind);
  const icons = useContext(WidgetIconsContext);
  const icon = icons.style(widget.kind);
  const optionDraft = useRef(new OptionDraft(widget.options));
  optionDraft.current.sync(widget.options);
  // An option set from inside the widget saves at once, as the settings panel does.
  const saveOptions = useCallback((values: Readonly<Record<string, OptionValue>>) => {
    const options = optionDraft.current.updateAll(definition?.options, values);
    if (options) onApply({ ...widget, options }, true);
  }, [definition, widget, onApply]);
  const saveOption = useCallback((key: string, value: OptionValue) => saveOptions({ [key]: value }), [saveOptions]);
  // A layout without tracks has no default widget to return to.
  const defaultTrack = template.tracks[0]?.id;
  const reset = definition && defaultTrack !== undefined
    && !(widgetAtDefault(widget, defaultTrack) && mapOptions.country === null && mapOptions.fog) ? () => {
    onApply(createWidget(widget.kind, defaultTrack, widget.id), true);
    setMapOptions({ country: null, fog: true });
  } : undefined;
  const effective = resolveWidgetCampaign(widget, { tracks: runtime.campaignTracks }, runtime.defaultRoleId);
  const missing = effective.trackIds.filter((id) => !runtime.tracks.some((track) => track.id === id));
  const HeaderControls = definition?.headerControls;
  const CountryChip = definition?.countryChip ?? DefaultCountryChip;
  const canFollow = !!definition?.history && definition.timeModes.includes('follow') && definition.timeModes.includes('campaign');
  const live = widget.time.mode === 'follow';
  const campaignTrack = runtime.tracks.find(track => track.id === effective.trackIds[0]);
  const source = campaignTrack && definition?.data !== false && campaignTrack.roleId !== runtime.defaultRoleId ? campaignTrack.source : undefined;
  return <article ref={element} className={`dash-widget kind-${widget.kind}`} aria-label={widget.title}>
    <header className="dash-widget-header">
      {editing && <span className="dash-drag-grip" data-tooltip="Drag to move" aria-hidden="true"><Icon name="grip" /></span>}
      <span className="dash-widget-icon" aria-hidden="true" data-widget-icon={widget.id} style={icon.colour ? { color: icon.colour } : undefined}
        onContextMenu={event => {
          if (!definition) return;
          event.preventDefault();
          icons.openMenu(widget.kind, event.currentTarget, definition.title, event.clientX, event.clientY);
        }}><StyledIcon icon={icon.icon} fallback={definition?.icon ?? 'grid'} /></span>
      <div className="dash-widget-title"><h2>{widget.title}<span className="dash-widget-summary" ref={slots.header} /></h2>
        {source && <span className="dash-widget-subtitle">{recordingDisplayName(source.path, names)}</span>}</div>
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
      {active === null ? <WidgetState kind="loading" label={`Loading ${widget.title}…`} /> : <HeaderActionsSlot.Provider value={actionsSlot}>
        <HeldFailuresContext.Provider value={failures}>
          <Body widget={widget} runtime={runtime} active={active} mapOptions={mapOptions} headerSlot={headerSlot} countrySlot={countrySlot} setOption={saveOption} setOptions={saveOptions} />
        </HeldFailuresContext.Provider>
      </HeaderActionsSlot.Provider>}
    </div></div>
  </article>;
}

export default memo(DashboardGrid);
