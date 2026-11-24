import { displayedHour } from '../boundedView.ts';
import type { FamilyHostAdapter, ParsedFamilyHit } from '../host/adapter.ts';
import type { LaneActivation } from './lanesDeclaration.ts';
import { layoutHeatLanes } from './lanesLayout.ts';
import { matrixWindow } from './matrixLayout.ts';
import type { FamilyHeatLanesBinding } from './lanesLaneView.ts';
import {
  deriveLanesAudit,
  parseLaneInspectKey,
  type HeatLaneIdentity,
  type HeatLanesModel,
} from './lanesModel.ts';
import type { MatrixActivation } from './matrixDeclaration.ts';
import type { FamilyHeatMatrixBinding } from './matrixLaneView.ts';
import {
  deriveMatrixAudit,
  matrixCellKey,
  parseMatrixCellKey,
  type HeatMatrixModel,
  type MatrixAxisEntity,
} from './matrixModel.ts';

interface MatrixExtentField {
  readonly totalWidth: number;
  readonly totalHeight: number;
  readonly cellSize: number;
  readonly cellWidth?: number;
  readonly gridLeft: number;
  readonly gridTop: number;
  readonly marginWidth: number;
  readonly margin: string | null;
  readonly rows: readonly MatrixAxisEntity[];
  readonly columns: readonly MatrixAxisEntity[];
}

interface LanesExtentField {
  readonly totalHeight: number;
  readonly rowStride: number;
  readonly plotTop: number;
  readonly plotHeight: number;
  readonly rows: readonly HeatLaneIdentity[];
}

export function parseHeatMatrixHit(hitKey: string | null): ParsedFamilyHit | null {
  if (!hitKey || !hitKey.startsWith('cell:')) return null;
  const key = hitKey.slice(5);
  return key ? { key, subject: key, toggle: false, inspectToken: hitKey } : null;
}

export function parseHeatLanesHit(hitKey: string | null): ParsedFamilyHit | null {
  if (!hitKey || !hitKey.startsWith('lane:')) return null;
  const key = hitKey.slice(5);
  if (!key) return null;
  const parsed = parseLaneInspectKey(key);
  return {
    key,
    subject: key,
    ...(parsed?.hour != null ? { hour: parsed.hour } : {}),
    toggle: false,
    inspectToken: hitKey,
  };
}

export const heatMatrixHostAdapter: FamilyHostAdapter<HeatMatrixModel, unknown, FamilyHeatMatrixBinding> = {
  family: 'heat-matrix',
  scroll: 'both',
  navigation: 'grid',
  mirror: 'table',
  auditOwnerId: -2,
  defaultSize: [320, 240],
  parseHit: parseHeatMatrixHit,
  inspectToken: key => `cell:${key}`,
  getScrollExtent: fields => {
    const ext = fields.extent as MatrixExtentField | undefined;
    return ext ? { width: ext.totalWidth, height: ext.totalHeight } : null;
  },
  getGridMetrics: fields => {
    const ext = fields.extent as MatrixExtentField | undefined;
    if (!ext) return null;
    return {
      rows: ext.rows.length,
      columns: ext.columns.length + (ext.margin !== null ? 1 : 0),
      cellSize: ext.cellSize,
      ...(ext.cellWidth !== undefined ? { cellWidth: ext.cellWidth } : {}),
      gridLeft: ext.gridLeft,
      gridTop: ext.gridTop,
      rowLabel: r => ext.rows[r]?.label ?? '',
      columnLabel: c => ext.columns[c]?.label ?? (c === ext.columns.length ? ext.margin ?? '' : ''),
    };
  },
  parseGridKey: parseMatrixCellKey,
  formatGridKey: matrixCellKey,
  resolveHitAction: (hit, installed, fields) => {
    const parsed = parseMatrixCellKey(hit.key);
    const ext = fields.extent as MatrixExtentField | undefined;
    if (!parsed || !ext) return null;
    const row = ext.rows[parsed.row];
    const column = ext.columns[parsed.column];
    if (!row || !column) return null;
    const activation: MatrixActivation = {
      row,
      column,
      rowIndex: parsed.row,
      columnIndex: parsed.column,
      hour: installed.hour,
    };
    return {
      installedId: installed.id,
      owner: installed.owner,
      subject: hit.key,
      hour: installed.hour,
      label: `${row.label} to ${column.label}`,
      meta: activation,
    };
  },
  deriveAuditRows: (model, _hour, { viewport, fields }) => {
    const ext = fields.extent as MatrixExtentField | undefined;
    if (!ext) return [];
    const geometry = {
      width: viewport.width,
      height: viewport.height,
      gridLeft: ext.gridLeft,
      gridTop: ext.gridTop,
      marginWidth: ext.marginWidth,
      cellWidth: ext.cellWidth ?? ext.cellSize,
      cellHeight: ext.cellSize,
    };
    return deriveMatrixAudit(model, matrixWindow(geometry, viewport.scrollLeft, viewport.scrollTop, model.rows.length, model.columns.length));
  },
  auditOrdered: false,
};

export const heatLanesHostAdapter: FamilyHostAdapter<HeatLanesModel, unknown, FamilyHeatLanesBinding> = {
  family: 'heat-lanes',
  pointerMode: 'continuous',
  scroll: 'vertical',
  navigation: 'rows',
  mirror: 'table',
  auditOwnerId: -3,
  defaultSize: [360, 220],
  parseHit: parseHeatLanesHit,
  inspectToken: key => `lane:${key}`,
  getScrollExtent: fields => {
    const ext = fields.extent as LanesExtentField | undefined;
    return ext ? { height: ext.totalHeight } : null;
  },
  resolveHitAction: (hit, installed, fields) => {
    const ext = fields.extent as LanesExtentField | undefined;
    if (!ext) return null;
    const parsed = parseLaneInspectKey(hit.key);
    let rowIndex = parsed?.rowIndex ?? -1;
    if (rowIndex < 0) {
      rowIndex = ext.rows.findIndex(r => r.id === hit.key);
    }
    const row = rowIndex >= 0 ? ext.rows[rowIndex] : undefined;
    if (!row) return null;
    const hour = displayedHour(parsed?.hour ?? hit.hour ?? installed.hour);
    const activation: LaneActivation = { row, rowIndex, hour };
    return {
      installedId: installed.id,
      owner: installed.owner,
      subject: row.id,
      hour,
      label: row.label,
      meta: activation,
    };
  },
  deriveAuditRows: (model, _hour, { viewport }) => {
    const layout = layoutHeatLanes({
      width: viewport.width,
      height: viewport.height,
      scrollTop: viewport.scrollTop,
      firstHour: model.firstHour,
      lastHour: model.lastHour,
      displayedHour: model.hour,
      rows: model.rows,
      legend: model.legend,
      requestedReadoutWidth: model.readoutWidth,
      requestedRowHeight: model.rowHeight,
    });
    return deriveLanesAudit(model, { firstRow: layout.firstRow, endRow: layout.endRow });
  },
  auditOrdered: true,
};
