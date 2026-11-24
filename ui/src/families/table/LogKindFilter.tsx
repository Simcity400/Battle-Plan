import { useId, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from '../../components/Icons';
import { Popover, usePopover } from '../../components/Popover';

export interface LogKindFilterItem {
  readonly value: string;
  readonly label: string;
  readonly group?: string;
  readonly checked: boolean;
}

export interface LogKindFilterProps {
  readonly slot: HTMLElement | null | undefined;
  readonly allLabel: string;
  readonly noun: string;
  readonly items: readonly LogKindFilterItem[];
  readonly searchThreshold?: number;
  readonly onToggle: (value: string, nextChecked: boolean) => void;
  readonly onSelectAll: () => void;
}

export default function LogKindFilter({
  slot,
  allLabel,
  noun,
  items,
  searchThreshold = 8,
  onToggle,
  onSelectAll,
}: LogKindFilterProps) {
  const id = useId();
  const menu = usePopover('end');
  const [query, setQuery] = useState('');

  const selectedCount = items.filter(item => item.checked).length;
  const allSelected = items.length > 0 && selectedCount === items.length;
  const label = allSelected || items.length === 0
    ? allLabel
    : selectedCount === 0
      ? `No ${noun}`
      : selectedCount === 1
        ? items.find(item => item.checked)!.label
        : `${selectedCount} of ${items.length} ${noun}`;

  const showSearch = items.length >= searchThreshold;
  const filtered = useMemo(() => {
    const trimmed = query.trim().toLowerCase();
    if (!trimmed) return items;
    return items.filter(item => item.label.toLowerCase().includes(trimmed) || (item.group?.toLowerCase().includes(trimmed) ?? false));
  }, [items, query]);

  if (!slot || items.length === 0) return null;

  const grouped = new Map<string, LogKindFilterItem[]>();
  for (const item of filtered) {
    const group = item.group ?? '';
    let list = grouped.get(group);
    if (!list) grouped.set(group, list = []);
    list.push(item);
  }

  return createPortal(
    <>
      <button
        ref={menu.trigger}
        type="button"
        className="dash-view-menu-trigger"
        popoverTarget={id}
        aria-haspopup="menu"
        aria-expanded={menu.open}
        aria-label={`Filter ${noun}: ${label}`}
        data-tooltip={`Filter ${noun}`}
      >
        <Icon name="filter" className="dash-widget-filter-icon" />
        <span>{label}</span>
        <Icon name="chevron-down" className="dash-view-menu-chevron" />
      </button>
      <Popover menu={menu} id={id} role="menu" className="log-kind-filter-menu" aria-label={`Filter ${noun}`}>
        {menu.open ? (
          <>
            {showSearch ? (
              <div className="log-kind-filter-search">
                <input
                  type="search"
                  value={query}
                  onChange={event => setQuery(event.target.value)}
                  placeholder={`Search ${items.length} ${noun}`}
                  aria-label={`Search ${noun}`}
                />
              </div>
            ) : null}
            <div className="log-kind-filter-list themed-scrollbar">
              {[...grouped.entries()].map(([group, list]) => (
                <div key={group}>
                  {group ? <div className="log-kind-filter-group">{group}</div> : null}
                  {list.map(item => (
                    <button
                      key={item.value}
                      type="button"
                      role="menuitemcheckbox"
                      className="menu-item"
                      aria-checked={item.checked}
                      onClick={() => onToggle(item.value, !item.checked)}
                    >
                      <span className="menu-item-label">{item.label}</span>
                      {item.checked ? <Icon name="check" /> : null}
                    </button>
                  ))}
                </div>
              ))}
            </div>
            {!allSelected ? (
              <div className="log-kind-filter-footer">
                <button type="button" className="log-kind-filter-all" onClick={() => { onSelectAll(); setQuery(''); }}>
                  {allLabel}
                </button>
              </div>
            ) : null}
          </>
        ) : null}
      </Popover>
    </>,
    slot,
  );
}
