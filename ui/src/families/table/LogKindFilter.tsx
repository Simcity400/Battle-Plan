import { useContext, useId, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from '../../components/Icons';
import { Popover, usePopover } from '../../components/Popover';
import { Select } from '../../components/Select';
import { ValueState } from '../../components/ValueState';
import TrimmedArt from '../art/TrimmedArt.tsx';
import { formatScalar, formatValueState, valueStateProps } from '../valueFormat.ts';
import { PickerContext, type PickerStatus } from './picker.ts';
import { appliedFacets, facetChoices, layoutEntries, nextOption, visibleEntries, type Entry, type FacetChoices, type Filters, type LogKindFilterItem } from './pickerList.ts';
import './logKindFilter.css';

export type { LogKindFilterItem };

const LIST_MAX: Readonly<Record<'single' | 'multi', number>> = { single: 360, multi: 288 };
const ROW_HEIGHT = { single: 36, multi: 32 } as const;
const READY: PickerStatus = { kind: 'ready' };

export interface LogKindFilterProps {
  readonly shownHour?: () => number | null;
  readonly single?: { readonly selected?: string; readonly label: string; readonly icon?: string; readonly pinned: boolean };
  readonly slot: HTMLElement | null | undefined;
  readonly allLabel: string;
  readonly noun: string;
  readonly items: readonly LogKindFilterItem[];
  readonly filtered?: boolean;
  readonly searchThreshold?: number;
  readonly onToggle: (value: string, nextChecked: boolean) => void;
  readonly onSelectAll: () => void;
}

function Mark({ state }: { state: NonNullable<LogKindFilterItem['state']> }) {
  return <span className="tf-picker-state" data-mark={state.kind}><i aria-hidden="true" /><span data-tooltip-overflow={state.label}>{state.label}</span></span>;
}

function Quantity({ quantity }: { quantity: NonNullable<LogKindFilterItem['quantity']> }) {
  const { value, unit } = quantity;
  const formatted = formatValueState(value);
  if (formatted.kind !== 'known' && formatted.kind !== 'zero') return <ValueState {...valueStateProps(value)} />;
  return <span className="tf-picker-quantity"><b>{formatted.text}</b> {unit}</span>;
}

function Row({ entry, single, active, id, onChoose, onPoint }: {
  entry: Extract<Entry, { kind: 'item' }>; single: boolean; active: boolean; id: string; onChoose: () => void; onPoint: () => void;
}) {
  const { item } = entry;
  return <div id={id} role="option" aria-selected={item.checked} aria-disabled={!!item.unavailable || undefined} className="tf-picker-row" data-active={active || undefined} data-last={entry.last || undefined}
    style={{ top: entry.top, height: entry.height }} onPointerMove={onPoint} onClick={onChoose}>
    {single
      ? <span className="tf-picker-art">{item.icon ? <TrimmedArt src={item.icon} /> : <span className="val-picture-missing" />}</span>
      : <span className="tf-picker-check" aria-hidden="true">{item.checked ? <Icon name="check" /> : null}</span>}
    {!single && item.icon ? <img className="tf-picker-kind" src={item.icon} alt="" /> : null}
    <span className="tf-picker-name"><span className="tf-picker-label" data-tooltip-overflow={item.label}>{item.label}</span>
      {item.unavailable ? <span className="tf-picker-detail"><ValueState kind="missing" reason={item.unavailable} /></span> : null}
      {single && item.detail ? <span className="tf-picker-detail" data-tooltip-overflow={item.detail}>{item.detail}</span> : null}
      {single && !item.detail && item.detailMissing ? <span className="tf-picker-detail"><ValueState kind="missing" reason={item.detailMissing} /></span> : null}</span>
    {single ? <span className="tf-picker-note">{item.state ? <Mark state={item.state} /> : item.quantity ? <Quantity quantity={item.quantity} /> : null}</span> : null}
  </div>;
}

function Facets({ choices, country, kind, filters, setFilters }: {
  choices: FacetChoices; country: string; kind: string; filters: Filters; setFilters: (filters: Filters) => void;
}) {
  const { countries, kinds } = choices;
  const only = countries.length === 1 && !choices.untagged ? countries[0]! : null;
  const shown = country ? countries.find(([tag]) => tag === country)?.[1] : only?.[1];
  return <div className="tf-picker-facets">
    {countries.length ? <Select label="Country" value={only ? only[0] : country} className="tf-picker-chip"
      leading={shown?.flag ? <img className="tf-picker-flag" src={shown.flag} alt="" /> : <Icon name="globe" />}
      options={[...(only ? [] : [{ value: '', label: 'All countries' }]), ...countries.map(([tag, label]) => ({ value: tag, label: label.name }))]}
      onChange={value => setFilters({ ...filters, country: only ? '' : value })} /> : null}
    {kinds.length ? <Select label="Kind" value={kind} className="tf-picker-chip"
      options={[{ value: '', label: 'All kinds' }, ...kinds.map(value => ({ value, label: value }))]}
      onChange={value => setFilters({ ...filters, kind: value })} /> : null}
  </div>;
}

function FilterList({ items, status, single, noun, allLabel, filtered: appliedFilter, searchThreshold = 8, onToggle, onSelectAll, close, filters, setFilters }:
  LogKindFilterProps & { status: PickerStatus; close: () => void; filters: Filters; setFilters: (filters: Filters) => void }) {
  const id = useId();
  const mode = single ? 'single' : 'multi';
  const list = useRef<HTMLDivElement>(null);
  const listbox = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ scrollTop: 0, height: LIST_MAX[mode] });
  const [active, setActive] = useState(-1);
  const choices = useMemo(() => facetChoices(items), [items]);
  const { query } = filters;
  const { country, kind } = appliedFacets(filters, choices);
  const showSearch = !!single || items.length >= searchThreshold;
  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return items.filter(item => (!single || ((!country || item.country === country) && (!kind || item.detail === kind)))
      && (!needle || item.label.toLowerCase().includes(needle) || !!item.detail?.toLowerCase().includes(needle) || !!item.group?.toLowerCase().includes(needle)));
  }, [items, query, country, kind, single]);
  const laid = useMemo(() => layoutEntries(filtered, ROW_HEIGHT[mode]), [filtered, mode]);
  const shown = visibleEntries(laid.entries, view.scrollTop, view.height);
  const selectable = items.filter(item => !item.unavailable);
  const allSelected = selectable.length > 0 && selectable.every(item => item.checked);
  const optionId = (index: number) => `${id}-option-${index}`;

  useLayoutEffect(() => {
    if (list.current) setView({ scrollTop: list.current.scrollTop, height: list.current.clientHeight || LIST_MAX[mode] });
  }, [mode, laid.height]);
  const opened = useRef(false);
  useLayoutEffect(() => {
    if (opened.current || !single || !laid.options.length) return;
    opened.current = true;
    const chosen = laid.options.findIndex(entry => entry.kind === 'item' && entry.item.checked);
    if (chosen > 0) { setActive(chosen); reveal(chosen); }
  });

  const reveal = (index: number) => {
    const entry = laid.options[index];
    const scroller = list.current;
    if (!entry || !scroller) return;
    const top = entry.top < scroller.scrollTop ? entry.top : entry.top + entry.height > scroller.scrollTop + scroller.clientHeight
      ? entry.top + entry.height - scroller.clientHeight : scroller.scrollTop;
    scroller.scrollTop = top;
    setView(previous => ({ ...previous, scrollTop: top }));
  };
  const choose = (entry: Entry | undefined) => {
    if (entry?.kind !== 'item' || entry.item.unavailable) return;
    onToggle(entry.item.value, !entry.item.checked);
    if (single) close();
  };
  const move = (event: KeyboardEvent, from: number) => {
    const next = nextOption(event.key, from, laid.options.length, Math.max(1, Math.floor(view.height / ROW_HEIGHT[mode]) - 1));
    if (next === null) return false;
    event.preventDefault();
    setActive(next);
    reveal(next);
    return true;
  };
  const setQuery = (next: Filters) => {
    setFilters(next);
    setActive(-1);
    if (list.current) list.current.scrollTop = 0;
    setView(previous => ({ ...previous, scrollTop: 0 }));
  };

  const empty = status.kind === 'loading' ? <p className="tf-picker-empty" role="status"><span className="spinner" />Loading {noun}…</p>
    : status.kind === 'failed' ? <p className="tf-picker-empty" role="alert">{status.message}</p>
    : status.kind === 'unrecorded' ? <p className="tf-picker-empty" role="status">The {noun} are not recorded at the shown hour.</p>
    : !filtered.length ? <p className="tf-picker-empty" role="status">{items.length ? `No ${noun} match${query.trim() ? ` “${query.trim()}”` : ' these filters'}.` : `No ${noun} recorded.`}</p>
    : null;

  return <>
    {showSearch ? <label className="tf-picker-search">
      <Icon name="search" />
      <input type="text" data-popover-autofocus value={query} aria-controls={`${id}-list`} aria-label={`Search ${noun}`}
        placeholder={single ? `Search ${noun}` : `Search ${formatScalar(items.length, 'integer')} ${noun}`} spellCheck={false} autoComplete="off"
        onChange={event => setQuery({ ...filters, query: event.target.value })}
        onKeyDown={event => {
          if (event.key === 'Enter' && laid.options.length) { event.preventDefault(); choose(laid.options[Math.max(0, active)]); return; }
          if ((event.key !== 'ArrowDown' && event.key !== 'ArrowUp') || !laid.options.length) return;
          listbox.current?.focus({ preventScroll: true });
          move(event, active);
        }} />
    </label> : null}
    {single ? <Facets choices={choices} country={country} kind={kind} filters={filters} setFilters={setQuery} /> : null}
    <div ref={list} className="tf-picker-list themed-scrollbar" style={{ maxHeight: LIST_MAX[mode] }}
      onScroll={event => setView({ scrollTop: event.currentTarget.scrollTop, height: event.currentTarget.clientHeight || LIST_MAX[mode] })}>
      <div ref={listbox} id={`${id}-list`} role="listbox" aria-label={single ? `Choose ${noun}` : `Show ${noun}`} aria-multiselectable={!single || undefined}
        tabIndex={showSearch ? -1 : 0} data-popover-autofocus={!showSearch || undefined} className="tf-picker-options" style={{ height: laid.height }}
        aria-activedescendant={active >= 0 && active < laid.options.length ? optionId(active) : undefined}
        onFocus={event => { if (event.target === event.currentTarget && active < 0 && laid.options.length) setActive(0); }}
        onKeyDown={event => {
          if ((event.key === 'Enter' || event.key === ' ') && active >= 0) { event.preventDefault(); choose(laid.options[active]); return; }
          move(event, active);
        }}>
        {shown.map(entry => entry.kind === 'group'
          ? <div key={`group:${entry.label}`} role="presentation" className="tf-picker-group" style={{ top: entry.top, height: entry.height }}>{entry.label}</div>
          : <Row key={entry.item.value} entry={entry} single={!!single} active={entry.index === active} id={optionId(entry.index)}
              onPoint={() => { if (active !== entry.index) setActive(entry.index); }} onChoose={() => choose(entry)} />)}
      </div>
      {empty}
    </div>
    {status.kind === 'ready' && status.notice ? <p className="tf-picker-notice"><span className="val-hatch" aria-hidden="true" />{status.notice}</p> : null}
    {(single ? single.pinned : appliedFilter ?? (selectable.length > 0 && !allSelected)) ? <button type="button" className="tf-picker-reset" onClick={() => {
      onSelectAll(); setQuery({ ...filters, query: '' }); if (single) close();
    }}>{allLabel}</button> : null}
  </>;
}

export default function LogKindFilter(props: LogKindFilterProps) {
  const { slot, allLabel, single, noun, items } = props;
  const id = useId();
  const menu = usePopover('end');
  const picker = useContext(PickerContext);
  const [hour, setHour] = useState<number | null>(null);
  const [filters, setFilters] = useState<Filters>({ query: '', country: '', kind: '' });
  const selectable = items.filter(item => !item.unavailable);
  const selectedCount = selectable.filter(item => item.checked).length;
  const allSelected = selectable.length > 0 && selectedCount === selectable.length;
  const label = single?.label ?? (selectable.length === 0 && props.filtered ? `Filtered ${noun}` : allSelected || selectable.length === 0
    ? allLabel
    : selectedCount === 0
      ? `No ${noun}`
      : selectedCount === 1
        ? selectable.find(item => item.checked)!.label
        : `${selectedCount} of ${selectable.length} ${noun}`);

  if (!slot || (items.length === 0 && !single && !picker)) return null;

  const list = (choices: readonly LogKindFilterItem[], status: PickerStatus = READY): ReactNode =>
    <FilterList {...props} items={choices} status={status} filters={filters} setFilters={setFilters} close={menu.close} />;
  return createPortal(
    <>
      <button ref={menu.trigger} type="button" className="tf-filter-chip" data-subject={single ? true : undefined} popoverTarget={id}
        aria-haspopup="dialog" aria-expanded={menu.open} aria-label={`${single ? `${noun[0]!.toUpperCase()}${noun.slice(1)}` : `Filter ${noun}`}: ${label}`}
        data-tooltip={single ? label : `Filter ${noun}`}>
        {single ? single.icon ? <span className="tf-filter-art"><TrimmedArt src={single.icon} /></span> : <Icon name="helmet" className="tf-filter-icon" />
          : <Icon name="filter" className="tf-filter-icon" />}
        <span className="tf-filter-label">{label}</span>
        <Icon name="chevron-down" className="tf-filter-chevron" />
      </button>
      <Popover menu={menu} onToggle={event => {
        if (event.newState === 'open' && picker) setHour(props.shownHour?.() ?? null);
        if (event.newState === 'closed') setFilters(previous => ({ ...previous, query: '' }));
      }} id={id} className={single ? 'tf-picker is-single' : 'tf-picker'} aria-label={single ? `Choose ${noun}` : `Filter ${noun}`}>
        {menu.open && (picker ? picker(hour, single?.selected, list) : list(items))}
      </Popover>
    </>,
    slot,
  );
}
