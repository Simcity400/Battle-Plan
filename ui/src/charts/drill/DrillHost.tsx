import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { ReactNode, RefObject } from 'react';
import { Icon } from '../../components/Icons';
import { Popover, usePopover } from '../../components/Popover';
import FollowControl from './FollowControl';
import { TimeRegion } from '../../dashboard/widgetTime';
import GateState from '../../dashboard/GateState';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { DrillControl } from './useDrill';
import { DrillScope, ShownHour, useDrillShown } from '../../dashboard/widgetPerf.ts';
export { default as FollowControl } from './FollowControl';
import './drill.css';

export default function DrillHost({ mode, drill, runtime, subject = true, title, day, canFollow, onClose: closed, onPin, pinRefused = null, anchor, returnFocus, root, main, children }: {
  mode: 'inline' | 'popout'; drill: Pick<DrillControl, 'saved' | 'open' | 'following' | 'region' | 'gate' | 'hour' | 'outside' | 'follow' | 'pause' | 'close'>; runtime: Pick<WidgetRuntime, 'clock' | 'tracks'>; subject?: boolean;
  title: string; day: ReactNode; canFollow: boolean; onClose?: () => void; onPin?: () => void; pinRefused?: string | null;
  anchor: RefObject<HTMLElement | null>;
  returnFocus?: string;
  root?: (element: HTMLDivElement | null) => void;
  main: ReactNode; children: ReactNode;
}) {
  const saved = drill.saved;
  const open = drill.open && subject;
  const onClose = () => { drill.close(); closed?.(); };
  const panel = usePopover('start');
  const panelRef = useRef(panel);
  panelRef.current = panel;
  const back = useRef<HTMLButtonElement>(null);
  const close = useRef({ onClose, returnFocus, open });
  close.current = { onClose, returnFocus, open };
  const popout = mode === 'popout' && !drill.saved;
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
    if (!open) {
      if (element?.matches(':popover-open')) element.hidePopover();
      return;
    }
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

  const shut = drill.gate.stage.stage !== 'open' && drill.gate.stage.stage !== 'unbound';
  useDrillShown(open && !popout);
  const body = <DrillScope value={popout ? 'quiet' : 'owns'}><TimeRegion runtime={runtime} region={drill.region}>{shut ? <><ShownHour clock={runtime.clock} at={null} /><GateState gate={drill.gate} loading="Loading…" /></> : children}</TimeRegion></DrillScope>;
  const header = <div className="drill-header">
    {popout || saved ? null : <button ref={back} type="button" className="button small quiet" onClick={requestClose}><Icon name="chevron-left" />Back</button>}
    {saved ? null : <strong>{title}</strong>}
    <span className="drill-day">{shut ? '' : day}</span>
    {canFollow && <FollowControl following={drill.following} onFollow={drill.follow} onPause={drill.pause} />}
    {onPin && !shut && <button type="button" className="icon-button small" aria-label="Pin as widget" data-tooltip="Pin as widget" onClick={onPin}><Icon name="pin" /></button>}
    {!onPin && pinRefused && !shut && <button type="button" className="icon-button small" aria-disabled="true" aria-label={`Can’t pin: ${pinRefused}`} data-tooltip={pinRefused}><Icon name="pin" /></button>}
    {popout && !saved && <button type="button" className="icon-button small" aria-label="Close breakdown" onClick={requestClose}><Icon name="close" /></button>}
  </div>;

  if (saved) return <div className="drill-saved" role="region" aria-label={title}>{header}<div className="drill-body" ref={root}>{body}</div></div>;

  if (popout) return <>
    {main}
    <Popover menu={panel} manual className="drill-popout" aria-label={title} onToggle={event => { if (event.newState === 'closed' && open) requestClose(); }}>
      {open && <>{header}<div className="drill-body" ref={root}>{mounted && body}</div></>}
    </Popover>
  </>;
  return <div className="drill-host" data-open={open || undefined}>
    <div className="drill-face drill-main" inert={open}>{main}</div>
    <div className="drill-face drill-view" inert={!open} role="region" aria-label={title}>
      {open && <>{header}<div className="drill-body" ref={root}>{mounted && body}</div></>}
    </div>
  </div>;
}
