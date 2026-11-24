import { createContext, useContext, useEffect, useLayoutEffect, useRef } from 'react';
import type { PointerEvent as ReactPointerEvent, ReactNode, RefObject } from 'react';
import { Icon } from '../../components/Icons';
import { Popover, usePopover } from '../../components/Popover';
import { formatDateHover } from '../valueFormat.ts';
import { centerWindowPosition, clampWindowPosition } from './windowPlacement.ts';
import './detailWindow.css';

export { centerWindowPosition, clampWindowPosition, DETAIL_WINDOW_MARGIN } from './windowPlacement.ts';

export interface DetailWindowClock {
  readonly isPlaying?: () => boolean;
  readonly pause?: () => void;
  readonly play?: () => void;
}

export interface DetailWindowPosition {
  readonly left: number;
  readonly top: number;
}

export interface DetailWindowHostProps {
  readonly open: boolean;
  readonly subject: string | null;
  readonly hour: number | null;
  readonly title: string;
  readonly day?: ReactNode;
  readonly placement?: 'center' | 'beside';
  readonly anchor?: RefObject<HTMLElement | null>;
  readonly opener?: RefObject<HTMLElement | null>;
  readonly returnFocus?: string;
  readonly draggable?: boolean;
  readonly pauseClock?: boolean;
  readonly clock?: DetailWindowClock;
  readonly initialPosition?: DetailWindowPosition | null;
  readonly settled?: boolean;
  readonly onPositionChange?: (position: DetailWindowPosition) => void;
  readonly headerActions?: ReactNode;
  readonly className?: string;
  readonly closeLabel?: string;
  readonly onClose: () => void;
  readonly children: ReactNode;
}

const DetailWindowCloseContext = createContext<(() => void) | null>(null);

export function useDetailWindowClose(): (() => void) | null {
  return useContext(DetailWindowCloseContext);
}

export function DetailWindowHost({
  open,
  subject,
  hour,
  title,
  day,
  placement = 'center',
  anchor,
  opener,
  returnFocus,
  draggable = true,
  pauseClock = false,
  clock,
  initialPosition = null,
  settled = true,
  onPositionChange,
  headerActions,
  className = '',
  closeLabel = 'Close window',
  onClose,
  children,
}: DetailWindowHostProps) {
  const visible = open && subject !== null;
  const panel = usePopover('start');
  const panelRef = useRef(panel);
  panelRef.current = panel;
  const draggedPos = useRef<DetailWindowPosition | null>(null);
  const userDragged = useRef(false);
  const wasVisible = useRef(false);
  const pausedByWindow = useRef(false);
  const stateRef = useRef({ onClose, returnFocus, visible, initialPosition, settled, onPositionChange });
  stateRef.current = { onClose, returnFocus, visible, initialPosition, settled, onPositionChange };

  const restoreFocus = (closingPanel: HTMLElement | null) => {
    const targetSelector = stateRef.current.returnFocus;
    const openerEl = opener?.current ?? null;
    if (!targetSelector && !openerEl) return;
    requestAnimationFrame(() => {
      const focused = document.activeElement;
      const inOwnSurface = !focused
        || focused === document.body
        || focused.isConnected === false
        || Boolean(anchor?.current?.contains(focused))
        || Boolean(closingPanel?.contains(focused));
      if (!inOwnSurface) return;
      if (targetSelector && anchor?.current?.isConnected) {
        anchor.current.querySelector<HTMLElement>(targetSelector)?.focus({ preventScroll: true });
        return;
      }
      if (openerEl?.isConnected) openerEl.focus({ preventScroll: true });
    });
  };

  const requestClose = () => {
    const closingPanel = panelRef.current.popup.current;
    if (!stateRef.current.visible) return;
    stateRef.current.visible = false;
    stateRef.current.onClose();
    restoreFocus(closingPanel);
  };

  useEffect(() => {
    if (!visible || !pauseClock || !clock) return;
    const wasPlaying = clock.isPlaying?.() ?? false;
    if (wasPlaying) {
      pausedByWindow.current = true;
      clock.pause?.();
    }
    return () => {
      if (pausedByWindow.current) {
        pausedByWindow.current = false;
        clock.play?.();
      }
    };
  }, [visible, pauseClock, clock]);

  const positionWindow = () => {
    const el = panelRef.current.popup.current;
    if (!el) return;
    const vw = document.documentElement?.clientWidth || window.innerWidth || 1280;
    const vh = document.documentElement?.clientHeight || window.innerHeight || 720;
    const pinned = draggedPos.current;
    if (!pinned && el.style) {
      el.style.left = '0px';
      el.style.top = '0px';
    }
    const rect = typeof el.getBoundingClientRect === 'function' ? el.getBoundingClientRect() : { width: 360, height: 260 };
    const width = rect.width || 360;
    const height = rect.height || 260;
    let next: DetailWindowPosition;
    if (pinned) {
      next = clampWindowPosition(pinned.left, pinned.top, width, height, vw, vh);
    } else if (placement === 'beside' && anchor?.current) {
      const anchorRect = anchor.current.getBoundingClientRect();
      next = clampWindowPosition(anchorRect.right + 8, anchorRect.top - 6, width, height, vw, vh);
    } else {
      next = centerWindowPosition(width, height, vw, vh);
    }
    if (el.style) {
      el.style.left = `${next.left}px`;
      el.style.top = `${next.top}px`;
      el.style.visibility = 'visible';
    }
    if (!pinned && !userDragged.current && stateRef.current.settled && rect.width > 0 && rect.height > 0) {
      draggedPos.current = next;
      stateRef.current.onPositionChange?.(next);
    }
  };

  useLayoutEffect(() => {
    const element = panelRef.current.popup.current;
    if (!visible) {
      wasVisible.current = false;
      draggedPos.current = null;
      userDragged.current = false;
      if (element?.matches?.(':popover-open')) element.hidePopover();
      return;
    }
    const opening = !wasVisible.current;
    wasVisible.current = true;
    if (opening) {
      userDragged.current = false;
      draggedPos.current = stateRef.current.initialPosition ?? null;
    }
    if (element && !element.matches?.(':popover-open')) {
      element.showPopover?.();
    }
    positionWindow();
    if (opening && element && !element.contains(document.activeElement)) {
      element.focus?.({ preventScroll: true });
    }
    const observer = typeof ResizeObserver === 'function' && element ? new ResizeObserver(positionWindow) : null;
    if (observer && element) observer.observe(element);
    window.addEventListener('resize', positionWindow);
    return () => {
      observer?.disconnect();
      window.removeEventListener('resize', positionWindow);
    };
  }, [visible, placement, anchor]);

  useLayoutEffect(() => {
    if (!visible || !settled) return;
    positionWindow();
  }, [visible, settled]);

  useEffect(() => {
    if (!visible || subject === null) return;
    const element = panelRef.current.popup.current;
    if (element && !element.contains(document.activeElement)) {
      element.focus?.({ preventScroll: true });
    }
  }, [visible, subject]);

  useEffect(() => {
    if (!visible) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.defaultPrevented) return;
      const active = document.activeElement;
      const inAnchor = Boolean(anchor?.current?.contains(active));
      const inPanel = Boolean(panelRef.current.popup.current?.contains(active));
      if (inAnchor || inPanel) {
        event.preventDefault();
        requestClose();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [visible, anchor]);

  const onHeaderPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!draggable || event.button !== 0) return;
    if ((event.target as Element | null)?.closest('button, a, input, select, textarea')) return;
    const element = panelRef.current.popup.current;
    if (!element) return;
    if (!element.contains(document.activeElement)) {
      element.focus?.({ preventScroll: true });
    }
    const rect = element.getBoundingClientRect();
    const offsetX = event.clientX - rect.left;
    const offsetY = event.clientY - rect.top;
    userDragged.current = true;
    const onMove = (moveEvent: PointerEvent) => {
      const vw = document.documentElement.clientWidth || window.innerWidth;
      const vh = document.documentElement.clientHeight || window.innerHeight;
      const next = clampWindowPosition(moveEvent.clientX - offsetX, moveEvent.clientY - offsetY, rect.width, rect.height, vw, vh);
      draggedPos.current = { left: next.left, top: next.top };
      element.style.left = `${next.left}px`;
      element.style.top = `${next.top}px`;
      stateRef.current.onPositionChange?.(next);
    };
    const onUp = () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', onUp);
  };

  const displayDay = day !== undefined ? day : hour !== null ? formatDateHover(hour) : '';

  return (
    <Popover
      menu={panel}
      manual
      tabIndex={-1}
      className={`detail-window ${className}`.trim()}
      aria-label={title}
      data-subject={subject ?? undefined}
      data-hour={hour ?? undefined}
      onToggle={event => {
        if (event.newState === 'closed' && visible) requestClose();
      }}
    >
      {visible && (
        <DetailWindowCloseContext.Provider value={requestClose}>
          <div className={`detail-window-header${draggable ? ' is-draggable' : ''}`} onPointerDown={onHeaderPointerDown}>
            <strong>{title}</strong>
            {displayDay ? <span className="detail-window-day">{displayDay}</span> : null}
            <div className="detail-window-actions">
              {headerActions}
              <button type="button" className="icon-button small" aria-label={closeLabel} onClick={requestClose}>
                <Icon name="close" />
              </button>
            </div>
          </div>
          <div className="detail-window-body">{children}</div>
        </DetailWindowCloseContext.Provider>
      )}
    </Popover>
  );
}
