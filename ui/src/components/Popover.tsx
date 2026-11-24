import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { ComponentProps, ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from './Icons';
import './Popover.css';

export interface AnchorRect { left: number; right: number; top: number; bottom: number; aside?: number }

export function popoverPlacement(anchor: AnchorRect, width: number, height: number, viewport: { readonly width: number; readonly height: number }, align: 'start' | 'end'): { readonly left: number; readonly top: number } {
  const below = anchor.bottom + 6;
  const fits = below + height <= viewport.height - 12;
  const beside = !fits && anchor.aside !== undefined;
  const left = beside ? anchor.aside! + 6 : align === 'start' ? anchor.left : anchor.right - width;
  const top = fits ? below : beside ? anchor.top : anchor.top - height - 6;
  return {
    left: Math.max(12, Math.min(left, viewport.width - width - 12)),
    top: Math.max(12, Math.min(top, viewport.height - height - 12)),
  };
}

const closed = (state: { open: boolean }) => state.open ? { open: false } : state;

export function usePopover(align: 'start' | 'end' = 'end') {
  const trigger = useRef<HTMLButtonElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  const anchorRect = useRef<(() => AnchorRect | null) | null>(null);
  const [state, setState] = useState({ open: false });
  const { open } = state;
  const size = useRef<{ width: number; height: number } | null>(null);
  const place = useCallback((panel: HTMLElement, width: number, height: number) => {
    const anchor = anchorRect.current ? anchorRect.current() : trigger.current?.getBoundingClientRect();
    if (!anchor) return false;
    const viewport = document.documentElement;
    const at = popoverPlacement(anchor, width, height, { width: viewport.clientWidth, height: viewport.clientHeight }, align);
    panel.style.left = `${at.left}px`;
    panel.style.top = `${at.top}px`;
    panel.style.visibility = 'visible';
    return true;
  }, [align]);
  const opening = useCallback((panel: HTMLElement) => !!size.current && place(panel, size.current.width, size.current.height), [place]);
  useLayoutEffect(() => {
    if (!open) { anchorRect.current = null; return; }
    const position = () => {
      const panel = popup.current;
      if (!panel?.matches(':popover-open')) return;
      const { width, height } = panel.getBoundingClientRect();
      if (place(panel, width, height)) size.current = { width, height };
    };
    position();
    popup.current?.querySelector<HTMLElement>('[data-popover-autofocus]')?.focus({ preventScroll: true });
    const observer = new ResizeObserver(position);
    if (popup.current) observer.observe(popup.current);
    if (trigger.current) observer.observe(trigger.current);
    window.addEventListener('resize', position);
    window.addEventListener('scroll', position, true);
    return () => { observer.disconnect(); window.removeEventListener('resize', position); window.removeEventListener('scroll', position, true); };
  }, [state, place]);
  const opener = useRef<HTMLElement | null>(null);
  const close = useCallback(() => {
    const target = opener.current?.isConnected ? opener.current : trigger.current;
    popup.current?.hidePopover();
    target?.focus({ preventScroll: true });
  }, []);
  const openAt = useCallback((anchor: AnchorRect | (() => AnchorRect | null)) => {
    anchorRect.current = typeof anchor === 'function' ? anchor : () => anchor;
    const panel = popup.current;
    if (!panel) return;
    if (panel.matches(':popover-open')) setState({ open: true });
    else panel.showPopover();
  }, []);
  return useMemo(() => ({ trigger, popup, opener, open, setState, close, openAt, opening }), [open, close, openAt, opening]);
}

export const PopoverBody = memo(function PopoverBody({ open, children }: { open: boolean; children: () => ReactNode }) {
  const opened = useRef(false);
  opened.current ||= open;
  return opened.current ? children() : null;
}, (previous, next) => !previous.open && !next.open);

export function Popover({ menu, portal = true, manual = false, className = '', onToggle, children, ...props }: {
  menu: ReturnType<typeof usePopover>; portal?: boolean; manual?: boolean;
} & Omit<ComponentProps<'div'>, 'ref' | 'popover' | 'onBeforeToggle' | 'onKeyDown'>) {
  const { popup, setState } = menu;
  useEffect(() => () => {
    if (popup.current?.isConnected) popup.current.hidePopover();
    setState(closed);
  }, [popup, setState]);
  const panel = <div {...props} ref={menu.popup} popover={manual ? 'manual' : 'auto'} role={props.role ?? 'dialog'} className={`ui-popover ${className}`}
    onBeforeToggle={event => {
      if (event.target === event.currentTarget && event.newState === 'open' && !menu.opening(event.currentTarget)) event.currentTarget.style.visibility = 'hidden';
    }}
    onToggle={event => {
      if (event.target !== event.currentTarget) return;
      if (event.newState !== 'open') menu.opener.current = null;
      menu.setState(event.newState === 'open' ? { open: true } : closed);
      onToggle?.(event);
    }}
    onKeyDown={event => {
      if (event.key === 'Escape' && !event.defaultPrevented) {
        event.preventDefault(); event.stopPropagation(); menu.close();
      }
    }}>{children}</div>;
  return portal && typeof document !== 'undefined' ? createPortal(panel, document.body) : panel;
}

export function PopoverHeading({ title, menu, onDelete, onReset }: {
  title: string; menu: ReturnType<typeof usePopover>; onDelete?: () => void; onReset?: () => void;
}) {
  return <div className="ui-popover-heading"><strong>{title}</strong><div className="ui-popover-actions">
    {onReset && <button type="button" className="button small quiet ui-popover-reset" aria-label="Reset widget settings" onClick={onReset}><Icon name="refresh" />Reset</button>}
    {onDelete && <button type="button" className="icon-button ui-popover-action is-danger" aria-label="Delete widget" data-tooltip="Delete widget"
      onClick={() => { menu.close(); onDelete(); }}><Icon name="trash" /></button>}
    <button type="button" className="icon-button ui-popover-action" aria-label={`Close ${title.toLowerCase()}`} onClick={menu.close}><Icon name="close" /></button>
  </div></div>;
}
