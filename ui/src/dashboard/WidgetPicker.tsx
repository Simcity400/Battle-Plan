import { useId, useLayoutEffect, useRef, useState } from 'react';
import { Icon } from '../Icons';
import { widgetDefinitions } from './definitions';

export default function WidgetPicker({ disabled, onAdd, onOpen }: { disabled: boolean; onAdd: (kind: string) => void; onOpen: () => void }) {
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
      popup.current.style.left = `${Math.max(12, Math.min(anchor.right - width, document.documentElement.clientWidth - width - 12))}px`;
      popup.current.style.top = `${Math.max(12, Math.min(anchor.bottom + 8, document.documentElement.clientHeight - height - 12))}px`;
    };
    position();
    const observer = new ResizeObserver(position);
    if (popup.current) observer.observe(popup.current);
    window.addEventListener('resize', position);
    window.addEventListener('scroll', position, true);
    return () => { observer.disconnect(); window.removeEventListener('resize', position); window.removeEventListener('scroll', position, true); };
  }, [open]);

  return <>
    <button ref={trigger} type="button" className="button quiet header-add-widget" aria-label="Add widget" data-tooltip="Add widget" disabled={disabled}
      popoverTarget={id} aria-haspopup="dialog" aria-expanded={open} onClick={onOpen}><Icon name="plus" /></button>
    <div ref={popup} id={id} popover="auto" className="widget-picker" role="dialog" aria-label="Add a widget"
      onToggle={(event) => setOpen(event.newState === 'open')}>
      <div className="widget-picker-heading"><strong>Add a widget</strong><button type="button" className="dash-icon-button" aria-label="Close widget picker" popoverTarget={id} popoverTargetAction="hide"><Icon name="close" /></button></div>
      <div className="widget-picker-items">{widgetDefinitions.map((definition) => <button type="button" key={definition.kind} disabled={disabled} onClick={() => {
        popup.current?.hidePopover(); onAdd(definition.kind); trigger.current?.focus();
      }}><Icon name={definition.kind === 'map' ? 'globe' : 'grid'} /><span>{definition.kind === 'placeholder' ? 'Empty panel' : definition.title}</span><Icon name="plus" /></button>)}</div>
    </div>
  </>;
}
