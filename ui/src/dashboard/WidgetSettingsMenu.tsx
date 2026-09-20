import { Popover, usePopover } from '../ui/Popover';
import { useId } from 'react';
import { Icon } from '../Icons';
import { WidgetSettings } from './controls';
import type { DashboardTemplate, WidgetSpec } from './model';
import type { WidgetRuntime } from './widgets';

export default function WidgetSettingsMenu({ widget, template, runtime, onApply, onRemove, diagnostics = [] }: {
  diagnostics?: string[]; widget: WidgetSpec; template: DashboardTemplate; runtime: WidgetRuntime;
  onApply: (widget: WidgetSpec) => void; onRemove: () => void;
}) {
  const id = useId();
  const menu = usePopover();
  const { trigger, open } = menu;
  const close = menu.close;
  return <>
    <button ref={trigger} className="dash-icon-button" type="button" aria-label={`Configure ${widget.title}`} data-tooltip="Widget settings"
      popoverTarget={id} aria-haspopup="dialog" aria-expanded={open}><Icon name="settings" /></button>
    <Popover menu={menu} id={id} className="widget-settings-menu" aria-label={`${widget.title} settings`}>
      <div className="widget-settings-heading"><strong>Widget settings</strong><button type="button" className="dash-icon-button" aria-label="Close widget settings" onClick={close}><Icon name="close" /></button></div>
      {open && <WidgetSettings widget={widget} template={template} tracks={runtime.tracks} client={runtime.client} defaultRoleId={runtime.defaultRoleId}
        onLive={(time) => onApply({ ...widget, time })}
        onApply={(next) => { onApply(next); close(); }} onRemove={() => { close(); onRemove(); }} />}
      {open && diagnostics.length > 0 && <details className="widget-advanced"><summary>Data diagnostics ({diagnostics.length})</summary>
        <div>{diagnostics.map((message, index) => <p key={index}>{message}</p>)}</div></details>}
    </Popover>
  </>;
}
