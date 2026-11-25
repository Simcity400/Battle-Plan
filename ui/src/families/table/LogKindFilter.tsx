import { PickerContext } from './picker.ts';
import { useContext, useId, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from '../../components/Icons';
import { Popover, usePopover } from '../../components/Popover';

export interface LogKindFilterItem {
  readonly value: string;
  readonly label: string;
  readonly group?: string;
  readonly checked: boolean;
  readonly country?: string;
  readonly detail?: string;
  readonly icon?: string;
  readonly note?: string;
}

export interface LogKindFilterProps {
  readonly shownHour?: () => number | null;
  readonly single?: { readonly selected?: string; readonly label: string; readonly icon?: string; readonly pinned: boolean; readonly countryTag?: string; readonly countryLabel?: string; readonly countryFlag?: string };
  readonly slot: HTMLElement | null | undefined;
  readonly allLabel: string;
  readonly noun: string;
  readonly compact?: boolean;
  readonly items: readonly LogKindFilterItem[];
  readonly searchThreshold?: number;
  readonly onToggle: (value: string, nextChecked: boolean) => void;
  readonly onSelectAll: () => void;
}

interface Filters { query: string; country: string; kind: string }

function FilterList({ items, single, noun, allLabel, searchThreshold = 8, onToggle, onSelectAll, close, filters, setFilters, notice }:
  LogKindFilterProps & { close: () => void; filters: Filters; setFilters: (filters: Filters) => void; notice?: string | null }) {
  const allSelected = items.length > 0 && items.every(item => item.checked);
  const { query, country, kind } = filters;
  const countries = useMemo(() => [...new Set(items.flatMap(item => item.country ? [item.country] : []))].sort(), [items]);
  const kinds = useMemo(() => [...new Set(items.flatMap(item => item.detail ? [item.detail] : []))].sort(), [items]);
  const showSearch = single || items.length >= searchThreshold;
  const filtered = useMemo(() => {
    const trimmed = query.trim().toLowerCase();
    if (single) return items.filter(item => (!country || item.country === country) && (!kind || item.detail === kind)
      && (!trimmed || item.label.toLowerCase().includes(trimmed)));
    if (!trimmed) return items;
    return items.filter(item => item.label.toLowerCase().includes(trimmed) || (item.group?.toLowerCase().includes(trimmed) ?? false));
  }, [items, query, country, kind, single]);

  const grouped = new Map<string, LogKindFilterItem[]>();
  for (const item of filtered) {
    const group = item.group ?? '';
    let list = grouped.get(group);
    if (!list) grouped.set(group, list = []);
    list.push(item);
  }

  const oneCountry = countries.length === 1 && single?.countryTag === countries[0];
  return (
    <>
      {showSearch ? (
        <div className="log-kind-filter-search">
          {single && <Icon name="search" />}
          <input
            type="search"
            value={query}
            onChange={event => setFilters({ ...filters, query: event.target.value })}
            placeholder={single ? `Search ${noun}` : `Search ${items.length} ${noun}`}
            aria-label={`Search ${noun}`}
            aria-description={notice ?? undefined}
            title={notice ?? undefined}
          />
        </div>
      ) : null}
      {single && <div className="tf-picker-facets">
        <label>{oneCountry && single?.countryFlag && <img alt="" src={single.countryFlag} />}
        <select aria-label="Country" value={country} onChange={event => setFilters({ ...filters, country: event.target.value })}>
          <option value="">{oneCountry ? single?.countryLabel ?? countries[0] : 'All countries'}</option>{countries.length > 1 && countries.map(value => <option key={value}>{value}</option>)}
        </select></label>
        <select aria-label="Kind" value={kind} onChange={event => setFilters({ ...filters, kind: event.target.value })}>
          <option value="">All kinds</option>{kinds.map(value => <option key={value}>{value}</option>)}
        </select>
      </div>}
      <div className="log-kind-filter-list themed-scrollbar">
        {[...grouped.entries()].map(([group, list]) => (
          <div key={group}>
            {group ? <div className="log-kind-filter-group">{group}</div> : null}
            <div className={single ? "tf-picker-options themed-scrollbar" : undefined}>{list.map(item => (
              <button
                key={item.value}
                type="button"
                role={single ? 'menuitemradio' : 'menuitemcheckbox'}
                className="menu-item"
                aria-checked={item.checked}
                onClick={() => { onToggle(item.value, !item.checked); if (single) close(); }}
              >
                {single && (item.icon ? <img className="tf-picker-icon" alt="" src={item.icon} /> : <Icon name="helmet" />)}
                <span className="menu-item-label">{item.label}</span>
                {item.detail && <small>{item.detail}</small>}{item.note && <small>{item.note}</small>}
                {item.checked ? <Icon name="check" /> : null}
              </button>
            ))}</div>
          </div>
        ))}
      </div>
      {(single ? single.pinned : !allSelected) ? (
        <div className="log-kind-filter-footer">
          <button type="button" className="log-kind-filter-all" onClick={() => { onSelectAll(); setFilters({ ...filters, query: '' }); if (single) close(); }}>
            {allLabel}
          </button>
        </div>
      ) : null}
    </>
  );
}

export default function LogKindFilter(props: LogKindFilterProps) {
  const { slot, allLabel, single, noun, items, compact } = props;
  const id = useId();
  const menu = usePopover('end');
  const picker = useContext(PickerContext);
  const [hour, setHour] = useState<number | null>(null);
  const [filters, setFilters] = useState<Filters>({ query: '', country: '', kind: '' });
  const selectedCount = items.filter(item => item.checked).length;
  const allSelected = items.length > 0 && selectedCount === items.length;
  const label = single?.label ?? (allSelected || items.length === 0
    ? allLabel
    : selectedCount === 0
      ? `No ${noun}`
      : selectedCount === 1
        ? items.find(item => item.checked)!.label
        : `${selectedCount} of ${items.length} ${noun}`);

  if (!slot || (items.length === 0 && !single)) return null;

  const list = (choices: readonly LogKindFilterItem[], notice?: string | null) =>
    <FilterList {...props} items={choices} filters={filters} setFilters={setFilters} notice={notice} close={menu.close} />;
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
        {single?.icon ? <img className="tf-picker-icon" src={single.icon} alt="" /> : <Icon name={single ? "helmet" : "filter"} className="dash-widget-filter-icon" />}
        {!compact && <span>{label}</span>}
        {!compact && <Icon name="chevron-down" className="dash-view-menu-chevron" />}
      </button>
      <Popover menu={menu} onToggle={event => {
        if (event.newState === 'open' && picker) setHour(props.shownHour?.() ?? null);
      }} id={id} role="menu" className={single ? 'log-kind-filter-menu tf-subject-picker' : 'log-kind-filter-menu'} aria-label={`Filter ${noun}`}>
        {menu.open && (picker ? hour !== null && picker(hour, single?.selected, list) : list(items))}
      </Popover>
    </>,
    slot,
  );
}
