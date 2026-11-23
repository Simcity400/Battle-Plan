import { useRef, type ReactNode } from 'react';
import { Popover, usePopover } from '../../components/Popover';
import {
  HEADER_CARET_LANE,
  endAligned,
  type TableColumnLayout,
  type TableDisplay,
  type TableSettings,
  type TableTone,
} from './settings.ts';
import type { TableColumnSpec } from './model.ts';
import './tableFamily.css';

export const columnWidthVar = (key: string) => `--tf-w-${key.replace(/[^A-Za-z0-9_-]/g, '_')}`;

export const DISPLAY_LABELS: Readonly<Record<TableDisplay, string>> = {
  plain: 'Number',
  faint: 'Number on faint bar',
  track: 'Number + track',
  meterAfter: 'Number + meter',
  meterBefore: 'Meter + number',
  underline: 'Number + underline',
  dot: 'Dot and text',
  outline: 'Outlined pill',
  filled: 'Filled pill',
  text: 'Text only',
  icon: 'Icon and text',
  flagName: 'Flag and name',
  name: 'Name only',
  flag: 'Flag only',
};

export const TONES: readonly (readonly [TableTone, string])[] = [
  ['blue', 'Blue'],
  ['red', 'Red'],
  ['amber', 'Amber'],
  ['green', 'Green'],
  ['grey', 'Grey'],
  ['country', 'Country colour'],
  ['value-bad', 'By value: high is bad'],
  ['value-good', 'By value: high is good'],
];

const BARRED = new Set<TableDisplay>(['faint', 'track', 'meterAfter', 'meterBefore', 'underline']);

const line = (d: string) => <path d={d} fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />;
const GLYPH: Record<TableDisplay, ReactNode> = {
  plain: <path d="M4.5 4.5v7M3 6l1.5-1.5M8.5 6c.4-1.2 3-1.4 3.3 0 .4 1.6-3.4 3.2-3.4 5.5h3.7" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />,
  faint: <><rect x="1.5" y="4" width="13" height="8" rx="2" fill="currentColor" opacity=".28" />{line('M8.5 8h4')}</>,
  track: <>{line('M1.5 8h3')}<rect x="6.5" y="6.5" width="8" height="3" rx="1.5" fill="currentColor" opacity=".28" /><rect x="6.5" y="6.5" width="5" height="3" rx="1.5" fill="currentColor" /></>,
  meterAfter: <>{line('M1.5 8h5.5')}<rect x="9" y="7" width="5.5" height="2" rx="1" fill="currentColor" opacity=".28" /><rect x="9" y="7" width="3.2" height="2" rx="1" fill="currentColor" /></>,
  meterBefore: <><rect x="1.5" y="7" width="5.5" height="2" rx="1" fill="currentColor" opacity=".28" /><rect x="1.5" y="7" width="3.2" height="2" rx="1" fill="currentColor" />{line('M9 8h5.5')}</>,
  underline: <>{line('M6 6.5h7')}<path d="M3 10.5h10" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" opacity=".28" fill="none" /><path d="M3 10.5h6.5" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" fill="none" /></>,
  dot: <><circle cx="3.5" cy="8" r="2" fill="currentColor" />{line('M7.5 8h7')}</>,
  outline: <rect x="1.5" y="4.5" width="13" height="7" rx="3.5" fill="none" stroke="currentColor" strokeWidth="1.3" />,
  filled: <rect x="1.5" y="4.5" width="13" height="7" rx="3.5" fill="currentColor" opacity=".5" />,
  text: line('M2.5 8h11'),
  flagName: <><rect x="1.5" y="5" width="5" height="6" rx="1" fill="currentColor" opacity=".6" />{line('M9 8h5.5')}</>,
  name: line('M2.5 8h11'),
  flag: <rect x="4" y="4" width="8" height="8" rx="1.2" fill="currentColor" opacity=".6" />,
  icon: <><rect x="1.5" y="5" width="5" height="6" rx="1" fill="none" stroke="currentColor" strokeWidth="1.3" />{line('M9 8h5.5')}</>,
};

const svg = (children: ReactNode) => <svg viewBox="0 0 16 16" aria-hidden="true">{children}</svg>;
const CARET = svg(<path d="M4 6l4 4 4-4" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />);
const CHECK = svg(<path d="M3.5 8.5l3 3 6-7" fill="none" stroke="currentColor" strokeWidth="1.9" strokeLinecap="round" strokeLinejoin="round" />);
const SORT_DOWN = svg(<path d="M4 3.5v9M2 10.5l2 2 2-2M8.5 4.5h5.5M8.5 8h4M8.5 11.5h2.5" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />);
const SORT_UP = svg(<path d="M4 12.5v-9M2 5.5l2-2 2 2M8.5 4.5h2.5M8.5 8h4M8.5 11.5h5.5" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />);
const HIDE = svg(<><path d="M2 8s2.2-4 6-4 6 4 6 4-2.2 4-6 4-6-4-6-4Z" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" /><circle cx="8" cy="8" r="1.7" fill="none" stroke="currentColor" strokeWidth="1.4" /><path d="M3 13 13 3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" /></>);

export interface TableHeaderColumn extends TableColumnLayout {
  displays: readonly (readonly [TableDisplay, string])[];
  text?: boolean;
  hideable?: boolean;
  sortable?: boolean;
  hint?: string;
}

export interface FamilyTableHeaderProps {
  columns: readonly TableHeaderColumn[];
  allColumns: readonly TableColumnSpec[];
  hiddenColumns: readonly string[];
  settings: TableSettings;
  unsorted?: boolean;
  editableColumns?: boolean;
  rankWidth?: number;
  pinned: boolean;
  scrollLeft: number;
  onChange(update: (next: TableSettings) => void): void;
  onResize(column: string, width: number | null): void;
  onResizing(column: string, width: number): void;
}

function ColumnMenu({ column, settings, onChange, close }: {
  column: TableHeaderColumn;
  settings: TableSettings;
  onChange(update: (next: TableSettings) => void): void;
  close(): void;
}) {
  const own = settings.columns[column.key];
  if (!own) return null;
  const toned = BARRED.has(own.as);
  const sorted = settings.sort.column === column.key ? settings.sort.direction : null;
  const pair = column.text
    ? [['ascending', 'A to Z'], ['descending', 'Z to A']] as const
    : [['descending', 'High to low'], ['ascending', 'Low to high']] as const;
  return <>
    <div className="tf-menu-title">{column.label}</div>
    {column.displays.length > 0 && <>
      <div className="tf-menu-label">Show as</div>
      {column.displays.map(([value, label]) => <button key={value} type="button" role="menuitemradio" aria-checked={own.as === value} className="tf-menu-item"
        onClick={() => onChange(next => { next.columns[column.key]!.as = value; if (BARRED.has(value) && !next.columns[column.key]!.tone) next.columns[column.key]!.tone = 'blue'; })}>
        <span className="tf-menu-icon">{svg(GLYPH[value])}</span><span className="tf-menu-text">{label}</span><span className="tf-menu-check">{CHECK}</span>
      </button>)}
    </>}
    {toned && <>
      <div className="tf-menu-label">Colour<span>{TONES.find(([tone]) => tone === own.tone)?.[1] ?? ''}</span></div>
      <div className="tf-swatches">{TONES.map(([tone, label]) => <button key={tone} type="button" className={`tf-swatch tone-${tone}`} aria-pressed={own.tone === tone}
        aria-label={label} title={label} onClick={() => onChange(next => { next.columns[column.key]!.tone = tone; })} />)}</div>
    </>}
    <div className="tf-menu-label">Sort</div>
    <div className="tf-menu-pair">{pair.map(([direction, label]) => <button key={direction} type="button" aria-pressed={sorted === direction}
      onClick={() => onChange(next => { next.sort = { column: column.key, direction }; })}>{direction === 'descending' ? SORT_DOWN : SORT_UP}<span>{label}</span></button>)}</div>
    {settings.order[0] !== column.key && column.hideable !== false && <>
      <div className="tf-menu-sep" />
      <button type="button" className="tf-menu-item danger" onClick={() => { close(); onChange(next => { next.columns[column.key]!.show = false; }); }}>
        <span className="tf-menu-icon">{HIDE}</span><span className="tf-menu-text">Hide column</span>
      </button>
    </>}
  </>;
}

export function FamilyTableHeader({
  columns,
  allColumns,
  hiddenColumns,
  settings,
  unsorted = false,
  editableColumns = true,
  rankWidth = 0,
  pinned,
  scrollLeft,
  onChange,
  onResize,
  onResizing,
}: FamilyTableHeaderProps) {
  const menu = usePopover('end');
  const chooser = usePopover('end');
  const target = useRef<string | null>(null);
  const drag = useRef<{ key: string; start: number; width: number; current: number } | null>(null);
  const column = columns.find(item => item.key === target.current);
  const hiddenSet = new Set(hiddenColumns);
  const sort = settings.sort;
  const firstKey = allColumns[0]?.key;

  const moreSlot = hiddenColumns.length > 0 && <div className="tf-more-slot">
    <button type="button" className="tf-more-cols" aria-expanded={chooser.open}
      aria-label={`Show ${hiddenColumns.length} hidden ${hiddenColumns.length === 1 ? 'column' : 'columns'}`}
      onClick={event => {
        const box = event.currentTarget.getBoundingClientRect();
        chooser.openAt({ left: box.left, right: box.right, top: box.top, bottom: box.bottom });
      }}>
      +{hiddenColumns.length} {hiddenColumns.length === 1 ? 'column' : 'columns'}
    </button>
    <Popover menu={chooser} className="tf-menu tf-chooser" aria-label="Columns">
      <div className="tf-menu-title">Columns</div>
      {allColumns.map(col => {
        const own = settings.columns[col.key];
        const shown = own?.show ?? true;
        const narrowed = shown && hiddenSet.has(col.key);
        const locked = unsorted || !editableColumns || col.key === firstKey || col.key === sort.column || col.hideable === false;
        return <button key={col.key} type="button" role="menuitemcheckbox" aria-checked={shown} disabled={locked}
          className={`tf-menu-item${narrowed ? ' is-narrowed' : ''}`}
          onClick={() => onChange(draft => {
            const entry = draft.columns[col.key];
            if (entry) entry.show = !entry.show;
          })}>
          <span className="tf-menu-text">{col.label}{narrowed ? ' (in tooltip)' : ''}</span>
          <span className="tf-menu-check">{shown ? '\u2713' : ''}</span>
        </button>;
      })}
    </Popover>
  </div>;

  if (unsorted) {
    return <div className="tf-head-row">
      <div className="tf-head" role="row" style={{ transform: `translateX(${-scrollLeft}px)` }}>
        {rankWidth > 0 && <div className="tf-th tf-rank" role="columnheader" style={{ width: rankWidth, ...(pinned && { transform: `translateX(${scrollLeft}px)`, position: 'relative', zIndex: 3 }) }}>#</div>}
        {columns.map((item, index) => {
          const end = endAligned(item);
          const stick = pinned && index === 0;
          return <div key={item.key} role="columnheader" className={`tf-th${end ? ' end' : ''}${stick ? ' pinned' : ''}`}
            style={{ width: `var(${columnWidthVar(item.key)}, ${item.width}px)`, ...(stick && { transform: `translateX(${scrollLeft}px)` }) }}>
            <span className="tf-th-static" title={item.hint}>{item.label}</span>
          </div>;
        })}
      </div>
      {moreSlot}
    </div>;
  }

  return <div className="tf-head-row">
    <div className="tf-head" role="row" style={{ transform: `translateX(${-scrollLeft}px)` }}>
      {rankWidth > 0 && <div className="tf-th tf-rank" role="columnheader" style={{ width: rankWidth, ...(pinned && { transform: `translateX(${scrollLeft}px)`, position: 'relative', zIndex: 3 }) }}>#</div>}
      {columns.map((item, index) => {
        const active = sort.column === item.key;
        const end = endAligned(item);
        const stick = pinned && index === 0;
        const canSort = item.sortable !== false;
        return <div key={item.key} role="columnheader" aria-sort={canSort ? (active ? sort.direction : 'none') : undefined}
          className={`tf-th${end ? ' end' : ''}${active ? ' sorted' : ''}${stick ? ' pinned' : ''}`}
          style={{ width: `var(${columnWidthVar(item.key)}, ${item.width}px)`, ...(stick && { transform: `translateX(${scrollLeft}px)` }) }}>
          {canSort ? <button type="button" className="tf-th-label" title={item.hint} onClick={() => onChange(next => {
            next.sort = active ? { column: item.key, direction: sort.direction === 'descending' ? 'ascending' : 'descending' }
              : { column: item.key, direction: item.text ? 'ascending' : 'descending' };
          })}><span>{item.label}</span>{active && <span className="tf-sort-mark" aria-hidden="true">{sort.direction === 'descending' ? '↓' : '↑'}</span>}</button>
            : <span className="tf-th-static" title={item.hint}>{item.label}</span>}
          {editableColumns && <button type="button" className="tf-th-menu" aria-label={`${item.label} options`} aria-expanded={menu.open && target.current === item.key}
            onClick={event => { target.current = item.key; const box = event.currentTarget.getBoundingClientRect(); menu.openAt({ left: box.left, right: box.right, top: box.top, bottom: box.bottom }); }}>{CARET}</button>}
          {editableColumns && <span className="tf-resize" title="Drag to resize. Double-click to fit." style={{ width: 8 }}
            onPointerDown={event => {
              event.preventDefault(); event.stopPropagation();
              event.currentTarget.setPointerCapture(event.pointerId);
              const width = (event.currentTarget.parentElement as HTMLElement).offsetWidth;
              drag.current = { key: item.key, start: event.clientX, width, current: width };
            }}
            onPointerMove={event => {
              const now = drag.current; if (!now) return;
              now.current = Math.max(40, Math.min(1000, Math.round(now.width + event.clientX - now.start)));
              onResizing(now.key, now.current);
            }}
            onPointerUp={() => { const now = drag.current; drag.current = null; if (now && now.current !== now.width) onResize(now.key, now.current); }}
            onPointerCancel={() => { drag.current = null; }}
            onDoubleClick={() => onResize(item.key, null)} />}
        </div>;
      })}
      {editableColumns && <Popover menu={menu} className="tf-menu" aria-label={column ? `${column.label} options` : 'Column options'}>
        {column && <ColumnMenu column={column} settings={settings} onChange={update => onChange(update)} close={menu.close} />}
      </Popover>}
    </div>
    {moreSlot}
  </div>;
}

export function buildHeaderColumns(
  layoutColumns: readonly TableHeaderColumn[],
  specs: ReadonlyMap<string, TableColumnSpec>,
): TableHeaderColumn[] {
  return layoutColumns.map(col => {
    const spec = specs.get(col.key);
    return {
      ...col,
      text: spec?.text,
      hideable: spec?.hideable,
      sortable: spec?.sortable,
      hint: spec?.hint,
      displays: spec && spec.offered.length > 1 ? spec.offered.map(display => [display, DISPLAY_LABELS[display]] as const) : [],
    };
  });
}

export { HEADER_CARET_LANE };
