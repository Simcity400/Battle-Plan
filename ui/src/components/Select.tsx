import { selectFocusable, selectIndex } from './selectNavigation';
import { VirtualListIndex } from './virtualListIndex';
import { useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import { Icon } from './Icons';
import { Popover, usePopover } from './Popover';
import './Select.css';

export interface SelectOption { value: string; label: string; disabled?: boolean; reason?: string; detail?: string }

export function Select({ id: triggerId, label, value, options, disabled = false, autoFocus, repick = false, className, leading, onOpen, onChange }: {
  id?: string; label: string; value: string; options: readonly SelectOption[]; disabled?: boolean; autoFocus?: boolean;
  repick?: boolean; className?: string; leading?: ReactNode; onOpen?: () => void; onChange: (value: string) => void;
}) {
  const id = useId();
  const menu = usePopover('start');
  const [held, setHeld] = useState(options);
  if (held !== options && !sameOptions(held, options)) setHeld(options);
  const selected = held.find(option => option.value === value);
  const shown = selected?.label ?? value;
  return <>
    <button ref={menu.trigger} id={triggerId} type="button" className={className ? `select-trigger ${className}` : 'select-trigger'} popoverTarget={id} autoFocus={autoFocus} aria-haspopup="listbox" aria-expanded={menu.open}
      aria-label={label} aria-describedby={`${id}-value`} disabled={disabled} data-detail={selected?.detail} data-tooltip={selected?.reason} data-tooltip-overflow={shown}>
      {leading}<span id={`${id}-value`} data-overflow-label>{shown}</span><Icon name="chevron-down" /></button>
    { }
    <Popover menu={menu} portal={false} id={id} className="select-menu" aria-label={label}
      onToggle={event => { if (event.newState === 'open') onOpen?.(); }}>
      {menu.open && <SelectMenu label={label} value={value} options={held} onLeave={menu.close} onChoose={chosen => { menu.close(); if (repick || chosen !== value) onChange(chosen); }} />}

    </Popover>
  </>;
}

const sameOptions = (a: readonly SelectOption[], b: readonly SelectOption[]) => a.length === b.length && a.every((option, at) => {
  const other = b[at];
  return option.value === other.value && option.label === other.label && !!option.disabled === !!other.disabled
    && option.reason === other.reason && option.detail === other.detail;
});

const OPTION_HEIGHT = 32;
function SelectMenu({ label, value, options, onChoose, onLeave }: { label: string; value: string; options: readonly SelectOption[]; onChoose(value: string): void; onLeave(): void }) {
  const selected = options.findIndex(option => option.value === value && selectFocusable(option));
  const [activeValue, setActiveValue] = useState<string | null>(() => options[selected >= 0 ? selected : selectIndex(options, -1, 'Home') ?? -1]?.value ?? null);
  const kept = options.findIndex(option => option.value === activeValue && selectFocusable(option));
  const active = kept >= 0 ? kept : selectIndex(options, -1, 'Home') ?? -1;
  const [top, setTop] = useState(Math.max(0, active * OPTION_HEIGHT));
  const list = useRef<HTMLDivElement>(null);
  const nodes = useRef(new Map<number, HTMLButtonElement>()), observer = useRef<ResizeObserver | null>(null);
  const index = useMemo(() => new VirtualListIndex(options.length, OPTION_HEIGHT), [options]);
  const [, update] = useState(0);
  const height = Math.min(index.offset(options.length), 288);
  const [start, end] = index.range(top, height, 6);
  const visible = Array.from({ length: end - start }, (_, i) => start + i);
  if (active >= 0 && active < options.length && !visible.includes(active)) visible.push(active);
  visible.sort((a, b) => a - b);
  useLayoutEffect(() => {
    const scroller = list.current;
    if (!scroller || typeof ResizeObserver === 'undefined') return;
    let typographyKey = '';
    const sizeWidth = () => {
      const button = nodes.current.values().next().value;
      if (!button) return;
      const style = getComputedStyle(button), canvas = document.createElement('canvas').getContext('2d');
      if (!canvas) return;
      canvas.font = style.font;
      canvas.letterSpacing = style.letterSpacing;
      canvas.fontKerning = style.fontKerning as CanvasFontKerning;
      const nextTypography = `${style.font}:${style.letterSpacing}`;
      if (typographyKey !== nextTypography) {
        typographyKey = nextTypography;
        let changed = false;
        for (let at = 0; at < options.length; at++) changed = index.set(at, OPTION_HEIGHT) || changed;
        if (changed) update(version => version + 1);
      }
      let width = 0;
      for (const option of options) {
        const text = option.label.replace(/[ \t\n\r\f]+/g, ' ').replace(/^ | $/g, '');
        width = Math.max(width, canvas.measureText(text).width + (option.value === value ? 16 + parseFloat(style.columnGap) : 0));
      }
      const gutter = scroller.offsetWidth - scroller.clientWidth;
      scroller.style.width = `${Math.ceil(width + parseFloat(style.paddingLeft) + parseFloat(style.paddingRight) + gutter)}px`;
    };
    sizeWidth();
    let width = scroller.clientWidth;
    observer.current = new ResizeObserver(() => {
      const focused = [...nodes.current].find(([, node]) => node === document.activeElement);
      const wasVisible = focused && focused[1].offsetTop >= scroller.scrollTop
        && focused[1].offsetTop < scroller.scrollTop + scroller.clientHeight;
      let changed = false;
      if (width !== scroller.clientWidth) {
        width = scroller.clientWidth;
        for (let at = 0; at < options.length; at++) changed = index.set(at, OPTION_HEIGHT) || changed;
      }
      for (const [at, node] of nodes.current) changed = index.set(at, node.offsetHeight) || changed;
      if (changed) {
        update(version => version + 1);
        if (wasVisible) reveal(focused[0]);
      }
    });
    observer.current.observe(scroller);
    for (const node of nodes.current.values()) observer.current.observe(node);
    window.addEventListener('resize', sizeWidth);
    document.fonts.addEventListener('loadingdone', sizeWidth);
    const typography = new MutationObserver(sizeWidth);
    typography.observe(document.documentElement, { attributes: true, attributeFilter: ['style', 'class'] });
    return () => {
      observer.current?.disconnect(); observer.current = null; typography.disconnect();
      window.removeEventListener('resize', sizeWidth); document.fonts.removeEventListener('loadingdone', sizeWidth);
    };
  }, [index, options, value]);
  const reveal = (at: number) => {
    const scroller = list.current;
    if (!scroller || at < 0) return;
    const start = index.offset(at), end = index.offset(at + 1);
    const nextTop = start < scroller.scrollTop ? start : end > scroller.scrollTop + scroller.clientHeight ? end - scroller.clientHeight : scroller.scrollTop;
    scroller.scrollTop = nextTop; setTop(nextTop);
  };
  useLayoutEffect(() => {
    const resolved = options[active]?.value ?? null;
    if (activeValue !== resolved) setActiveValue(resolved);
    reveal(active);
    nodes.current.get(active)?.focus({ preventScroll: true });
  }, [active, activeValue, index]);
  const keys = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Tab') { onLeave(); return; }
    const next = selectIndex(options, active, event.key);
    if (next === null) return;
    event.preventDefault();
    reveal(next); setActiveValue(options[next].value);
  };
  return <div ref={list} className="select-options" role="listbox" aria-label={label} onKeyDown={keys} style={{ height }}
    onScroll={event => setTop(event.currentTarget.scrollTop)}>
    <div style={{ height: index.offset(options.length), position: 'relative' }}>
      {visible.map(at => {
        const option = options[at];
        return <button key={option.value} type="button" role="option" className="menu-item" aria-selected={option.value === value}
          aria-checked={option.value === value} aria-posinset={at + 1} aria-setsize={options.length} tabIndex={at === active ? 0 : -1}
          ref={node => {
            const previous = nodes.current.get(at); if (previous) observer.current?.unobserve(previous);
            if (node) { nodes.current.set(at, node); observer.current?.observe(node); }
            else nodes.current.delete(at);
          }}
          style={{ position: 'absolute', top: index.offset(at), minHeight: OPTION_HEIGHT, left: 0, right: 0 }}
          disabled={option.disabled && !option.reason} aria-disabled={option.disabled || undefined} data-detail={option.detail} data-tooltip={option.reason} data-option-index={at}
          data-popover-autofocus={at === active || undefined} onFocus={() => setActiveValue(option.value)} onClick={() => { if (!option.disabled) onChoose(option.value); }}>
          <span className="menu-item-label">{option.label}</span>{option.value === value && <Icon name="check" />}
        </button>;
      })}
    </div>
  </div>;
}
