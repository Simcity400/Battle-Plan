import { displayedHour } from '../boundedView.ts';
import type { FamilyHostAdapter, ParsedFamilyHit } from '../host/adapter.ts';
import type { LaneActivation } from './lanesDeclaration.ts';
import { layoutHeatLanes } from './lanesLayout.ts';
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
      columns: ext.columns.length,
      cellSize: ext.cellSize,
      ...(ext.cellWidth !== undefined ? { cellWidth: ext.cellWidth } : {}),
      gridLeft: ext.gridLeft,
      gridTop: ext.gridTop,
      rowLabel: r => ext.rows[r]?.label ?? '',
      columnLabel: c => ext.columns[c]?.label ?? '',
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
  deriveAuditRows: model => deriveMatrixAudit(model),
  auditOrdered: false,
  buildBinding: ctx => ({
    family: 'heat-matrix',
    kind: ctx.kind,
    view: ctx.view,
    options: ctx.options,
    sources: ctx.sources,
    ...(ctx.staticPanels ? { staticPanels: ctx.staticPanels } : {}),
    ...(ctx.staticAnswers ? { staticAnswers: ctx.staticAnswers } : {}),
    width: ctx.viewport.width,
    height: ctx.viewport.height,
    scrollLeft: ctx.viewport.scrollLeft,
    scrollTop: ctx.viewport.scrollTop,
    theme: ctx.theme,
    ...(ctx.identitySnapshot ? { identitySnapshot: ctx.identitySnapshot } : {}),
    focusedCell: ctx.focusedKey,
    pauseFlush: ctx.pauseFlush,
    ...(ctx.audit ? { audit: true, auditId: ctx.auditId } : {}),
  }),
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
  getRowStride: fields => {
    const ext = fields.extent as LanesExtentField | undefined;
    return ext?.rowStride ?? 30;
  },
  getPageStep: (fields, viewport) => {
    const ext = fields.extent as LanesExtentField | undefined;
    const stride = Math.max(1, ext?.rowStride ?? 30);
    return Math.max(1, Math.floor(Math.max(1, ext?.plotHeight ?? viewport.height) / stride));
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
  buildBinding: ctx => ({
    family: 'heat-lanes',
    kind: ctx.kind,
    view: ctx.view,
    options: ctx.options,
    sources: ctx.sources,
    ...(ctx.staticPanels ? { staticPanels: ctx.staticPanels } : {}),
    ...(ctx.staticAnswers ? { staticAnswers: ctx.staticAnswers } : {}),
    width: ctx.viewport.width,
    height: ctx.viewport.height,
    scrollTop: ctx.viewport.scrollTop,
    theme: ctx.theme,
    ...(ctx.identitySnapshot ? { identitySnapshot: ctx.identitySnapshot } : {}),
    selectedRowId: ctx.selectedKey,
    focusedRowId: ctx.focusedKey,
    pauseFlush: ctx.pauseFlush,
    ...(ctx.audit ? { audit: true, auditId: ctx.auditId } : {}),
  }),
};
