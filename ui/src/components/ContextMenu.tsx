import { Fragment, useCallback, useLayoutEffect, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';
import { Icon } from './Icons';
import type { IconName } from './Icons';
import { Popover, usePopover } from './Popover';

export interface Confirmation { title: string; detail: string; action: string }

export interface MenuItem {
  label: string; icon?: IconName; disabled?: boolean; hint?: string; danger?: boolean; confirm?: Confirmation; run: () => void;
}

export function menuKeys(event: KeyboardEvent<HTMLElement>) {
  const items = [...event.currentTarget.querySelectorAll<HTMLElement>('[role^="menuitem"]:not(:disabled)')];
  if (!items.length) return;
  const at = items.indexOf(document.activeElement as HTMLElement);
  const next = event.key === 'ArrowDown' ? (at + 1) % items.length : event.key === 'ArrowUp' ? (at - 1 + items.length) % items.length
    : event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : -1;
  if (next < 0) return;
  event.preventDefault();
  items[next].focus();
}

export function ConfirmStep({ confirmation, onCancel, onConfirm }: { confirmation: Confirmation; onCancel: () => void; onConfirm: () => void }) {
  return <div className="menu-confirm" role="alertdialog" aria-label={confirmation.title}>
    <strong>{confirmation.title}</strong>
    <p>{confirmation.detail}</p>
    <div className="menu-confirm-actions">
      <button type="button" className="button small quiet" autoFocus data-popover-autofocus onClick={onCancel}>Cancel</button>
      <button type="button" className="button small is-danger" onClick={onConfirm}>{confirmation.action}</button>
    </div>
  </div>;
}

export function useContextMenu(label: string) {
  const popover = usePopover('start');
  const [items, setItems] = useState<readonly MenuItem[]>([]);
  const [confirming, setConfirming] = useState<MenuItem | null>(null);
  const returnFocus = useRef<HTMLElement | null>(null);
  const backTo = useRef<number | null>(null);
  useLayoutEffect(() => {
    const at = backTo.current;
    if (confirming || at === null) return;
    backTo.current = null;
    popover.popup.current?.querySelectorAll<HTMLElement>('[role="menuitem"]')[at]?.focus({ preventScroll: true });
  }, [confirming, popover.popup]);
  const open = useCallback((x: number, y: number, next: readonly MenuItem[]) => {
    if (!next.length) return;
    returnFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setItems(next);
    setConfirming(null);
    popover.openAt({ left: x, right: x, top: y, bottom: y });
  }, [popover.openAt]);
  const firstUsable = items.findIndex(item => !item.disabled);
  const choose = (item: MenuItem) => {
    if (item.confirm) { setConfirming(item); return; }
    popover.close(); item.run();
  };
  const back = () => {
    backTo.current = confirming ? items.indexOf(confirming) : null;
    setConfirming(null);
  };
  const menu = <Popover menu={popover} className={`context-menu${confirming ? ' is-confirming' : ''}`} aria-label={label} role={confirming ? 'dialog' : 'menu'}
    onKeyDownCapture={confirming ? event => {
      if (event.key !== 'Escape' || event.defaultPrevented) return;
      event.preventDefault(); event.stopPropagation(); back();
    } : menuKeys}
    onToggle={event => {
      if (event.newState !== 'closed') return;
      setConfirming(null);
      backTo.current = null;
      const active = document.activeElement;
      if (!active || active === document.body || event.currentTarget.contains(active)) returnFocus.current?.focus({ preventScroll: true });
    }}>
    {confirming?.confirm ? <ConfirmStep confirmation={confirming.confirm} onCancel={back}
      onConfirm={() => { popover.close(); confirming.run(); }} />
      : items.map((item, index) => <Fragment key={index}>
        {item.danger && index > 0 && !items[index - 1].danger && <hr className="menu-separator" />}
        <button type="button" role="menuitem" className={`menu-item${item.danger ? ' is-danger' : ''}`} disabled={item.disabled}
          data-popover-autofocus={index === firstUsable || undefined} onClick={() => choose(item)}>
          {item.icon ? <Icon name={item.icon} /> : <span className="menu-item-icon-slot" aria-hidden="true" />}
          {item.hint ? <span className="menu-item-label"><span>{item.label}</span><small>{item.hint}</small></span> : <span className="menu-item-label">{item.label}</span>}
        </button>
      </Fragment>)}
  </Popover>;
  return { open, menu };
}
