import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { PointerEvent as ReactPointerEvent, ReactNode, RefObject } from 'react';
import { Icon } from '../../components/Icons';
import { Popover, usePopover } from '../../components/Popover';
import { BAR, centerPanel, clampPanel } from '../../components/floatingPlacement.ts';
import FollowControl from './FollowControl';
import { TimeRegion } from '../../dashboard/widgetTime';
import GateState from '../../dashboard/GateState';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { DrillControl } from './useDrill';
import { DrillScope, ShownHour, useDrillShown } from '../../dashboard/widgetPerf.ts';
export { default as FollowControl } from './FollowControl';
import './drill.css';

export default function DrillHost({ mode, drill, runtime, subject = true, title, day, canFollow, onClose: closed, onPin, pinRefused = null, anchor, returnFocus, root, windowPosition = null, windowSettled = true, focusTrigger, onPositionChange, main, children }: {
  mode: 'inline' | 'popout' | 'window'; drill: Pick<DrillControl, 'saved' | 'open' | 'following' | 'region' | 'gate' | 'hour' | 'outside' | 'follow' | 'pause' | 'close'>; runtime: Pick<WidgetRuntime, 'clock' | 'tracks'>; subject?: boolean;
  title: string; day: ReactNode; canFollow: boolean; onClose?: () => void; onPin?: () => void; pinRefused?: string | null;
  anchor: RefObject<HTMLElement | null>;
  returnFocus?: string;
  root?: (element: HTMLDivElement | null) => void;
  windowPosition?: { x: number; y: number } | null;
  windowSettled?: boolean;
  focusTrigger?: string | number;
  onPositionChange?: (position: { x: number; y: number }) => void;
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
  const windowed = mode === 'window' && !drill.saved;
  const floating = popout || windowed;
  const [mounted, setMounted] = useState(open);
  const wasWindowOpen = useRef(false);
  const restoreQueued = useRef(false);
  const pinnedPos = useRef<{ x: number; y: number } | null>(null);
  const dragged = useRef(false);
  const windowMeta = useRef({ windowPosition, windowSettled, onPositionChange });
  windowMeta.current = { windowPosition, windowSettled, onPositionChange };
  useEffect(() => {
    if (!open) { setMounted(false); return; }
    if (mounted) return;
    const frame = requestAnimationFrame(() => setMounted(true));
    return () => cancelAnimationFrame(frame);
  }, [open, mounted]);
  const scheduleReturnFocus = (closingPanel: HTMLElement | null) => {
    const { returnFocus } = close.current;
    if (!returnFocus) return;
    restoreQueued.current = true;
    requestAnimationFrame(() => {
      if (!anchor.current?.isConnected) return;
      const focused = document.activeElement;
      if (focused && focused !== document.body && focused.isConnected !== false
        && !anchor.current?.contains(focused) && !closingPanel?.contains(focused)) return;
      anchor.current?.querySelector<HTMLElement>(returnFocus)?.focus({ preventScroll: true });
    });
  };
  const requestClose = () => {
    const closingPanel = floating ? panelRef.current.popup.current : null;
    const { onClose, returnFocus } = close.current;
    if (returnFocus && !close.current.open) return;
    if (returnFocus) close.current.open = false;
    onClose();
    scheduleReturnFocus(closingPanel);
  };
  const placeWindow = (element: HTMLElement, shown = Infinity) => {
    const view = { width: window.innerWidth, height: window.innerHeight };
    const pinned = pinnedPos.current;
    if (!pinned) {
      element.style.left = '0px';
      element.style.top = '0px';
    }
    const width = element.offsetWidth;
    const height = element.offsetHeight;
    const at = pinned
      ? clampPanel(width, height, pinned.x, pinned.y, view, shown)
      : centerPanel(width, height, view);
    element.style.left = `${at.x}px`;
    element.style.top = `${at.y}px`;
    element.style.visibility = 'visible';
    if (!pinned && !dragged.current && windowMeta.current.windowSettled && width > 0 && height > 0) {
      pinnedPos.current = at;
      windowMeta.current.onPositionChange?.(at);
    }
  };
  useLayoutEffect(() => {
    if (!popout) return;
    const current = panelRef.current;
    const element = current.popup.current;
    if (!open) { if (element?.matches(':popover-open')) element.hidePopover(); return; }
    restoreQueued.current = false;
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
  useLayoutEffect(() => {
    if (!windowed) return;
    const element = panelRef.current.popup.current;
    if (!open) {
      const hadOpen = wasWindowOpen.current;
      wasWindowOpen.current = false;
      if (hadOpen && !restoreQueued.current) scheduleReturnFocus(element);
      if (element?.matches?.(':popover-open')) element.hidePopover();
      pinnedPos.current = null;
      dragged.current = false;
      return;
    }
    const opening = !wasWindowOpen.current;
    wasWindowOpen.current = true;
    restoreQueued.current = false;
    if (opening) {
      dragged.current = false;
      pinnedPos.current = windowMeta.current.windowPosition ?? null;
    }
    if (!element) return;
    if (!element.matches?.(':popover-open')) element.showPopover?.();
    placeWindow(element);
    if (opening) element.focus?.({ preventScroll: true });
    const reposition = () => placeWindow(element);
    const observer = typeof ResizeObserver === 'function' ? new ResizeObserver(reposition) : null;
    observer?.observe(element);
    window.addEventListener('resize', reposition);
    return () => {
      observer?.disconnect();
      window.removeEventListener('resize', reposition);
    };
  }, [windowed, open]);
  useLayoutEffect(() => {
    if (!windowed || !open || !windowSettled) return;
    const element = panelRef.current.popup.current;
    if (element) placeWindow(element);
  }, [windowed, open, windowSettled]);
  useEffect(() => {
    if (!windowed || !open || focusTrigger === undefined) return;
    const element = panelRef.current.popup.current;
    if (element && !element.contains(document.activeElement)) {
      element.focus?.({ preventScroll: true });
    }
  }, [windowed, open, focusTrigger]);
  useEffect(() => {
    if (!open) return;
    if (!floating) back.current?.focus({ preventScroll: true });
    const key = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.defaultPrevented) return;
      const active = document.activeElement;
      if (windowed) {
        if (panelRef.current.popup.current?.contains(active)) requestClose();
      } else if (anchor.current?.contains(active) || (popout && panelRef.current.popup.current?.contains(active))) {
        requestClose();
      }
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [floating, popout, windowed, open, anchor]);
  const startDrag = (event: ReactPointerEvent<HTMLElement>) => {
    const node = panelRef.current.popup.current;
    if (!node || event.button !== 0 || (event.target as Element).closest('button, a, input, select')) return;
    event.preventDefault();
    if (!node.contains(document.activeElement)) node.focus({ preventScroll: true });
    const handle = event.currentTarget;
    const dx = event.clientX - node.offsetLeft;
    const dy = event.clientY - node.offsetTop;
    dragged.current = true;
    handle.setPointerCapture(event.pointerId);
    node.classList.add('is-dragging');
    const move = (next: PointerEvent) => {
      const at = clampPanel(node.offsetWidth, node.offsetHeight, next.clientX - dx, next.clientY - dy,
        { width: window.innerWidth, height: window.innerHeight }, BAR);
      node.style.left = `${at.x}px`;
      node.style.top = `${at.y}px`;
      pinnedPos.current = at;
      windowMeta.current.onPositionChange?.(at);
    };
    const end = () => {
      node.classList.remove('is-dragging');
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', end);
      handle.removeEventListener('pointercancel', end);
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', end);
    handle.addEventListener('pointercancel', end);
  };

  const shut = drill.gate.stage.stage !== 'open' && drill.gate.stage.stage !== 'unbound';
  useDrillShown(open && !floating);
  const body = <DrillScope value={floating ? 'quiet' : 'owns'}><TimeRegion runtime={runtime} region={drill.region}>{shut ? <><ShownHour clock={runtime.clock} at={null} /><GateState gate={drill.gate} loading="Loading…" /></> : children}</TimeRegion></DrillScope>;
  const header = <div className={`drill-header${windowed ? ' drill-window-bar' : ''}`} onPointerDown={windowed ? startDrag : undefined}>
    {windowed && <span className="drill-window-grip" aria-hidden="true" />}
    {floating || saved ? null : <button ref={back} type="button" className="button small quiet" onClick={requestClose}><Icon name="chevron-left" />Back</button>}
    {saved ? null : <strong>{title}</strong>}
    <span className="drill-day">{shut ? '' : day}</span>
    {canFollow && <FollowControl following={drill.following} onFollow={drill.follow} onPause={drill.pause} />}
    {onPin && !shut && <button type="button" className="icon-button small" aria-label="Pin as widget" data-tooltip="Pin as widget" onClick={onPin}><Icon name="pin" /></button>}
    {!onPin && pinRefused && !shut && <button type="button" className="icon-button small" aria-disabled="true" aria-label={`Can’t pin: ${pinRefused}`} data-tooltip={pinRefused}><Icon name="pin" /></button>}
    {floating && !saved && <button type="button" className="icon-button small" aria-label={windowed ? `Close ${title}` : 'Close breakdown'} onClick={requestClose}><Icon name="close" /></button>}
  </div>;

  if (saved) return <div className="drill-saved" role="region" aria-label={title}>{header}<div className="drill-body" ref={root}>{body}</div></div>;

  if (floating) return <>
    {main}
    <Popover menu={panel} manual tabIndex={windowed ? -1 : undefined} className={windowed ? 'drill-popout drill-window' : 'drill-popout'} aria-label={title} onToggle={event => { if (event.newState === 'closed' && open) requestClose(); }}>
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
