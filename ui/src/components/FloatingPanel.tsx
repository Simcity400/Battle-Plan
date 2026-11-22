import { useEffect, useLayoutEffect, useRef } from 'react';
import type { PointerEvent as ReactPointerEvent, ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from './Icons';
import { BAR, MARGIN, clampPanel } from './floatingPlacement.ts';
import './FloatingPanel.css';

export interface FloatingAnchor { x: number; y: number }

function place(panel: HTMLElement, x: number, y: number, shown = Infinity) {
  const at = clampPanel(panel.offsetWidth, panel.offsetHeight, x, y, { width: innerWidth, height: innerHeight }, shown);
  panel.style.left = at.x + 'px';
  panel.style.top = at.y + 'px';
}

export default function FloatingPanel({ title, label, anchor, onClose, className = '', children }: {
  title: ReactNode; label: string; anchor: FloatingAnchor; onClose: () => void; className?: string; children: ReactNode;
}) {
  const panel = useRef<HTMLElement>(null);
  const opener = useRef<Element | null>(null);
  const close = useRef(onClose);
  close.current = onClose;
  useLayoutEffect(() => {
    const node = panel.current;
    if (!node) return;
    opener.current = document.activeElement;
    node.showPopover();
    const fits = anchor.x + 16 + node.offsetWidth + MARGIN <= innerWidth;
    place(node, fits ? anchor.x + 16 : anchor.x - 16 - node.offsetWidth, anchor.y - 24);
    node.focus({ preventScroll: true });
    const keep = () => place(node, node.offsetLeft, node.offsetTop);
    addEventListener('resize', keep);
    return () => {
      removeEventListener('resize', keep);
      if (node.isConnected && node.matches(':popover-open')) node.hidePopover();
      if (opener.current instanceof HTMLElement && opener.current.isConnected) opener.current.focus({ preventScroll: true });
    };
  }, []);
  useEffect(() => {
    const node = panel.current;
    if (!node) return;
    const observer = new ResizeObserver(() => place(node, node.offsetLeft, node.offsetTop));
    observer.observe(node);
    return () => observer.disconnect();
  }, []);
  const drag = (event: ReactPointerEvent<HTMLElement>) => {
    const node = panel.current;
    if (!node || event.button !== 0 || (event.target as Element).closest('button')) return;
    event.preventDefault();
    const handle = event.currentTarget, dx = event.clientX - node.offsetLeft, dy = event.clientY - node.offsetTop;
    handle.setPointerCapture(event.pointerId);
    node.classList.add('is-dragging');
    const move = (next: PointerEvent) => place(node, next.clientX - dx, next.clientY - dy, BAR);
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
  return createPortal(<section ref={panel} popover="manual" role="dialog" aria-label={label} tabIndex={-1} className={`floating-panel ${className}`}
    onKeyDown={event => {
      if (event.key === 'Escape' && !event.defaultPrevented) { event.preventDefault(); event.stopPropagation(); close.current(); }
    }}>
    <header className="floating-panel-bar" onPointerDown={drag}>
      <span className="floating-panel-grip" aria-hidden="true" />
      <strong className="floating-panel-title">{title}</strong>
      <button type="button" className="icon-button floating-panel-close" aria-label={`Close ${label}`} data-tooltip="Close" onClick={() => close.current()}><Icon name="close" /></button>
    </header>
    <div className="floating-panel-body">{children}</div>
  </section>, document.body);
}
