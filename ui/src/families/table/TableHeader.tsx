import { useEffect, useRef, useState, type CSSProperties } from 'react';
import {
  CELL_INSET,
  endAligned,
  ROW_MARGIN_X,
  type TableColumnLayout,
  type TableDisplay,
  type TableSettings,
} from './settings.ts';
import type { TableCellKind, TableColumnSpec, TableHeaderGroup } from './model.ts';
import { TREE_CHEVRON_LANE, treeColumnOf } from './layout.ts';
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
  text: 'Text only',
  icon: 'Icon and text',
  flagName: 'Flag and name',
  name: 'Name only',
  flag: 'Flag only',
};

const SORT_ARROW_DOWN = <svg className="tf-sort-arrow" viewBox="0 0 9 11" width="9" height="11" aria-hidden="true">
  <path d="M4.5 1v8M1.5 6.2 4.5 9.2l3-3" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
</svg>;

const SORT_ARROW_UP = <svg className="tf-sort-arrow" viewBox="0 0 9 11" width="9" height="11" aria-hidden="true">
  <path d="M4.5 10V2M1.5 4.8 4.5 1.8l3 3" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
</svg>;

const GRIP_DOTS = <svg className="tf-th-grip" viewBox="0 0 8 12" width="7" height="11" aria-hidden="true">
  <circle cx="2" cy="2.5" r="1" fill="currentColor" />
  <circle cx="6" cy="2.5" r="1" fill="currentColor" />
  <circle cx="2" cy="6" r="1" fill="currentColor" />
  <circle cx="6" cy="6" r="1" fill="currentColor" />
  <circle cx="2" cy="9.5" r="1" fill="currentColor" />
  <circle cx="6" cy="9.5" r="1" fill="currentColor" />
</svg>;

const visibleBoxes = new Map<string, string | null>();

function visibleBox(image: HTMLImageElement): string | null {
  const { naturalWidth: width, naturalHeight: height } = image;
  const context = Object.assign(document.createElement('canvas'), { width, height }).getContext('2d', { willReadFrequently: true });
  if (!context || !width || !height) return null;
  context.drawImage(image, 0, 0);
  const alpha = context.getImageData(0, 0, width, height).data;
  let top = height, left = width, bottom = -1, right = -1;
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      if (alpha[(y * width + x) * 4 + 3]! < 8) continue;
      if (y < top) top = y;
      if (y > bottom) bottom = y;
      if (x < left) left = x;
      if (x > right) right = x;
    }
  }
  return bottom < 0 ? null : `inset(${top}px ${width - 1 - right}px ${height - 1 - bottom}px ${left}px)`;
}

function GroupIcon({ src }: { readonly src: string }) {
  const [box, setBox] = useState(() => visibleBoxes.get(src));
  useEffect(() => {
    if (visibleBoxes.has(src)) { setBox(visibleBoxes.get(src)); return; }
    let live = true;
    const image = new Image();
    image.src = src;
    image.decode().then(() => {
      let found: string | null = null;
      try { found = visibleBox(image); } catch { found = null; }
      visibleBoxes.set(src, found);
      if (live) setBox(found);
    }, () => {});
    return () => { live = false; };
  }, [src]);
  return <img src={src} alt="" style={box ? { objectViewBox: box } as CSSProperties : undefined} />;
}

export interface TableHeaderColumn extends TableColumnLayout {
  displays: readonly (readonly [TableDisplay, string])[];
  text?: boolean;
  hideable?: boolean;
  sortable?: boolean;
  hint?: string;
  headerGroup?: TableHeaderGroup;
  cellKind?: TableCellKind;
}

export interface TableHeaderIdentity {
  readonly name: string;
  readonly missing: boolean;
  readonly flag: string | null;
  readonly flagPending: boolean;
  readonly country: string | null;
}

function HeaderIdentity({ identity }: { identity: TableHeaderIdentity }) {
  const initials = identity.country ?? identity.name.slice(0, 3);
  return <span className="tf-th-identity">
    {identity.flag
      ? <img className="tf-th-flag" src={identity.flag} alt="" aria-hidden="true" />
      : <span className={`tf-th-flag${identity.flagPending ? '' : ' is-missing'}`} aria-hidden="true">{identity.flagPending ? null : initials}</span>}
    <span className={`tf-th-identity-name${identity.missing ? ' is-missing' : ''}`}>{identity.name}</span>
  </span>;
}

export interface FamilyTableHeaderProps {
  columns: readonly TableHeaderColumn[];
  allColumns: readonly TableColumnSpec[];
  settings: TableSettings;
  unsorted?: boolean;
  treeLane?: boolean;
  editableColumns?: boolean;
  inset?: number;
  rankWidth?: number;
  pinnedCount?: number;
  pinned: boolean;
  scrollLeft: number;
  canScrollRight?: boolean;
  identity?: TableHeaderIdentity;
  onChange(update: (next: TableSettings) => void): void;
  onResize(column: string, width: number | null): void;
  onResizing(column: string, width: number, guideX?: number | null): void;
}

export function FamilyTableHeader({
  columns,
  settings,
  unsorted = false,
  treeLane = false,
  editableColumns = true,
  inset = 0,
  rankWidth = 0,
  pinnedCount = 1,
  pinned,
  scrollLeft,
  canScrollRight = false,
  identity,
  onChange,
  onResize,
  onResizing,
}: FamilyTableHeaderProps) {
  const dragResize = useRef<{ key: string; start: number; width: number; current: number; originLeft: number } | null>(null);
  const dragCol = useRef<string | null>(null);
  const [dropTarget, setDropTarget] = useState<{ key: string; side: 'left' | 'right' } | null>(null);
  const headRef = useRef<HTMLDivElement>(null);
  const sort = settings.sort;
  const hasTall = columns.some(col => Boolean(col.unit) || Boolean(col.headerGroup));
  const showPinShadow = pinned && scrollLeft > 0;
  const treeColumn = treeColumnOf(columns);

  const reorderColumns = (fromKey: string, toKey: string, side: 'left' | 'right') => {
    if (fromKey === toKey) return;
    onChange(next => {
      const fromIndex = next.order.indexOf(fromKey);
      if (fromIndex < 0) return;
      next.order.splice(fromIndex, 1);
      const targetIndex = next.order.indexOf(toKey);
      if (targetIndex < 0) {
        next.order.push(fromKey);
        return;
      }
      next.order.splice(side === 'right' ? targetIndex + 1 : targetIndex, 0, fromKey);
    });
  };

  const renderLabel = (item: TableHeaderColumn, active: boolean, end: boolean, index: number) => {
    if (item.cellKind === 'kind') return <span className="sr-only">{item.label}</span>;
    const arrow = active ? (sort.direction === 'descending' ? SORT_ARROW_DOWN : SORT_ARROW_UP) : null;
    if (identity && index === 0) {
      return <>
        <HeaderIdentity identity={identity} />
        {arrow && <span className="tf-sort-mark" aria-hidden="true">{arrow}</span>}
      </>;
    }
    if (item.headerGroup?.kind === 'pair') {
      const { icon, iconMissing } = item.headerGroup;
      return <span className="tf-th-pair">
        <span className="tf-th-group">
          {icon ? <GroupIcon src={icon} /> : iconMissing ? <i className="tf-th-hatch" title={iconMissing} /> : null}
          <span>{item.label}</span>
          {arrow && <span className="tf-sort-mark" aria-hidden="true">{arrow}</span>}
        </span>
        <span className="tf-th-sides">{item.headerGroup.labels.map(label => <span key={label}>{label}</span>)}</span>
      </span>;
    }
    if (item.headerGroup?.kind === 'shortOver') {
      return <span className="tf-th-short-over">
        <span className="tf-th-so-bar">
          <span>Short</span>
          <i className="tf-th-so-tick" aria-hidden="true" />
          <span>Over</span>
        </span>
        {arrow && <span className="tf-sort-mark" aria-hidden="true">{arrow}</span>}
      </span>;
    }
    if (item.headerGroup?.kind === 'parts') {
      return <span className="tf-th-text-block tf-th-parts-block">
        <span className="tf-th-main tf-th-parts">
          {item.headerGroup.parts.map((part, idx, parts) => <span key={idx} className="tf-th-part" title={part.notRecordedReason}>
            <span>{part.label}</span>
            {part.notRecordedReason && <span className="tf-th-hatch" aria-hidden="true" />}
            {parts[idx + 1]?.operator && <span className="tf-th-operator" aria-hidden="true">{parts[idx + 1]!.operator}</span>}
          </span>)}
        </span>
        {item.unit && <span className="tf-th-unit">{item.unit}</span>}
      </span>;
    }
    const textBlock = <span className="tf-th-text-block">
      <span className="tf-th-main">
        {item.notRecordedReason && <span className="tf-th-hatch" aria-hidden="true" />}
        <span>{item.label}</span>
      </span>
      {item.unit && <span className="tf-th-unit">{item.unit}</span>}
    </span>;
    return <>
      {end && arrow && <span className="tf-sort-mark" aria-hidden="true">{arrow}</span>}
      {textBlock}
      {!end && arrow && <span className="tf-sort-mark" aria-hidden="true">{arrow}</span>}
    </>;
  };

  return <div className={`tf-head-row${hasTall ? ' is-tall' : ''}${canScrollRight ? ' has-right-fade' : ''}`}>
    <div ref={headRef} className="tf-head" role="row" style={{ transform: `translateX(${-scrollLeft}px)` }}>
      {rankWidth > 0 && <div
        className={`tf-th tf-rank${pinned ? ' pinned' : ''}`}
        role="columnheader"
        style={{ width: rankWidth, ...(pinned && { transform: `translateX(${scrollLeft}px)` }) }}
      >
        #
      </div>}
      {columns.map((item, index) => {
        const active = !unsorted && sort.column === item.key;
        const end = endAligned(item);
        const isPinned = pinned && index < pinnedCount;
        const isLastPinned = isPinned && index === pinnedCount - 1;
        const canSort = !unsorted && item.sortable !== false && !item.notRecordedReason && item.cellKind !== 'kind';
        const canDrag = editableColumns && !unsorted && index >= pinnedCount;
        const dropClass = dropTarget?.key === item.key ? ` is-drop-${dropTarget.side}` : '';
        const title = identity && index === 0 ? identity.name : (item.notRecordedReason ?? item.hint);
        const lead = index === 0 ? ROW_MARGIN_X : inset;
        const trail = index === columns.length - 1 ? ROW_MARGIN_X : inset;
        const padLeft = CELL_INSET + lead + (treeLane && index === treeColumn ? TREE_CHEVRON_LANE : 0);
        const padRight = CELL_INSET + trail;
        return <div
          key={item.key}
          role="columnheader"
          aria-sort={canSort ? (active ? sort.direction : 'none') : undefined}
          className={`tf-th${end ? ' end' : ''}${active ? ' sorted' : ''}${isPinned ? ' pinned' : ''}${isLastPinned && showPinShadow ? ' pinned-shadow' : ''}${item.notRecordedReason ? ' is-unrecorded' : ''}${dropClass}`}
          style={{
            width: `var(${columnWidthVar(item.key)}, ${item.width}px)`,
            ...(padLeft !== CELL_INSET && { paddingLeft: padLeft }),
            ...(padRight !== CELL_INSET && { paddingRight: padRight }),
            ...(item.headerGroup?.kind === 'pair' && { '--tf-pad-l': `${padLeft}px`, '--tf-pad-r': `${padRight}px` }),
            ...(isPinned && { transform: `translateX(${scrollLeft}px)` }),
          } as CSSProperties}
          draggable={canDrag}
          onDragStart={canDrag ? event => {
            dragCol.current = item.key;
            event.dataTransfer.effectAllowed = 'move';
            event.dataTransfer.setData('text/plain', item.key);
          } : undefined}
          onDragOver={canDrag ? event => {
            if (!dragCol.current || dragCol.current === item.key) return;
            event.preventDefault();
            const box = event.currentTarget.getBoundingClientRect();
            const side = event.clientX < box.left + box.width / 2 ? 'left' : 'right';
            setDropTarget(prev => (prev?.key === item.key && prev.side === side ? prev : { key: item.key, side }));
          } : undefined}
          onDragLeave={canDrag ? () => {
            setDropTarget(prev => (prev?.key === item.key ? null : prev));
          } : undefined}
          onDrop={canDrag ? event => {
            event.preventDefault();
            const from = dragCol.current;
            const side = dropTarget?.key === item.key ? dropTarget.side : 'left';
            dragCol.current = null;
            setDropTarget(null);
            if (from) reorderColumns(from, item.key, side);
          } : undefined}
          onDragEnd={() => {
            dragCol.current = null;
            setDropTarget(null);
          }}
        >
          {canSort ? <button
            type="button"
            className="tf-th-label"
            title={title}
            onClick={() => onChange(next => {
              next.sort = active
                ? { column: item.key, direction: sort.direction === 'descending' ? 'ascending' : 'descending' }
                : { column: item.key, direction: item.text ? 'ascending' : 'descending' };
            })}
          >
            {canDrag && GRIP_DOTS}
            {renderLabel(item, active, end, index)}
          </button> : <span className={`tf-th-static${item.headerGroup ? ' tf-th-static-fill' : ''}`} title={title}>{renderLabel(item, false, end, index)}</span>}
          {editableColumns && <span
            className="tf-resize"
            title="Drag to resize. Double-click to fit."
            onPointerDown={event => {
              event.preventDefault();
              event.stopPropagation();
              event.currentTarget.setPointerCapture(event.pointerId);
              const th = event.currentTarget.parentElement as HTMLElement;
              const width = th.offsetWidth;
              const headBox = headRef.current?.parentElement?.getBoundingClientRect();
              const thBox = th.getBoundingClientRect();
              const originLeft = headBox ? thBox.left - headBox.left : 0;
              dragResize.current = { key: item.key, start: event.clientX, width, current: width, originLeft };
              onResizing(item.key, width, originLeft + width);
            }}
            onPointerMove={event => {
              const now = dragResize.current;
              if (!now) return;
              now.current = Math.max(40, Math.min(1000, Math.round(now.width + event.clientX - now.start)));
              onResizing(now.key, now.current, now.originLeft + now.current);
            }}
            onPointerUp={() => {
              const now = dragResize.current;
              dragResize.current = null;
              onResizing(item.key, now?.current ?? item.width, null);
              if (now && now.current !== now.width) onResize(now.key, Math.max(40, now.current - lead - trail));
            }}
            onPointerCancel={() => {
              dragResize.current = null;
              onResizing(item.key, item.width, null);
            }}
            onDoubleClick={() => {
              onResizing(item.key, item.width, null);
              onResize(item.key, null);
            }}
          />}
        </div>;
      })}
    </div>
  </div>;
}
