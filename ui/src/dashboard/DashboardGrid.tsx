import { memo, useCallback, useEffect, useRef, useState } from 'react';
import { Responsive, useContainerWidth } from 'react-grid-layout';
import type { Layout, ResponsiveLayouts } from 'react-grid-layout';
import 'react-grid-layout/css/styles.css';
import 'react-resizable/css/styles.css';
import { Icon } from '../components/Icons';
import type { MapOptions } from '../shell/api';
import { createWidget, GRID_COLUMNS } from './model';
import { resolveWidgetCampaign } from './campaigns';
import { recordingName } from '../shell/api';
import type { Breakpoint, DashboardTemplate, GridLayouts, WidgetSpec } from './model';
import { widgetDefinition } from './definitions';
import type { WidgetRuntime } from './definitions';
import { OptionDraft } from './optionDraft';
import type { OptionValue } from './options';
import WidgetSettingsMenu from './WidgetSettingsMenu';
import CountryChip from './CountryChip';
import { WidgetBody } from './widgets';

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

/** Common chrome for every widget: header, drag grip, settings and the scaled body. */
function WidgetFrame({ widget, template, runtime, editing, onApply, onRemove }: Omit<Props, 'onLayouts'> & { widget: WidgetSpec }) {
  const element = useRef<HTMLElement>(null);
  const [active, setActive] = useState(true);
  const [diagnostics, setDiagnostics] = useState<string[]>([]);
  const [headerSlot, setHeaderSlot] = useState<HTMLSpanElement | null>(null);
  useEffect(() => {
    const target = element.current;
    if (!target) return;
    const observer = new IntersectionObserver(([entry]) => setActive(entry.isIntersecting),
      { root: target.closest('.dash-grid-scroll'), rootMargin: '240px 0px' });
    observer.observe(target);
    return () => observer.disconnect();
  }, []);
  const [mapOptions, setMapOptions] = useState<MapOptions>({ country: null, fog: true });
  const definition = widgetDefinition(widget.kind);
  const optionDraft = useRef(new OptionDraft(widget.options));
  optionDraft.current.sync(widget.options);
  // An option set from inside the widget saves at once, as the settings panel does.
  const saveOption = useCallback((key: string, value: OptionValue) => {
    const options = optionDraft.current.update(definition?.options, key, value);
    if (options) onApply({ ...widget, options }, true);
  }, [definition, widget, onApply]);
  const reset = definition ? () => {
    onApply(createWidget(widget.kind, template.tracks[0].id, widget.id), true);
    setMapOptions({ country: null, fog: true });
  } : undefined;
  const missing = widget.trackIds.filter((id) => !runtime.tracks.some((track) => track.id === id));
  const HeaderControls = definition?.headerControls;
  const canFollow = !!definition?.history && definition.timeModes.includes('follow') && definition.timeModes.includes('campaign');
  const live = widget.time.mode === 'follow';
  // Name the campaign only when several are open; one campaign needs no label.
  const source = runtime.tracks.length > 1 && definition?.data !== false
    ? runtime.tracks.find(track => track.id === resolveWidgetCampaign(widget, { tracks: runtime.campaignTracks }, runtime.defaultRoleId).trackIds[0])?.source : undefined;
  return <article ref={element} className={`dash-widget kind-${widget.kind}`} aria-label={widget.title}>
    <header className="dash-widget-header">
      {editing && <span className="dash-drag-grip" data-tooltip="Drag to move" aria-hidden="true"><Icon name="grip" /></span>}
      <span className="dash-widget-icon" aria-hidden="true"><Icon name={definition?.icon ?? 'grid'} /></span>
      <div className="dash-widget-title"><h2>{widget.title}<span className="dash-widget-summary" ref={setHeaderSlot} /></h2>
        {source && <span className="dash-widget-subtitle">{recordingName(source.path).replace(/\.(?:battleplan|hoi4replay)$/i, '')}</span>}</div>
      {definition?.data !== false && <CountryChip widget={widget} runtime={runtime} />}
      <div className="dash-widget-actions">
      {canFollow && <button type="button" role="switch" aria-checked={live} className="live-toggle" aria-label={`${widget.title}: follow playback`}
        data-tooltip={live ? 'Following the timeline. Switch off to see the whole campaign.' : 'Showing the whole campaign. Switch on to follow the timeline.'}
        onClick={() => onApply({ ...widget, time: { mode: live ? 'campaign' : 'follow' } }, true)}>
        <span className="live-toggle-track" aria-hidden="true"><span className="live-toggle-knob" /></span><span className="live-toggle-label">Live</span>
      </button>}
      {HeaderControls
        ? <HeaderControls widget={widget} runtime={runtime} template={template} editing={editing} onApply={onApply} onRemove={() => onRemove(widget.id)} onReset={reset}
          mapOptions={mapOptions} onMapOptions={setMapOptions} diagnostics={editing ? diagnostics : []} />
        : <WidgetSettingsMenu diagnostics={editing ? diagnostics : []} widget={widget} template={template} runtime={runtime} onApply={onApply} onRemove={editing ? () => onRemove(widget.id) : undefined} onReset={reset} />}
      </div>
    </header>
    {missing.length > 0 && runtime.tracks.some((track) => widget.trackIds.includes(track.id)) && <div className="dash-inline-warning"><Icon name="alert" />{missing.map((id) => template.tracks.find((track) => track.id === id)?.label ?? id).join(', ')} unavailable</div>}
    <div className="dash-widget-content"><div className="dash-widget-scaled" style={{ zoom: (widget.contentScale ?? 100) / 100 }}>
      <WidgetBody widget={widget} runtime={runtime} active={active} mapOptions={mapOptions} headerSlot={headerSlot} onDiagnostics={setDiagnostics} setOption={saveOption} />
    </div></div>
  </article>;
}

export default memo(DashboardGrid);
