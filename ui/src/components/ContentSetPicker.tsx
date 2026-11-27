import { useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { Popover, usePopover } from './Popover';
import { Icon } from './Icons';
import { COUNTRY_ROW_HEIGHT, moveIndex, revealRow } from './countryList';
import { contentChoices, contentWindow, filterContent } from './contentList';
import type { ContentChoice } from './contentList';
import type { ChoicesUnavailable } from '../dashboard/definitions';
import { failureText } from '../dashboard/queryError.ts';
import './ContentSetPicker.css';

const LIST_HEIGHT = 340;

export default function ContentSetPicker({ label, automatic, choices, selected, onToggle, onReset, single = false, trigger, triggerClassName, unavailable }: {
  label: string; automatic: string; choices: readonly ContentChoice[]; selected: readonly string[];
  onToggle: (value: string) => void; onReset: () => void;
  single?: boolean; trigger?: ReactNode; triggerClassName?: string; unavailable?: ChoicesUnavailable;
}) {
  const id = useId();
  const listId = `${id}-list`;
  const optionId = (index: number) => `${id}-option-${index}`;
  const menu = usePopover();
  const list = useRef<HTMLDivElement>(null);
  const listbox = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(-1);
  const [view, setView] = useState({ scrollTop: 0, height: LIST_HEIGHT });
  const rows = useMemo(() => unavailable && !choices.length ? [] : contentChoices(choices, selected), [choices, selected, unavailable]);
  const chosen = useMemo(() => new Set(selected), [selected]);
  const filtered = useMemo(() => filterContent(rows, query), [rows, query]);
  const window = contentWindow(filtered, view.scrollTop, view.height);
  useLayoutEffect(() => {
    if (menu.open && list.current) setView({ scrollTop: list.current.scrollTop, height: list.current.clientHeight || LIST_HEIGHT });
  }, [menu.open]);
  const scrollTo = (scrollTop: number) => {
    if (list.current) list.current.scrollTop = scrollTop;
    setView(previous => ({ ...previous, scrollTop }));
  };
  const reset = () => { setActive(-1); scrollTo(0); };
  const moveTo = (index: number) => {
    setActive(index);
    scrollTo(revealRow(index, list.current?.scrollTop ?? view.scrollTop, list.current?.clientHeight || view.height));
  };
  const toggle = (index: number) => {
    const row = filtered[index];
    if (row && !row.unavailable) { onToggle(row.value); if (single) menu.close(); }
  };
  return <div className="content-set-picker">
    <button type="button" ref={menu.trigger} popoverTarget={id} aria-label={`Choose ${label.toLowerCase()}`}
      aria-haspopup="dialog" aria-expanded={menu.open} className={triggerClassName ?? 'button small quiet'}>
      {trigger ?? <span>{selected.length ? `${selected.length.toLocaleString()} selected` : automatic}</span>}<Icon name="chevron-down" />
    </button>
    <Popover menu={menu} portal={false} id={id} aria-label={label} className="content-picker"
      onToggle={event => { if (event.newState === 'closed') { setQuery(''); reset(); } }}>
      {menu.open && <>
        <div className="content-picker-search"><Icon name="search" /><input data-popover-autofocus aria-label={`Search ${label.toLowerCase()}`}
          placeholder={`Search ${label.toLowerCase()}`} aria-controls={listId} value={query}
          onChange={event => { setQuery(event.currentTarget.value); reset(); }}
          onKeyDown={event => { if (event.key === 'ArrowDown' && filtered.length) { event.preventDefault(); listbox.current?.focus({ preventScroll: true }); moveTo(0); } }} />
          {!unavailable && <span className="pill">{rows.length.toLocaleString()}</span>}</div>
        <button type="button" className="button small quiet" aria-pressed={!selected.length} onClick={() => { onReset(); if (single) menu.close(); }}>{automatic}</button>
        <div ref={list} className="content-picker-list" onScroll={event => setView({ scrollTop: event.currentTarget.scrollTop,
          height: event.currentTarget.clientHeight || LIST_HEIGHT })}>
          <div ref={listbox} id={listId} role="listbox" aria-label={label} aria-multiselectable={!single} tabIndex={filtered.length ? 0 : -1}
            className="content-picker-options" style={{ height: filtered.length * COUNTRY_ROW_HEIGHT }}
            aria-activedescendant={active >= window.start && active < window.end ? optionId(active) : undefined}
            onFocus={event => { if (event.target === event.currentTarget && active < 0 && filtered.length) setActive(Math.min(filtered.length - 1, Math.ceil(view.scrollTop / COUNTRY_ROW_HEIGHT))); }}
            onKeyDown={event => {
              if ((event.key === 'Enter' || event.key === ' ') && active >= 0) { event.preventDefault(); toggle(active); return; }
              const next = moveIndex(event.key, active, filtered.length);
              if (next !== null) { event.preventDefault(); moveTo(next); }
            }}>
            {window.rows.map((row, offset) => {
              const index = window.start + offset;
              return <div key={row.value} id={optionId(index)} role="option" data-value={row.value} aria-selected={chosen.has(row.value)} aria-disabled={!!row.unavailable}
                aria-setsize={filtered.length} aria-posinset={index + 1} className="content-picker-option" data-active={index === active || undefined}
                style={{ top: index * COUNTRY_ROW_HEIGHT }} data-tooltip={[row.hint, row.unavailable].filter(Boolean).join('\n') || row.label} onClick={() => toggle(index)}>
                {row.icon && <img src={row.icon} alt="" />}<span>{row.label}</span>{chosen.has(row.value) && <Icon name="check" />}
              </div>;
            })}
          </div>
          {unavailable ? 'loading' in unavailable
            ? <p className="content-picker-empty" role="status"><span className="spinner" />Loading choices…</p>
            : <p className="content-picker-empty" role="alert" data-detail={unavailable.error.message}>{failureText(unavailable.error, 'Couldn’t load choices.')}{' '}
              <button type="button" className="button small quiet" onClick={unavailable.retry}>Retry</button></p>
            : !filtered.length && <p className="content-picker-empty">{query ? 'No matching choices' : 'No recorded choices'}</p>}
        </div>
      </>}
    </Popover>
  </div>;
}
