import { useEffect, useId, useRef } from 'react';
import { hasMapOverlay } from '../shell/mapOverlays';
import { commitZone } from '../shell/commitCount';

export default function Tooltip() {
  const id = useId();
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const tip = ref.current!;
    let target: Element | null = null;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const close = () => {
      clearTimeout(timer); timer = undefined;
      observer.disconnect(); shown.disconnect(); sized.disconnect();
      if (target) {
        const description = (target.getAttribute('aria-describedby') ?? '').split(/\s+/).filter(value => value && value !== id).join(' ');
        if (description) target.setAttribute('aria-describedby', description); else target.removeAttribute('aria-describedby');
      }
      target = null;
      if (tip.isConnected && tip.matches(':popover-open')) tip.hidePopover();
    };
    const position = () => {
      if (!target) return;
      const anchor = target.getBoundingClientRect();
      const overlay = hasMapOverlay();
      const rail = !overlay ? target.closest('.sidebar')?.getBoundingClientRect() : null;
      const header = !overlay ? target.closest('.shell-header')?.getBoundingClientRect() : null;
      tip.style.maxWidth = rail ? `${Math.max(80, rail.width - 16)}px` : '';
      tip.style.maxHeight = header ? `${Math.max(24, header.height - 8)}px` : '';
      const bounds = tip.getBoundingClientRect();
      const viewport = document.documentElement;
      tip.style.left = `${Math.max(rail ? rail.left + 8 : 8, Math.min(anchor.left + (anchor.width - bounds.width) / 2, (rail?.right ?? viewport.clientWidth) - bounds.width - 8))}px`;
      const above = !overlay && !!target.closest('.dash-widget-header');
      const top = !above && anchor.bottom + bounds.height <= viewport.clientHeight - 8 ? anchor.bottom : anchor.top - bounds.height;
      tip.style.top = `${Math.max(header ? header.top + 4 : 8, Math.min(top, (header?.bottom ?? viewport.clientHeight) - bounds.height - (header ? 4 : 8)))}px`;
    };
    const observer = new MutationObserver(() => {
      if (!target?.isConnected || !target.getAttribute('data-tooltip')) { close(); return; }
      const text = target.getAttribute('data-tooltip')!;
      if (tip.textContent !== text) tip.textContent = text;
    });
    const shown = new IntersectionObserver(entries => {
      const last = entries[entries.length - 1];
      if (last && (!last.isIntersecting && last.boundingClientRect.width === 0 && last.boundingClientRect.height === 0)) close();
    });
    let measured = false;
    const sized = new ResizeObserver(() => {
      if (measured) position();
      measured = true;
    });
    const schedule = (next: Element | null) => {
      if (next === target) return;
      close();
      if (!next?.getAttribute('data-tooltip')) return;
      target = next;
      timer = setTimeout(() => {
        timer = undefined;
        if (!next.isConnected || !next.getClientRects().length || !next.getAttribute('data-tooltip')) { close(); return; }
        tip.textContent = next.getAttribute('data-tooltip');
        next.setAttribute('aria-describedby', [next.getAttribute('aria-describedby'), id].filter(Boolean).join(' '));
        tip.showPopover(); position();
        observer.observe(document.body, { childList: true, subtree: true });
        observer.observe(next, { attributes: true, attributeFilter: ['data-tooltip'] });
        shown.observe(next);
        measured = false;
        sized.observe(tip);
      }, 500);
    };
    const element = (value: EventTarget | null) => value instanceof Element ? value : null;
    const enter = (event: Event) => {
      const node = element(event.target);
      if (!tip.contains(node)) schedule(node?.closest('[data-tooltip]') ?? null);
    };
    const leave = (event: PointerEvent | FocusEvent) => {
      const next = element(event.relatedTarget);
      if (target?.contains(next) || tip.contains(next)) return;
      if (target?.contains(element(event.target)) || tip.contains(element(event.target))) close();
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || !target) return;
      if (tip.matches(':popover-open')) { event.preventDefault(); event.stopPropagation(); }
      close();
    };
    const scroll = (event: Event) => {
      if (tip.contains(element(event.target))) return;
      if (!timer || !target?.contains(document.activeElement)) close();
    };
    const toggle = (event: Event) => { if (event.target !== tip) close(); };
    document.addEventListener('pointerover', enter, true);
    document.addEventListener('pointerout', leave, true);
    document.addEventListener('focusin', enter, true);
    document.addEventListener('focusout', leave, true);
    document.addEventListener('pointerdown', close, true);
    document.addEventListener('keydown', escape, true);
    document.addEventListener('scroll', scroll, true);
    document.addEventListener('beforetoggle', toggle, true);
    document.addEventListener('visibilitychange', close);
    window.addEventListener('resize', close);
    window.addEventListener('blur', close);
    return () => {
      close();
      document.removeEventListener('pointerover', enter, true);
      document.removeEventListener('pointerout', leave, true);
      document.removeEventListener('focusin', enter, true);
      document.removeEventListener('focusout', leave, true);
      document.removeEventListener('pointerdown', close, true);
      document.removeEventListener('keydown', escape, true);
      document.removeEventListener('scroll', scroll, true);
      document.removeEventListener('beforetoggle', toggle, true);
      document.removeEventListener('visibilitychange', close);
      window.removeEventListener('resize', close);
      window.removeEventListener('blur', close);
    };
  }, [id]);
  return <div ref={ref} id={id} popover="manual" role="tooltip" className="app-tooltip" />;
}

commitZone(Tooltip, 'tooltip');
