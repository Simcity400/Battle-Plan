import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { ComponentProps, ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from './Icons';
import './Popover.css';

export interface AnchorRect { left: number; right: number; top: number; bottom: number }

/** The closed state; one already closed is kept, so its owner does not render again. */
const closed = (state: { open: boolean }) => state.open ? { open: false } : state;

/** Keep a native popover hidden until its complete contents have been positioned.
 * It sits under its trigger button, or at a point or beside a box given to `openAt`
 * (a right-click menu, a panel beside its widget). Opened again, its contents are kept, so it is placed at once with
 * the size it had and shown in the task that opens it: a click the moment it opens reaches it, never the page under it. */
export function usePopover(align: 'start' | 'end' = 'end') {
  const trigger = useRef<HTMLButtonElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  /** Where openAt placed the popover; re-read on every reposition so it follows scrolling. */
  const anchorRect = useRef<(() => AnchorRect | null) | null>(null);
  const [state, setState] = useState({ open: false });
  const { open } = state;
  /** The panel's size when it was last placed. */
  const size = useRef<{ width: number; height: number } | null>(null);
  /** Places `panel` of `width` by `height` at its anchor and shows it; false without an anchor. */
  const place = useCallback((panel: HTMLElement, width: number, height: number) => {
    const anchor = anchorRect.current ? anchorRect.current() : trigger.current?.getBoundingClientRect();
    if (!anchor) return false;
    const viewport = document.documentElement;
    const below = anchor.bottom + 6;
    const top = below + height <= viewport.clientHeight - 12 ? below : anchor.top - height - 6;
    panel.style.left = `${Math.max(12, Math.min(align === 'start' ? anchor.left : anchor.right - width, viewport.clientWidth - width - 12))}px`;
    panel.style.top = `${Math.max(12, Math.min(top, viewport.clientHeight - height - 12))}px`;
    panel.style.visibility = 'visible';
    return true;
  }, [align]);
  /** As the panel opens: placed and shown at once with the size it last had, if it has been placed before. */
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
  const close = useCallback(() => { popup.current?.hidePopover(); trigger.current?.focus({ preventScroll: true }); }, []);
  /** Opens at a point, or beside a box measured each time it is repositioned. */
  const openAt = useCallback((anchor: AnchorRect | (() => AnchorRect | null)) => {
    anchorRect.current = typeof anchor === 'function' ? anchor : () => anchor;
    const panel = popup.current;
    if (!panel) return;
    if (panel.matches(':popover-open')) setState({ open: true });
    else panel.showPopover();
  }, []);
  // One object while the open state holds, so owners can depend on it.
  return useMemo(() => ({ trigger, popup, open, setState, close, openAt, opening }), [open, close, openAt, opening]);
}

/** Pop-down contents built only once it has opened. Closed again, they keep their last look
 * (through the exit fade) and skip rendering until it reopens. */
export const PopoverBody = memo(function PopoverBody({ open, children }: { open: boolean; children: () => ReactNode }) {
  const opened = useRef(false);
  opened.current ||= open;
  return opened.current ? children() : null;
}, (previous, next) => !previous.open && !next.open);

/** Nested popovers stay inside their parent so native light-dismiss preserves it. */
/** `manual` popovers are not closed by clicks elsewhere; their owner decides. */
export function Popover({ menu, portal = true, manual = false, className = '', onToggle, children, ...props }: {
  menu: ReturnType<typeof usePopover>; portal?: boolean; manual?: boolean;
} & Omit<ComponentProps<'div'>, 'ref' | 'popover' | 'onBeforeToggle' | 'onKeyDown'>) {
  const { popup, setState } = menu;
  useEffect(() => () => {
    if (popup.current?.isConnected) popup.current.hidePopover();
    setState(closed);
  }, [popup, setState]);
  const panel = <div {...props} ref={menu.popup} popover={manual ? 'manual' : 'auto'} role="dialog" className={`ui-popover ${className}`}
    onBeforeToggle={event => {
      // Opening: shown at once where it was placed before; the first time hidden until positioned. Closing keeps it
      // visible for the exit fade.
      if (event.target === event.currentTarget && event.newState === 'open' && !menu.opening(event.currentTarget)) event.currentTarget.style.visibility = 'hidden';
    }}
    onToggle={event => {
      // React delivers toggles from nested details and popovers to ancestors too.
      if (event.target !== event.currentTarget) return;
      menu.setState(event.newState === 'open' ? { open: true } : closed);
      onToggle?.(event);
    }}
    onKeyDown={event => {
      if (event.key === 'Escape' && !event.defaultPrevented) {
        event.preventDefault(); event.stopPropagation(); menu.close();
      }
    }}>{children}</div>;
  // Server rendering (component tests) has no document to portal into.
  return portal && typeof document !== 'undefined' ? createPortal(panel, document.body) : panel;
}

export function PopoverHeading({ title, menu, onDelete, onReset }: {
  title: string; menu: ReturnType<typeof usePopover>; onDelete?: () => void; onReset?: () => void;
}) {
  return <div className="ui-popover-heading"><strong>{title}</strong><div className="ui-popover-actions">
    {onReset && <button type="button" className="icon-button ui-popover-action" aria-label="Reset widget settings" data-tooltip="Reset to defaults" onClick={onReset}><Icon name="refresh" /></button>}
    {onDelete && <button type="button" className="icon-button ui-popover-action is-danger" aria-label="Delete widget" data-tooltip="Delete widget"
      onClick={() => { menu.close(); onDelete(); }}><Icon name="trash" /></button>}
    <button type="button" className="icon-button ui-popover-action" aria-label={`Close ${title.toLowerCase()}`} onClick={menu.close}><Icon name="close" /></button>
  </div></div>;
}
