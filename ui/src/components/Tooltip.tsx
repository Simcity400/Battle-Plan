import { useEffect, useId, useRef } from 'react';
import { hasMapOverlay } from '../shell/mapOverlays';
import { commitZone } from '../shell/commitCount';

const GAP = 6;
const OCCUPANTS = 'button, a[href], input, select, textarea, label, [role="button"], [role^="menuitem"], [role="option"], [role="tab"], [role="link"], [tabindex]:not([tabindex="-1"]), .group-label';

type Area = { left: number; top: number; right: number; bottom: number };

function occupied(area: Area, anchor: Element, tip: Element): boolean {
  const xs = [area.left + 1, (area.left + area.right) / 2, area.right - 1], ys = [area.top + 1, (area.top + area.bottom) / 2, area.bottom - 1];
  for (const x of xs) for (const y of ys) for (const hit of document.elementsFromPoint(x, y)) {
    if (tip.contains(hit)) continue;
    const owner = hit.closest(OCCUPANTS);
    if (owner && !owner.contains(anchor) && !anchor.contains(owner)) return true;
  }
  return false;
}

export default function Tooltip() {
  const id = useId();
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const tip = ref.current!;
    let target: Element | null = null;
    let owner: Element | null = null;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const close = () => {
      clearTimeout(timer); timer = undefined;
      observer.disconnect(); shown.disconnect(); sized.disconnect();
      if (target) {
        target.removeAttribute('data-label-wrapped');
        const description = (target.getAttribute('aria-describedby') ?? '').split(/\s+/).filter(value => value && value !== id).join(' ');
        if (description) target.setAttribute('aria-describedby', description); else target.removeAttribute('aria-describedby');
      }
      target = null; owner = null;
      if (tip.isConnected && tip.matches(':popover-open')) tip.hidePopover();
    };
    const position = () => {
      if (!target) return;
      const anchor = target.getBoundingClientRect();
      const overlay = hasMapOverlay();
      const native = !overlay && !!document.querySelector('[data-native-map]');
      const sidebar = !overlay ? target.closest('.sidebar')?.getBoundingClientRect() : null;
      const header = !overlay ? target.closest('.shell-header')?.getBoundingClientRect() : null;
      const nativeHeader = !sidebar && native ? header : null;
      tip.style.maxWidth = sidebar && native ? `${Math.max(80, sidebar.width - 2 * GAP)}px` : nativeHeader ? `${Math.max(80, nativeHeader.width - 2 * GAP)}px` : '';
      tip.style.maxHeight = nativeHeader ? `${Math.max(24, nativeHeader.height - 2 * GAP)}px` : '';
      tip.toggleAttribute('data-compact', !!nativeHeader);
      const bounds = tip.getBoundingClientRect();
      const viewport = document.documentElement;
      const screen = { left: 8, top: 8, right: viewport.clientWidth - 8, bottom: viewport.clientHeight - 8 };
      const place = (left: number, top: number, area = screen) => {
        tip.style.left = `${Math.max(area.left, Math.min(left, area.right - bounds.width))}px`;
        tip.style.top = `${Math.max(area.top, Math.min(top, area.bottom - bounds.height))}px`;
      };
      const middle = anchor.top + (anchor.height - bounds.height) / 2;
      const centre = anchor.left + (anchor.width - bounds.width) / 2;
      if (sidebar) {
        if (!native) { place(sidebar.right + GAP, middle); return; }
        const area = { ...screen, left: sidebar.left + GAP, right: sidebar.right - GAP };
        const content = document.createRange();
        content.selectNodeContents(target);
        const after = Math.max(anchor.left, Math.min(anchor.right, content.getBoundingClientRect().right)) + GAP;
        const top = Math.max(area.top, Math.min(middle, area.bottom - bounds.height));
        const clear = (left: number, top: number) => !occupied({ left, top, right: left + bounds.width, bottom: top + bounds.height }, target!, tip);
        const left = Math.max(area.left, Math.min(centre, area.right - bounds.width));
        const underneath = anchor.bottom + GAP, over = anchor.top - bounds.height - GAP;
        const fits = (top: number) => top >= area.top && top + bounds.height <= area.bottom;
        if (after + bounds.width <= area.right && clear(after, top)) { place(after, top, area); return; }
        for (let step = 0; underneath + step <= area.bottom || over - step >= area.top; step += 8)
          for (const at of [underneath + step, over - step]) if (fits(at) && clear(left, at)) { place(left, at, area); return; }
        tip.hidePopover();
        if (clipped) target.setAttribute('data-label-wrapped', '');
        return;
      }
      if (nativeHeader) {
        const area = { left: Math.max(screen.left, nativeHeader.left + GAP), right: Math.min(screen.right, nativeHeader.right - GAP), top: nativeHeader.top, bottom: nativeHeader.bottom };
        if (anchor.bottom + GAP + bounds.height <= area.bottom) { place(centre, anchor.bottom + GAP, area); return; }
        place(anchor.right + GAP + bounds.width <= area.right ? anchor.right + GAP : anchor.left - bounds.width - GAP, middle, area);
        return;
      }
      if (header) {
        place(centre, anchor.bottom + GAP);
        return;
      }
      const side = target.closest('[data-tooltip-side]')?.getAttribute('data-tooltip-side');
      if (side === 'left' && anchor.left - bounds.width - GAP >= screen.left) { place(anchor.left - bounds.width - GAP, middle); return; }
      if (side === 'right' && anchor.right + GAP + bounds.width <= screen.right) { place(anchor.right + GAP, middle); return; }
      const above = side === 'left' || side === 'right' || side === 'above' || !overlay && !!target.closest('.dash-widget-header');
      place(centre, !above && anchor.bottom + GAP + bounds.height <= screen.bottom ? anchor.bottom + GAP : anchor.top - bounds.height - GAP);
    };
    let clipped = false;
    const overflows = (element: Element) => {
      const label = element.hasAttribute('data-tooltip-overflow') ? element.querySelector('[data-overflow-label]') ?? element : null;
      return !!label && (label.scrollWidth > label.clientWidth + 1 || !label.getClientRects().length);
    };
    const text = (element: Element) =>
      [clipped ? element.getAttribute('data-tooltip-overflow') : null, element.getAttribute('data-tooltip')].filter(Boolean).join(' · ') || null;
    const observer = new MutationObserver(() => {
      const label = target?.isConnected ? text(target) : null;
      if (!label) { close(); return; }
      if (tip.textContent !== label) tip.textContent = label;
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
      if (!next) return;
      target = next;
      timer = setTimeout(() => {
        timer = undefined;
        clipped = next.isConnected && overflows(next);
        const label = next.isConnected && next.getClientRects().length ? text(next) : null;
        if (!label) { close(); return; }
        tip.textContent = label;
        next.setAttribute('aria-describedby', [next.getAttribute('aria-describedby'), id].filter(Boolean).join(' '));
        tip.showPopover(); position();
        observer.observe(document.body, { childList: true, subtree: true });
        observer.observe(next, { attributes: true, attributeFilter: ['data-tooltip', 'data-tooltip-overflow'] });
        shown.observe(next);
        measured = false;
        sized.observe(tip);
      }, 500);
    };
    const element = (value: EventTarget | null) => value instanceof Element ? value : null;
    const TIPPED = '[data-tooltip], [data-tooltip-overflow]';
    const follow = (node: Element) => {
      const id = node.getAttribute('aria-activedescendant');
      const active = id ? document.getElementById(id) : null;
      schedule((active ?? node).closest(TIPPED));
      owner = active ? node : null;
    };
    const enter = (event: Event) => {
      const node = element(event.target);
      if (tip.contains(node)) return;
      if (event.type === 'focusin' && node) follow(node);
      else schedule(node?.closest(TIPPED) ?? null);
    };
    const moved = new MutationObserver(records => {
      for (const record of records) if (record.target === document.activeElement) follow(record.target as Element);
    });
    moved.observe(document.body, { attributes: true, attributeFilter: ['aria-activedescendant'], subtree: true });
    const leave = (event: PointerEvent | FocusEvent) => {
      if (owner && event.type === 'focusout' && owner === element(event.target)) { close(); return; }
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
      if (!timer || !(target?.contains(document.activeElement) || owner === document.activeElement)) close();
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
      close(); moved.disconnect();
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
