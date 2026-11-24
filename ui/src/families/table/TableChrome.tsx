import { useCallback, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import type { FamilyChromeProps } from '../host/FamilyHost.tsx';
import { syncTableEdges } from './edges.ts';
import type { TableChromeState } from './hostAdapter.ts';
import { buildColumnLayouts, layoutFamilyTable } from './layout.ts';
import type { TableColumnSpec } from './model.ts';
import { updateTableSettings, type TableSettings } from './settings.ts';
import { columnWidthVar, DISPLAY_LABELS, FamilyTableHeader, type TableHeaderColumn } from './TableHeader.tsx';
import './tableFamily.css';

const HEADER_FALLBACK_WIDTH = 120;
const NO_COLUMNS: readonly TableColumnSpec[] = [];
const NO_WIDTHS: Readonly<Record<string, number>> = {};

interface ChromeMeta {
  readonly hideHeader?: boolean;
  readonly unsorted?: boolean;
  readonly rankShown?: boolean;
  readonly rankWidth?: number;
  readonly columns?: readonly TableColumnSpec[];
}

interface ChromeWidths {
  readonly widths: Readonly<Record<string, number>>;
  readonly visible: readonly string[];
  readonly hidden: readonly string[];
  readonly scrollable: boolean;
  readonly pinnedCount?: number;
  readonly moreWidth: number;
}

export default function TableChrome({ state, settings, onSettings, editable, viewportWidth, bodyBlocked, announce, targets }: FamilyChromeProps<TableSettings>) {
  const chrome = state as TableChromeState | undefined;
  const meta = (chrome?.meta ?? {}) as ChromeMeta;
  const measured = chrome?.widths as ChromeWidths | undefined;
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
          totalWidth: rank + measured.moreWidth + visible.reduce((sum, col) => sum + col.width, 0),
          scrollable: measured.scrollable,
          pinnedWidth: rank + visible.slice(0, pinnedCount).reduce((sum, col) => sum + col.width, 0),
          pinnedCount,
        };
      }
    }
    const protectedKeys = new Set<string>();
    if (!meta.unsorted && settings.sort.column) protectedKeys.add(settings.sort.column);
    return layoutFamilyTable(base, Math.max(1, viewportWidth || 320), 'scroll', {
      rankShown: meta.rankShown === true,
      rankWidth: meta.rankWidth || 36,
      protectedKeys,
    });
  }, [columns, settings, measured, resizing, meta.unsorted, meta.rankShown, meta.rankWidth, viewportWidth]);

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
      headerGroup: spec?.headerGroup,
      displays: spec && spec.offered.length > 1 ? spec.offered.map(display => [display, DISPLAY_LABELS[display]] as const) : [],
    };
  }), [layout, specMap]);

  const widthVars = useMemo(() => {
    const vars: Record<string, string> = {};
    for (const [key, px] of Object.entries(measured?.widths ?? NO_WIDTHS)) vars[columnWidthVar(key)] = `${px}px`;
    return vars as CSSProperties;
  }, [measured]);

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
  const canScrollRight = layout.scrollable && scrollLeft < Math.max(0, layout.totalWidth - (viewportWidth || 320)) - 1;

  return (
    <>
      {!meta.hideHeader ? (
        <div className="tf-head-clip" style={bodyBlocked ? { ...widthVars, display: 'none' } : widthVars}>
          <FamilyTableHeader
            columns={headerColumns}
            allColumns={columns}
            hiddenColumns={[]}
            settings={settings}
            unsorted={meta.unsorted}
            editableColumns={editable}
            rankWidth={meta.rankShown ? meta.rankWidth : 0}
            pinnedCount={layout.pinnedCount}
            pinned={layout.scrollable}
            scrollLeft={scrollLeft}
            canScrollRight={canScrollRight}
            onChange={changeSettings}
            onResizing={(key, width, guideX) => setResizing(guideX === null ? null : { key, width, guideX })}
            onResize={(key, width) => {
              setResizing(null);
              changeSettings(draft => { if (draft.columns[key]) draft.columns[key]!.width = width; });
            }}
          />
        </div>
      ) : null}
      {targets.bodyWrap ? createPortal(
        <>
          {resizing?.guideX != null ? <div className="tf-resize-guide" style={{ left: resizing.guideX }} aria-hidden="true" /> : null}
          <div
            ref={edgesRef}
            className="tf-edges"
            data-right={canScrollRight ? 'true' : undefined}
            data-pinned={layout.scrollable && scrollLeft > 0 && layout.pinnedWidth > 0 ? 'true' : undefined}
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
