import { useId, useLayoutEffect, useRef, useState } from 'react';
import { Icon } from '../Icons';
import { WidgetSettings } from './controls';
import type { DashboardTemplate, WidgetSpec } from './model';
import type { WidgetRuntime } from './widgets';

export default function WidgetSettingsMenu({ widget, template, runtime, onApply, onRemove }: {
  widget: WidgetSpec; template: DashboardTemplate; runtime: WidgetRuntime;
  onApply: (widget: WidgetSpec) => void; onRemove: () => void;
}) {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  useLayoutEffect(() => {
    if (!open) return;
    const position = () => {
      if (!trigger.current || !popup.current) return;
      const anchor = trigger.current.getBoundingClientRect();
      const { width, height } = popup.current.getBoundingClientRect();
      const viewport = document.documentElement;
      popup.current.style.left = `${Math.max(12, Math.min(anchor.right - width, viewport.clientWidth - width - 12))}px`;
      popup.current.style.top = `${Math.max(12, Math.min(anchor.bottom + 6, viewport.clientHeight - height - 12))}px`;
    };
    position();
    const observer = new ResizeObserver(position);
    if (popup.current) observer.observe(popup.current);
    window.addEventListener('resize', position);
    window.addEventListener('scroll', position, true);
    return () => { observer.disconnect(); window.removeEventListener('resize', position); window.removeEventListener('scroll', position, true); };
  }, [open]);
  const close = () => { popup.current?.hidePopover(); trigger.current?.focus(); };
  return <>
    <button ref={trigger} className="dash-icon-button" type="button" aria-label={`Configure ${widget.title}`} data-tooltip="Widget settings"
      popoverTarget={id} aria-haspopup="dialog" aria-expanded={open}><Icon name="settings" /></button>
    <div ref={popup} id={id} popover="auto" className="widget-settings-menu" role="dialog" aria-label={`${widget.title} settings`}
      onToggle={(event) => setOpen(event.newState === 'open')}>
      <div className="widget-settings-heading"><strong>Widget settings</strong><button type="button" className="dash-icon-button" aria-label="Close widget settings" onClick={close}><Icon name="close" /></button></div>
      {open && <WidgetSettings widget={widget} template={template} tracks={runtime.tracks} clock={runtime.clock} defaultRoleId={runtime.defaultRoleId}
        onLive={(time) => onApply({ ...widget, time })}
        onApply={(next) => { onApply(next); close(); }} onRemove={() => { close(); onRemove(); }} />}
    </div>
  </>;
}
