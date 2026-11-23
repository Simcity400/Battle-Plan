import { loadedWidgetRenderer, loadWidgetRenderer } from '../widgets/rendererLoaders';
import { Popover, PopoverHeading, usePopover } from '../components/Popover';
import { useId, useMemo } from 'react';
import { Icon } from '../components/Icons';
import { loadable } from '../components/loadable';
import { WidgetSettings } from './settings/WidgetSettings';
import type { MapOptions } from '../shell/api';
import type { DashboardTemplate, WidgetSpec } from './model';
import type { WidgetRuntime } from './widgets';

export default function WidgetSettingsMenu({ widget, template, runtime, onApply, onRemove, onReset, resetUndoVisible = false, onUndoReset, mapOptions, onMapOptions }: {
  widget: WidgetSpec; template: DashboardTemplate; runtime: WidgetRuntime;
  onApply: (widget: WidgetSpec, view?: boolean) => void; onRemove?: () => void; onReset?: () => void;
  resetUndoVisible?: boolean; onUndoReset?: () => void;
  mapOptions?: MapOptions; onMapOptions?: (options: MapOptions) => void;
}) {
  const Settings = useMemo(() => loadable(async () => { await loadWidgetRenderer(widget.kind); return WidgetSettings; },
    () => <p role="status">Loading settings…</p>), [widget.kind]);
  const id = useId();
  const menu = usePopover();
  const { trigger, open } = menu;
  const loaded = loadedWidgetRenderer(widget.kind) !== undefined;
  return <>
    <button ref={trigger} className="dash-icon-button" type="button" data-widget-settings aria-label={`Configure ${widget.title}`} data-tooltip="Widget settings"
      popoverTarget={id} aria-haspopup="dialog" aria-expanded={open}><Icon name="settings" /></button>
    <Popover menu={menu} id={id} className="widget-settings-menu" aria-label={`${widget.title} settings`}>
      <PopoverHeading title="Widget settings" menu={menu} onDelete={onRemove} onReset={onReset} />
      {resetUndoVisible && onUndoReset && <div className="widget-reset-undo" role="status">
        <span>Settings reset</span><span aria-hidden="true">·</span>
        <button type="button" className="text-button" onClick={onUndoReset}>Undo</button>
      </div>}
      {open && (loaded ? <WidgetSettings widget={widget} template={template} runtime={runtime} onChange={onApply} mapOptions={mapOptions} onMapOptions={onMapOptions} />
        : <Settings widget={widget} template={template} runtime={runtime} onChange={onApply} mapOptions={mapOptions} onMapOptions={onMapOptions} />)}
    </Popover>
  </>;
}
