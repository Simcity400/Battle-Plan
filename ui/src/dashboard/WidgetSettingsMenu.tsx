import { loadWidgetRenderer } from '../widgets/rendererLoaders';
import { Popover, PopoverHeading, usePopover } from '../components/Popover';
import { lazy, Suspense, useId, useMemo } from 'react';
import { Icon } from '../components/Icons';
import { WidgetSettings } from './settings/WidgetSettings';
import type { DashboardTemplate, WidgetSpec } from './model';
import type { WidgetRuntime } from './widgets';

export default function WidgetSettingsMenu({ widget, template, runtime, onApply, onRemove, onReset }: {
  widget: WidgetSpec; template: DashboardTemplate; runtime: WidgetRuntime;
  onApply: (widget: WidgetSpec, view?: boolean) => void; onRemove?: () => void; onReset?: () => void;
}) {
  const Settings = useMemo(() => lazy(async () => { await loadWidgetRenderer(widget.kind); return { default: WidgetSettings }; }), [widget.kind]);
  const id = useId();
  const menu = usePopover();
  const { trigger, open } = menu;
  return <>
    <button ref={trigger} className="dash-icon-button" type="button" aria-label={`Configure ${widget.title}`} data-tooltip="Widget settings"
      popoverTarget={id} aria-haspopup="dialog" aria-expanded={open}><Icon name="settings" /></button>
    <Popover menu={menu} id={id} className="widget-settings-menu" aria-label={`${widget.title} settings`}>
      <PopoverHeading title="Widget settings" menu={menu} onDelete={onRemove} onReset={onReset} />
      {open && <Suspense fallback={<p role="status">Loading settings?</p>}><Settings widget={widget} template={template} runtime={runtime} onChange={onApply} /></Suspense>}
    </Popover>
  </>;
}
