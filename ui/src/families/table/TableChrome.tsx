import { useCallback, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import { Icon } from '../../components/Icons';
import { createPortal } from 'react-dom';
import type { FamilyChromeProps } from '../host/FamilyHost.tsx';
import { syncTableEdges } from './edges.ts';
import type { TableChromeState } from './hostAdapter.ts';
import { buildColumnLayouts, layoutFamilyTable } from './layout.ts';
import LogKindFilter from './LogKindFilter.tsx';
import type { TableColumnSpec, TableModel } from './model.ts';
import { updateTableSettings, type TableSettings } from './settings.ts';
import { columnWidthVar, DISPLAY_LABELS, FamilyTableHeader, type TableHeaderColumn, type TableHeaderIdentity } from './TableHeader.tsx';
import './tableFamily.css';

const HEADER_FALLBACK_WIDTH = 120;
const NO_COLUMNS: readonly TableColumnSpec[] = [];
const NO_WIDTHS: Readonly<Record<string, number>> = {};

interface ChromeMeta {
  readonly subjectSummary?: TableModel['subjectSummary'];
  readonly kindFilter?: TableModel['kindFilter'];
  readonly hideHeader?: boolean;
  readonly unsorted?: boolean;
  readonly rankShown?: boolean;
  readonly rankWidth?: number;
  readonly hasTree?: boolean;
  readonly columns?: readonly TableColumnSpec[];
}

interface ChromePanes {
  readonly width: number;
  readonly gutter: number;
  readonly xs: readonly number[];
  readonly tableWidth: number;
  readonly titles: readonly (TableHeaderIdentity & { readonly key: string })[];
}

interface ChromeWidths {
  readonly widths: Readonly<Record<string, number>>;
  readonly visible: readonly string[];
  readonly totals?: readonly string[];
  readonly scrollable: boolean;
  readonly pinnedCount?: number;
}

export default function TableChrome({ state, settings, onSettings, editable, viewportWidth, bodyBlocked, announce, targets, shownHour, filterSlot, setFilterOption }: FamilyChromeProps<TableSettings>) {
  const chrome = state as TableChromeState | undefined;
  const meta = (chrome?.meta ?? {}) as ChromeMeta;
  const measured = chrome?.widths as ChromeWidths | undefined;
  const panes = chrome?.panes as ChromePanes | undefined;
  const columns = meta.columns ?? NO_COLUMNS;
  const [scrollLeft, setScrollLeft] = useState(0);
  const [resizing, setResizing] = useState<{ key: string; width: number; guideX?: number | null } | null>(null);
  const edgesRef = useRef<HTMLDivElement>(null);

  const changeSettings = useCallback((update: (next: TableSettings) => void) => {
    if (!onSettings || !settings) return;
    const next = updateTableSettings(settings, update);
    if (next.sort.column !== settings.sort.column || next.sort.direction !== settings.sort.direction) {
      const col = columns.find(c => c.key === next.sort.column);
      announce(`Sorted by ${col?.label ?? next.sort.column}, ${next.sort.direction === 'descending' ? 'highest first' : 'lowest first'}.`);
    }
    onSettings(next);
  }, [settings, onSettings, columns, announce]);

  const layout = useMemo(() => {
    if (!settings) return null;
    const widths = measured?.widths ?? NO_WIDTHS;
    const base = buildColumnLayouts(columns, settings, widths).map(col => ({
      ...col,
      width: resizing?.key === col.key ? resizing.width : (widths[col.key] ?? (col.width || HEADER_FALLBACK_WIDTH)),
    }));
    if (measured) {
      const byKey = new Map(base.map(col => [col.key, col]));
      const visible = measured.visible.map(key => byKey.get(key)).filter((col): col is NonNullable<typeof col> => col !== undefined);
      if (visible.length === measured.visible.length) {
        const rank = meta.rankShown ? (meta.rankWidth || 36) : 0;
        const pinnedCount = measured.pinnedCount || 1;
        return {
          columns: visible,
          totalWidth: rank + visible.reduce((sum, col) => sum + col.width, 0),
          scrollable: measured.scrollable,
          pinnedWidth: rank + visible.slice(0, pinnedCount).reduce((sum, col) => sum + col.width, 0),
          pinnedCount,
        };
      }
    }
    return layoutFamilyTable(base, Math.max(1, panes?.width ?? (viewportWidth || 320)), {
      rankShown: meta.rankShown === true,
      rankWidth: meta.rankWidth || 36,
    });
  }, [columns, settings, measured, resizing, meta.rankShown, meta.rankWidth, viewportWidth, panes?.width]);

  const specMap = useMemo(() => new Map(columns.map(col => [col.key, col])), [columns]);
  const headerColumns = useMemo<TableHeaderColumn[]>(() => (layout?.columns ?? []).map(col => {
    const spec = specMap.get(col.key);
    return {
      ...col,
      unit: spec?.unit,
      text: spec?.text,
      hideable: spec?.hideable,
      sortable: spec?.sortable,
      hint: spec?.hint,
      notRecordedReason: spec?.notRecordedReason,
      headerGroup: measured?.totals?.includes(col.key) ? undefined : col.headerGroup,
      cellKind: spec?.cellKind,
      displays: spec && spec.offered.length > 1 ? spec.offered.map(display => [display, DISPLAY_LABELS[display]] as const) : [],
    };
  }), [layout, specMap, measured]);

  const widthVars = useMemo(() => {
    const vars: Record<string, string> = {};
    for (const [key, px] of Object.entries(measured?.widths ?? NO_WIDTHS)) vars[columnWidthVar(key)] = `${px}px`;
    return vars as CSSProperties;
  }, [measured]);

  const paned = panes !== undefined;
  useLayoutEffect(() => {
    const body = targets.body;
    if (!body || !paned) return;
    body.dataset.tfPanes = 'true';
    return () => { delete body.dataset.tfPanes; };
  }, [targets.body, paned]);

  useLayoutEffect(() => {
    const body = targets.body;
    const extent = targets.extent;
    if (!body) return;
    let extentHeight = extent?.offsetHeight ?? 0;
    const sync = () => syncTableEdges(edgesRef.current, { width: body.clientWidth, height: body.clientHeight, scrollTop: body.scrollTop }, extentHeight);
    const onScroll = () => {
      setScrollLeft(prev => (prev === body.scrollLeft ? prev : body.scrollLeft));
      sync();
    };
    body.addEventListener('scroll', onScroll, { passive: true });
    const observer = typeof ResizeObserver === 'function'
      ? new ResizeObserver(entries => {
          for (const entry of entries) if (entry.target === extent) extentHeight = entry.contentRect.height;
          sync();
        })
      : null;
    observer?.observe(body);
    if (extent) observer?.observe(extent);
    sync();
    return () => {
      body.removeEventListener('scroll', onScroll);
      observer?.disconnect();
    };
  }, [targets.body, targets.extent]);

  if (!settings || !layout) return null;
  const paneOverflow = panes ? Math.max(0, panes.tableWidth - panes.width) : 0;
  const shownLeft = panes ? Math.min(scrollLeft, paneOverflow) : scrollLeft;
  const canScrollRight = panes
    ? shownLeft < paneOverflow - 1
    : layout.scrollable && scrollLeft < Math.max(0, layout.totalWidth - (viewportWidth || 320)) - 1;
  const header = (identity?: TableHeaderIdentity) => (
    <FamilyTableHeader
      columns={headerColumns}
      allColumns={columns}
      settings={settings}
      unsorted={meta.unsorted}
      treeLane={meta.hasTree === true}
      editableColumns={editable}
      rankWidth={meta.rankShown ? meta.rankWidth : 0}
      pinnedCount={layout.pinnedCount}
      pinned={layout.scrollable}
      scrollLeft={shownLeft}
      canScrollRight={canScrollRight}
      {...(identity ? { identity } : {})}
      onChange={changeSettings}
      onResizing={(key, width, guideX) => setResizing(guideX === null ? null : { key, width, guideX })}
      onResize={(key, width) => {
        setResizing(null);
        changeSettings(draft => { if (draft.columns[key]) draft.columns[key]!.width = width; });
      }}
    />
  );

  return (
    <>
      {meta.subjectSummary && <div className="tf-subject-summary" data-compact={viewportWidth < 440 || undefined}>
        <span className="tf-subject-art">{meta.subjectSummary.icon ? <img src={meta.subjectSummary.icon} alt="" /> : <Icon name="helmet" />}</span>
        <div className="tf-subject-name"><strong>{meta.subjectSummary.name}</strong>
          <div>{meta.subjectSummary.flag && <img src={meta.subjectSummary.flag} alt={meta.subjectSummary.country} />}
            <span className="tf-subject-detail">{meta.subjectSummary.country} &middot; {meta.subjectSummary.detail}</span>
            {meta.subjectSummary.insignia && <img className="tf-level-icon" src={meta.subjectSummary.insignia} alt="" />}<span>{meta.subjectSummary.level}</span>
          </div>
        </div>
        <dl>{meta.subjectSummary.metrics.map(metric => <div key={metric.label}><dt>{metric.label}</dt><dd>
          {metric.icons?.length ? metric.icons.map((url, index) => <img key={index} src={url} alt="" />) : null}<span className={metric.icons?.length === 1 && metric.value === '1' ? 'sr-only' : undefined}>{metric.value}</span>
        </dd></div>)}</dl>
      </div>}
      {meta.kindFilter && setFilterOption ? <LogKindFilter
        slot={filterSlot}
        shownHour={shownHour}
        compact={viewportWidth < 440}
        single={meta.kindFilter.single}
        allLabel={meta.kindFilter.allLabel}
        noun={meta.kindFilter.noun}
        items={meta.kindFilter.items}
        onSelectAll={() => setFilterOption(meta.kindFilter!.key, '')}
        onToggle={(token, checked) => {
          const filter = meta.kindFilter!;
          if (filter.single) {
            setFilterOption(filter.key, token);
            return;
          }
          const visible = filter.items.map(item => item.value);
          const selected = new Set(filter.selected.length ? filter.selected : visible);
          selected.delete('-');
          if (checked) selected.add(token); else selected.delete(token);
          setFilterOption(filter.key, visible.every(value => selected.has(value)) ? '' : [...selected].join(' ') || '-');
        }}
      /> : null}
      {!meta.hideHeader && panes ? (
        <div className="tf-head-clip tf-pane-heads" style={bodyBlocked ? { ...widthVars, display: 'none' } : { ...widthVars, columnGap: panes.gutter }}>
          {panes.titles.map(title => <div key={title.key} className="tf-pane-head" style={{ width: panes.width }}>{header(title)}</div>)}
          {panes.xs.slice(1).map(x => <i key={x} className="tf-pane-rule" style={{ left: Math.round(x - panes.gutter / 2) }} aria-hidden="true" />)}
        </div>
      ) : !meta.hideHeader ? (
        <div className="tf-head-clip" style={bodyBlocked ? { ...widthVars, display: 'none' } : widthVars}>
          {header()}
        </div>
      ) : null}
      {targets.bodyWrap ? createPortal(
        <>
          {resizing?.guideX != null ? <div className="tf-resize-guide" style={{ left: resizing.guideX }} aria-hidden="true" /> : null}
          <div
            ref={edgesRef}
            className="tf-edges"
            data-right={canScrollRight && !panes ? 'true' : undefined}
            data-pinned={!panes && layout.scrollable && scrollLeft > 0 && layout.pinnedWidth > 0 ? 'true' : undefined}
            style={{ '--tf-pinned-width': `${layout.pinnedWidth}px` } as CSSProperties}
            aria-hidden="true"
          >
            <i className="tf-edge-right" />
            <i className="tf-edge-bottom" />
            <i className="tf-edge-pinned" />
          </div>
        </>,
        targets.bodyWrap,
      ) : null}
      {targets.mirrorHead ? createPortal(
        <tr>
          {meta.rankShown ? <th scope="col">Rank</th> : null}
          {layout.columns.map(col => <th key={col.key} scope="col">{col.label}</th>)}
        </tr>,
        targets.mirrorHead,
      ) : null}
    </>
  );
}
