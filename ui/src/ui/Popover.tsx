import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { ComponentProps } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from '../Icons';
import './Popover.css';

/** Keep a native popover hidden until its complete contents have been positioned. */
export function usePopover(align: 'start' | 'end' = 'end') {
  const trigger = useRef<HTMLButtonElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  const [state, setState] = useState({ open: false });
  const { open } = state;
  useLayoutEffect(() => {
    if (!open) return;
    const position = () => {
      const panel = popup.current;
      if (!trigger.current || !panel?.matches(':popover-open')) return;
      const anchor = trigger.current.getBoundingClientRect();
      const { width, height } = panel.getBoundingClientRect();
      const viewport = document.documentElement;
      const below = anchor.bottom + 6;
      const top = below + height <= viewport.clientHeight - 12 ? below : anchor.top - height - 6;
      panel.style.left = `${Math.max(12, Math.min(align === 'start' ? anchor.left : anchor.right - width, viewport.clientWidth - width - 12))}px`;
      panel.style.top = `${Math.max(12, Math.min(top, viewport.clientHeight - height - 12))}px`;
      panel.style.visibility = 'visible';
    };
    position();
    popup.current?.querySelector<HTMLElement>('[data-popover-autofocus]')?.focus({ preventScroll: true });
    const observer = new ResizeObserver(position);
    if (popup.current) observer.observe(popup.current);
    if (trigger.current) observer.observe(trigger.current);
    window.addEventListener('resize', position);
    window.addEventListener('scroll', position, true);
    return () => { observer.disconnect(); window.removeEventListener('resize', position); window.removeEventListener('scroll', position, true); };
  }, [state, align]);
  return {
    trigger, popup, open, setState,
    close: () => { popup.current?.hidePopover(); trigger.current?.focus({ preventScroll: true }); },
  };
}

/** Nested popovers stay inside their parent so native light-dismiss preserves it. */
export function Popover({ menu, portal = true, className = '', onToggle, children, ...props }: {
  menu: ReturnType<typeof usePopover>; portal?: boolean;
} & Omit<ComponentProps<'div'>, 'ref' | 'popover' | 'onBeforeToggle' | 'onKeyDown'>) {
  const { popup, setState } = menu;
  useEffect(() => () => {
    if (popup.current?.isConnected) popup.current.hidePopover();
    setState({ open: false });
  }, [popup, setState]);
  const panel = <div {...props} ref={menu.popup} popover="auto" role="dialog" className={`ui-popover ${className}`}
    onBeforeToggle={event => {
      if (event.target === event.currentTarget) event.currentTarget.style.visibility = 'hidden';
    }}
    onToggle={event => {
      // React delivers toggles from nested details and popovers to ancestors too.
      if (event.target !== event.currentTarget) return;
      menu.setState({ open: event.newState === 'open' });
      onToggle?.(event);
    }}
    onKeyDown={event => {
      if (event.key === 'Escape' && !event.defaultPrevented) {
        event.preventDefault(); event.stopPropagation(); menu.close();
      }
    }}>{children}</div>;
  return portal ? createPortal(panel, document.body) : panel;
}

export function PopoverHeading({ title, menu, onDelete, onReset }: {
  title: string; menu: ReturnType<typeof usePopover>; onDelete?: () => void; onReset?: () => void;
}) {
  return <div className="ui-popover-heading"><strong>{title}</strong><div className="ui-popover-actions">
    {onReset && <button type="button" className="ui-popover-action" aria-label="Reset widget settings" data-tooltip="Reset to defaults" onClick={onReset}><Icon name="refresh" /></button>}
    {onDelete && <button type="button" className="ui-popover-action" aria-label="Delete widget" data-tooltip="Delete widget"
      onClick={() => { menu.close(); onDelete(); }}><Icon name="trash" /></button>}
    <button type="button" className="ui-popover-action" aria-label={`Close ${title.toLowerCase()}`} onClick={menu.close}><Icon name="close" /></button>
  </div></div>;
}
