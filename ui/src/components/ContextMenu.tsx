import { useCallback, useRef, useState } from 'react';
import { Icon } from './Icons';
import type { IconName } from './Icons';
import { Popover, usePopover } from './Popover';

export interface MenuItem { label: string; icon?: IconName; disabled?: boolean; hint?: string; run: () => void }

export function useContextMenu(label: string) {
  const popover = usePopover('start');
  const [items, setItems] = useState<readonly MenuItem[]>([]);
  const returnFocus = useRef<HTMLElement | null>(null);
  const open = useCallback((x: number, y: number, next: readonly MenuItem[]) => {
    if (!next.length) return;
    returnFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setItems(next);
    popover.openAt({ left: x, right: x, top: y, bottom: y });
  }, [popover.openAt]);
  const firstUsable = items.findIndex(item => !item.disabled);
  const menu = <Popover menu={popover} className="context-menu" aria-label={label}
    onToggle={event => {
      if (event.newState !== 'closed') return;
      const active = document.activeElement;
      if (!active || active === document.body || event.currentTarget.contains(active)) returnFocus.current?.focus({ preventScroll: true });
    }}>
    {items.map((item, index) => <button key={index} type="button" className="menu-item" disabled={item.disabled} data-popover-autofocus={index === firstUsable || undefined}
      onClick={() => { popover.close(); item.run(); }}>
      {item.icon && <Icon name={item.icon} />}<span className="menu-item-label">{item.label}{item.hint && <small>{item.hint}</small>}</span>
    </button>)}
  </Popover>;
  return { open, menu };
}
