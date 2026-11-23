import { useRef } from 'react';
import { Popover, usePopover } from '../../components/Popover';
import { endAligned, type TableSettings } from '../../draw/tables/logic.ts';
import { columnWidthVar, DISPLAY_LABELS, TableHeader as BaseTableHeader, type TableHeaderColumn } from '../../draw/tables/TableHeader.tsx';
import type { TableColumnSpec } from './model.ts';
import './tableFamily.css';

export interface FamilyTableHeaderProps {
  columns: readonly TableHeaderColumn[];
  allColumns: readonly TableColumnSpec[];
  hiddenColumns: readonly string[];
  settings: TableSettings;
  unsorted?: boolean;
  rankWidth?: number;
  pinned: boolean;
  scrollLeft: number;
  onChange(update: (next: TableSettings) => void): void;
  onResize(column: string, width: number | null): void;
  onResizing(column: string, width: number): void;
}

export function FamilyTableHeader({
  columns,
  allColumns,
  hiddenColumns,
  settings,
  unsorted = false,
  rankWidth = 0,
  pinned,
  scrollLeft,
  onChange,
  onResize,
  onResizing,
}: FamilyTableHeaderProps) {
  const chooser = usePopover('end');
  const hiddenSet = new Set(hiddenColumns);
  const sort = settings.sort;
  const firstKey = allColumns[0]?.key;

  if (unsorted) {
    return <div className="tf-head" role="row" style={{ transform: `translateX(${-scrollLeft}px)` }}>
      {rankWidth > 0 && <div className="tf-th tf-rank" role="columnheader" style={{ width: rankWidth }}>#</div>}
      {columns.map((item, index) => {
        const end = endAligned(item);
        const stick = pinned && index === 0;
        return <div key={item.key} role="columnheader" className={`tf-th${end ? ' end' : ''}${stick ? ' pinned' : ''}`}
          style={{ width: `var(${columnWidthVar(item.key)}, ${item.width}px)`, ...(stick && { transform: `translateX(${scrollLeft}px)` }) }}>
          <span className="tf-th-static" title={item.hint}>{item.label}</span>
        </div>;
      })}
    </div>;
  }

  return <div className="tf-head-row">
    <BaseTableHeader
      columns={columns}
      settings={settings}
      rankWidth={rankWidth}
      pinned={pinned}
      scrollLeft={scrollLeft}
      onChange={onChange}
      onResize={onResize}
      onResizing={onResizing}
    />
    {hiddenColumns.length > 0 && <div className="tf-more-slot">
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
          const locked = col.key === firstKey || col.key === sort.column || col.hideable === false;
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
    </div>}
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
      hint: spec?.hint,
      displays: spec && spec.offered.length > 1 ? spec.offered.map(display => [display, DISPLAY_LABELS[display]] as const) : [],
    };
  });
}

export function useHeaderRef() {
  return useRef<HTMLDivElement>(null);
}
