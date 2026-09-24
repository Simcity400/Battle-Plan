import { Popover, PopoverHeading, usePopover } from '../components/Popover';
import { useId } from 'react';
import { Icon } from '../components/Icons';
import { WidgetSettings } from './controls';
import type { DashboardTemplate, WidgetSpec } from './model';
import type { WidgetRuntime } from './widgets';

export default function WidgetSettingsMenu({ widget, template, runtime, onApply, onRemove, onReset, diagnostics = [] }: {
  diagnostics?: string[]; widget: WidgetSpec; template: DashboardTemplate; runtime: WidgetRuntime;
  onApply: (widget: WidgetSpec, view?: boolean) => void; onRemove?: () => void; onReset?: () => void;
}) {
  const id = useId();
  const menu = usePopover();
  const { trigger, open } = menu;
  return <>
    <button ref={trigger} className="dash-icon-button" type="button" aria-label={`Configure ${widget.title}`} data-tooltip="Widget settings"
      popoverTarget={id} aria-haspopup="dialog" aria-expanded={open}><Icon name="settings" /></button>
    <Popover menu={menu} id={id} className="widget-settings-menu" aria-label={`${widget.title} settings`}>
      <PopoverHeading title="Widget settings" menu={menu} onDelete={onRemove} onReset={onReset} />
      {open && <WidgetSettings widget={widget} template={template} runtime={runtime} onChange={onApply} />}
      {open && diagnostics.length > 0 && <details className="widget-advanced"><summary>Data diagnostics ({diagnostics.length})</summary>
        <div>{diagnostics.map((message, index) => <p key={index}>{message}</p>)}</div></details>}
    </Popover>
  </>;
}
