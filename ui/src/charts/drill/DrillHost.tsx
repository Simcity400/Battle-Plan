import { useEffect, useLayoutEffect, useRef } from 'react';
import type { ReactNode, RefObject } from 'react';
import { Icon } from '../../components/Icons';
import { Popover, usePopover } from '../../components/Popover';
import './drill.css';

/** Hosts a widget's main view and one open breakdown. In the widget (the default)
 * the view slides across to the breakdown and ← Back returns; beside it, the
 * breakdown opens in a panel next to the widget that closes with ×, Esc or a click
 * elsewhere. Both share the header: the day shown, Follow playback and Pin as widget. */
export default function DrillHost({ mode, open, title, day, following, canFollow, onClose, onFollow, onPin, anchor, main, children }: {
  mode: 'inline' | 'popout'; open: boolean; title: string; day: string; following: boolean; canFollow: boolean;
  onClose: () => void; onFollow: () => void; onPin?: () => void;
  /** The widget body the pop-out panel sits beside. */
  anchor: RefObject<HTMLElement | null>;
  main: ReactNode; children: ReactNode;
}) {
  const panel = usePopover('start');
  // usePopover returns a new object each render; effects use this ref so they run only
  // when the drill state changes, not on every render.
  const panelRef = useRef(panel);
  panelRef.current = panel;
  const back = useRef<HTMLButtonElement>(null);
  const close = useRef(onClose);
  close.current = onClose;
  const popout = mode === 'popout';
  useLayoutEffect(() => {
    if (!popout) return;
    const current = panelRef.current;
    const element = current.popup.current;
    if (!open) { if (element?.matches(':popover-open')) element.hidePopover(); return; }
    // The panel sits beside the widget, measured each time it is placed so it follows scrolling.
    current.openAt(() => {
      const rect = anchor.current?.getBoundingClientRect();
      return rect ? { left: rect.right + 8, right: rect.right + 8, top: rect.top - 6, bottom: rect.top - 6 } : null;
    });
    // A manual panel ignores clicks on its own widget (picking a day, the chart menu)
    // and closes on a click anywhere else.
    const outside = (event: PointerEvent) => {
      const target = event.target as Node;
      if (element?.contains(target) || anchor.current?.contains(target)) return;
      if (target instanceof Element && target.closest('[popover]')) return;
      close.current();
    };
    document.addEventListener('pointerdown', outside, true);
    return () => document.removeEventListener('pointerdown', outside, true);
  }, [popout, open, anchor]);
  // In the widget, focus moves to Back when the breakdown opens (the main view becomes
  // inert). Esc anywhere in the widget, or in the pop-out, closes the breakdown.
  useEffect(() => {
    if (!open) return;
    if (!popout) back.current?.focus({ preventScroll: true });
    const key = (event: KeyboardEvent) => {
      const active = document.activeElement;
      if (event.key === 'Escape' && (anchor.current?.contains(active) || (popout && panelRef.current.popup.current?.contains(active)))) close.current();
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [popout, open, anchor]);

  const header = <div className="drill-header">
    {popout ? null : <button ref={back} type="button" className="button small quiet" onClick={onClose}><Icon name="chevron-left" />Back</button>}
    <strong>{title}</strong>
    <span className="drill-day">{day}</span>
    {canFollow && (following
      ? <span className="pill drill-following" data-tooltip="The breakdown moves with playback">Following playback</span>
      : <button type="button" className="button small" onClick={onFollow}>Follow playback</button>)}
    {onPin && <button type="button" className="icon-button small" aria-label="Pin as widget" data-tooltip="Pin as widget" onClick={onPin}><Icon name="pin" /></button>}
    {popout && <button type="button" className="icon-button small" aria-label="Close breakdown" onClick={onClose}><Icon name="close" /></button>}
  </div>;

  if (popout) return <>
    {main}
    <Popover menu={panel} manual className="drill-popout" aria-label={title} onToggle={event => { if (event.newState === 'closed' && open) onClose(); }}>
      {open && <>{header}<div className="drill-body">{children}</div></>}
    </Popover>
  </>;
  return <div className="drill-host" data-open={open || undefined}>
    <div className="drill-face drill-main" inert={open}>{main}</div>
    <div className="drill-face drill-view" inert={!open} role="region" aria-label={title}>
      {open && <>{header}<div className="drill-body">{children}</div></>}
    </div>
  </div>;
}
