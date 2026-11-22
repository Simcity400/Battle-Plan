import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { ReactNode, RefObject } from 'react';
import { Icon } from '../../components/Icons';
import { Popover, usePopover } from '../../components/Popover';
import FollowControl from './FollowControl';
import { TimeRegion } from '../../dashboard/widgetTime';
import type { Region } from '../../dashboard/regionTime';
import type { WidgetRuntime } from '../../dashboard/definitions';
export { default as FollowControl } from './FollowControl';
import './drill.css';

export default function DrillHost({ mode, open, title, day, following, canFollow, onClose, onFollow, onPause, onPin, anchor, returnFocus, main, children, time }: {
  mode: 'inline' | 'popout'; open: boolean; title: string; day: import('react').ReactNode; following: boolean; canFollow: boolean;
  onClose: () => void; onFollow: () => void; onPause?: () => void; onPin?: () => void;
  anchor: RefObject<HTMLElement | null>;
  returnFocus?: string;
  main: ReactNode; children: ReactNode;
  time: { runtime: Pick<WidgetRuntime, 'clock' | 'tracks'>; region: Region };
}) {
  const panel = usePopover('start');
  const panelRef = useRef(panel);
  panelRef.current = panel;
  const back = useRef<HTMLButtonElement>(null);
  const close = useRef({ onClose, returnFocus, open });
  close.current = { onClose, returnFocus, open };
  const popout = mode === 'popout';
  const [mounted, setMounted] = useState(open);
  useEffect(() => {
    if (!open) { setMounted(false); return; }
    if (mounted) return;
    const frame = requestAnimationFrame(() => setMounted(true));
    return () => cancelAnimationFrame(frame);
  }, [open, mounted]);
  const requestClose = () => {
    const closingPanel = popout ? panelRef.current.popup.current : null;
    const { onClose, returnFocus } = close.current;
    if (returnFocus && !close.current.open) return;
    if (returnFocus) close.current.open = false;
    onClose();
    if (!returnFocus) return;
    requestAnimationFrame(() => {
      if (!anchor.current?.isConnected) return;
      const focused = document.activeElement;
      if (focused && focused !== document.body && focused.isConnected !== false
        && !anchor.current?.contains(focused) && !closingPanel?.contains(focused)) return;
      anchor.current?.querySelector<HTMLElement>(returnFocus)?.focus({ preventScroll: true });
    });
  };
  useLayoutEffect(() => {
    if (!popout) return;
    const current = panelRef.current;
    const element = current.popup.current;
    if (!open) { if (element?.matches(':popover-open')) element.hidePopover(); return; }
    current.openAt(() => {
      const rect = anchor.current?.getBoundingClientRect();
      return rect ? { left: rect.right + 8, right: rect.right + 8, top: rect.top - 6, bottom: rect.top - 6 } : null;
    });
    const outside = (event: PointerEvent) => {
      const target = event.target as Node;
      if (element?.contains(target) || anchor.current?.contains(target)) return;
      if (target instanceof Element && target.closest('[popover]')) return;
      requestClose();
    };
    document.addEventListener('pointerdown', outside, true);
    return () => document.removeEventListener('pointerdown', outside, true);
  }, [popout, open, anchor]);
  useEffect(() => {
    if (!open) return;
    if (!popout) back.current?.focus({ preventScroll: true });
    const key = (event: KeyboardEvent) => {
      const active = document.activeElement;
      if (event.key === 'Escape' && (anchor.current?.contains(active) || (popout && panelRef.current.popup.current?.contains(active)))) requestClose();
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [popout, open, anchor]);

  const body = <TimeRegion runtime={time.runtime} region={time.region}>{children}</TimeRegion>;
  const header = <div className="drill-header">
    {popout ? null : <button ref={back} type="button" className="button small quiet" onClick={requestClose}><Icon name="chevron-left" />Back</button>}
    <strong>{title}</strong>
    <span className="drill-day">{day}</span>
    {canFollow && <FollowControl following={following} onFollow={onFollow} onPause={onPause} />}
    {onPin && <button type="button" className="icon-button small" aria-label="Pin as widget" data-tooltip="Pin as widget" onClick={onPin}><Icon name="pin" /></button>}
    {popout && <button type="button" className="icon-button small" aria-label="Close breakdown" onClick={requestClose}><Icon name="close" /></button>}
  </div>;

  if (popout) return <>
    {main}
    <Popover menu={panel} manual className="drill-popout" aria-label={title} onToggle={event => { if (event.newState === 'closed' && open) requestClose(); }}>
      {open && <>{header}<div className="drill-body">{mounted && body}</div></>}
    </Popover>
  </>;
  return <div className="drill-host" data-open={open || undefined}>
    <div className="drill-face drill-main" inert={open}>{main}</div>
    <div className="drill-face drill-view" inert={!open} role="region" aria-label={title}>
      {open && <>{header}<div className="drill-body">{mounted && body}</div></>}
    </div>
  </div>;
}
