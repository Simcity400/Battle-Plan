import { memo, useCallback, useEffect, useRef, useState } from 'react';
import { Responsive, useContainerWidth } from 'react-grid-layout';
import type { Layout, ResponsiveLayouts } from 'react-grid-layout';
import 'react-grid-layout/css/styles.css';
import 'react-resizable/css/styles.css';
import { Icon } from '../Icons';
import { createWidget, GRID_COLUMNS } from './model';
import { widgetDefinition } from './definitions';
import type { Breakpoint, DashboardTemplate, GridLayouts, WidgetSpec } from './model';
import MapControls from './MapControls';
import { WidgetSettings } from './controls';
import { resolveWidgetCampaign } from './campaigns';
import type { MapOptions } from '../api';
import WidgetSettingsMenu from './WidgetSettingsMenu';
import { WidgetBody } from './widgets';
import type { WidgetRuntime } from './widgets';

interface Props {
  template: DashboardTemplate;
  runtime: WidgetRuntime;
  editing: boolean;
  onLayouts: (layouts: GridLayouts) => void;
  onApply: (widget: WidgetSpec) => void;
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
      breakpoints={{ lg: 1280, md: 850, sm: 520, xs: 0 }} cols={GRID_COLUMNS} rowHeight={26} margin={[10, 10]} containerPadding={[0, 0]}
      dragConfig={{ enabled: editing, handle: '.dash-drag-grip', cancel: 'input,select,textarea,a' }}
      resizeConfig={{ enabled: editing, handles: ['se'] }} onLayoutChange={changed}
      onDrag={runtime.maps.schedule} onDragStop={runtime.maps.schedule} onResize={runtime.maps.schedule} onResizeStop={runtime.maps.schedule}>
      {template.widgets.map((widget) => <div key={widget.id} className="dash-grid-cell">
        <WidgetFrame widget={widget} template={template} runtime={runtime} editing={editing} onApply={onApply} onRemove={onRemove} />
      </div>)}
    </Responsive>}
  </div>;
}

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
  const reset = widgetDefinition(widget.kind) ? () => {
    onApply(createWidget(widget.kind, template.tracks[0].id, widget.id));
    setMapOptions({ country: null, fog: true });
  } : undefined;
  const resolved = resolveWidgetCampaign(widget, { tracks: runtime.campaignTracks }, runtime.defaultRoleId);
  const mapTrack = runtime.tracks.find(track => track.id === resolved.trackIds[0]);
  const missing = widget.trackIds.filter((id) => !runtime.tracks.some((track) => track.id === id));
  return <article ref={element} className={`dash-widget kind-${widget.kind}`} aria-label={widget.title}>
    <header className="dash-widget-header">
      {editing && <span className="dash-drag-grip" data-tooltip="Drag to move this view" aria-hidden="true">⠿</span>}
      <Icon name={widget.kind === 'map' ? 'globe' : 'grid'} />
      <div className="dash-widget-title"><h2 data-tooltip={widget.title}>{widget.title}{widget.kind === 'national-focus' && <span ref={setHeaderSlot} />}</h2>
        {widget.kind !== 'placeholder' && <span>{widget.trackIds.map((id) => template.tracks.find((track) => track.id === id)?.label ?? 'Missing track').join(' + ')}</span>}</div>
      {widget.kind === 'map' ? <MapControls compact viewId={widget.id} source={mapTrack?.source ?? null} client={runtime.client} metadata={mapTrack?.source.metadata ?? null}
        branchId={mapTrack?.branch.id ?? null} onBranch={id => { if (mapTrack) runtime.onMapBranch(mapTrack.id, id); }} options={mapOptions} onOptions={setMapOptions} onRemove={editing ? () => onRemove(widget.id) : undefined} onReset={reset}>
        <details className="map-menu-section map-widget-options"><summary>Widget settings</summary>
          <WidgetSettings widget={widget} template={template} tracks={runtime.tracks} client={runtime.client} defaultRoleId={runtime.defaultRoleId}
            onChange={onApply} />
        </details>
      </MapControls> : <WidgetSettingsMenu diagnostics={editing ? diagnostics : []} widget={widget} template={template} runtime={runtime} onApply={onApply} onRemove={editing ? () => onRemove(widget.id) : undefined} onReset={reset} />}
    </header>
    {missing.length > 0 && runtime.tracks.some((track) => widget.trackIds.includes(track.id)) && <div className="dash-inline-warning">{missing.map((id) => template.tracks.find((track) => track.id === id)?.label ?? id).join(', ')} unavailable. Ready tracks remain visible.</div>}
    <div className="dash-widget-content"><WidgetBody widget={widget} runtime={runtime} active={active} mapOptions={mapOptions} headerSlot={headerSlot} onDiagnostics={setDiagnostics} /></div>
  </article>;
}

export default memo(DashboardGrid);
