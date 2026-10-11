import { useId } from 'react';
import type { KeyboardEvent } from 'react';
import { Icon } from './Icons';
import { Popover, usePopover } from './Popover';
import './Select.css';

export interface SelectOption { value: string; label: string; disabled?: boolean; reason?: string }

/** A dropdown drawn like the app's other menus, where a native select would show the
 * system's list. Arrow keys move through the options, Enter picks and Escape closes. */
export function Select({ id: triggerId, label, value, options, disabled = false, autoFocus, onChange }: {
  /** The trigger's id, for a separate `<label htmlFor>`. */
  id?: string; label: string; value: string; options: readonly SelectOption[]; disabled?: boolean; autoFocus?: boolean; onChange: (value: string) => void;
}) {
  const id = useId();
  const menu = usePopover('start');
  const selected = options.find(option => option.value === value);
  const keys = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
    event.preventDefault();
    const items = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
    const at = items.indexOf(document.activeElement as HTMLButtonElement);
    items[(at + (event.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length]?.focus();
  };
  return <>
    <button ref={menu.trigger} id={triggerId} type="button" className="select-trigger" popoverTarget={id} autoFocus={autoFocus} aria-haspopup="listbox" aria-expanded={menu.open}
      aria-label={label} disabled={disabled} data-tooltip={selected?.reason}>
      <span>{selected?.label ?? value}</span><Icon name="chevron-down" /></button>
    {/* Kept in place: inside another popover this keeps native light-dismiss nested. */}
    <Popover menu={menu} portal={false} id={id} className="select-menu" aria-label={label}>
      {menu.open && <div role="listbox" aria-label={label} onKeyDown={keys}>
        {options.map(option => <button key={option.value} type="button" role="option" className="menu-item" aria-selected={option.value === value}
          aria-checked={option.value === value} disabled={option.disabled} data-tooltip={option.reason}
          data-popover-autofocus={option.value === value || undefined} onClick={() => { menu.close(); if (option.value !== value) onChange(option.value); }}>
          <span className="menu-item-label">{option.label}</span>{option.value === value && <Icon name="check" />}
        </button>)}
      </div>}
    </Popover>
  </>;
}
