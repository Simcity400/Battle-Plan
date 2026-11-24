import type { AuditRow } from '../../draw/notifications';
import type { ArtHandle, BoundedReadView, FamilyProjection, MirrorModel, MirrorRow } from '../contract.ts';
import type { FamilyTooltipPayload } from '../host/tooltips.ts';

export interface MatrixGroup {
  readonly id: string;
  readonly label: string;
  readonly color?: string;
}

export interface MatrixAxisEntity {
  readonly id: string;
  readonly label: string;
  readonly shortLabel?: string;
  readonly nameMissing?: string;
  readonly tag?: string;
  readonly flag?: ArtHandle | null;
  readonly flagPending?: boolean;
  readonly group?: MatrixGroup | null;
  readonly color?: string;
  readonly trailing?: string;
  readonly meta?: unknown;
}

export type MatrixValueMark = 'fill' | 'square' | 'ring';

export type MatrixCell =
  | {
      readonly kind: 'value';
      readonly color?: string;
      readonly intensity?: number;
      readonly mark?: MatrixValueMark;
      readonly fraction?: number;
      readonly text?: string;
      readonly value?: number | bigint;
      readonly tooltip?: FamilyTooltipPayload;
      readonly audit?: string;
      readonly selectable?: boolean;
    }
  | {
      readonly kind: 'empty';
      readonly mark?: 'dot';
      readonly label?: string;
      readonly tooltip?: FamilyTooltipPayload;
      readonly audit?: string;
      readonly selectable?: boolean;
    }
  | {
      readonly kind: 'diagonal';
      readonly label?: string;
      readonly text?: string;
      readonly outlined?: boolean;
      readonly tooltip?: FamilyTooltipPayload;
      readonly selectable?: boolean;
    }
  | {
      readonly kind: 'missing';
      readonly reason: string;
      readonly tooltip?: FamilyTooltipPayload;
      readonly audit?: string;
      readonly selectable?: boolean;
    };

export interface MatrixLegend {
  readonly entries?: readonly { readonly id: string; readonly label: string; readonly color: string; readonly hatched?: boolean; readonly outlined?: boolean }[];
  readonly gradient?: {
    readonly lowLabel: string;
    readonly highLabel: string;
    readonly stops: readonly string[];
  };
  readonly rightNote?: string;
}

export type MatrixRamp = 'accent' | 'red' | 'amber';

export interface MatrixCells {
  readonly present: ReadonlyMap<number, MatrixCell>;
  readonly absent: MatrixCell | ((rowIndex: number) => MatrixCell);
  readonly self?: MatrixCell | ((rowIndex: number) => MatrixCell);
}

const SELF: MatrixCell = { kind: 'diagonal' };

export const NO_CELLS: MatrixCells = { present: new Map(), absent: { kind: 'empty' } };

export function matrixPairIndex(columnCount: number, rowIndex: number, columnIndex: number): number {
  return rowIndex * columnCount + columnIndex;
}

export function matrixCellAt(model: Pick<HeatMatrixModel, 'rows' | 'columns' | 'cells'>, rowIndex: number, columnIndex: number): MatrixCell {
  const present = model.cells.present.get(rowIndex * model.columns.length + columnIndex);
  if (present) return present;
  if (model.rows[rowIndex]?.id === model.columns[columnIndex]?.id) {
    const self = model.cells.self ?? SELF;
    return typeof self === 'function' ? self(rowIndex) : self;
  }
  const absent = model.cells.absent;
  return typeof absent === 'function' ? absent(rowIndex) : absent;
}

export type MatrixMarginCell =
  | { readonly kind: 'value'; readonly fraction: number; readonly text: string; readonly exact?: string; readonly color?: string }
  | { readonly kind: 'missing'; readonly reason: string }
  | { readonly kind: 'none'; readonly label: string };

export interface MatrixMargin {
  readonly header: string;
  readonly cell: (rowIndex: number) => MatrixMarginCell;
}

export interface HeatMatrixModel {
  readonly hour: number;
  readonly corner?: string;
  readonly summaryText?: string;
  readonly rows: readonly MatrixAxisEntity[];
  readonly columns: readonly MatrixAxisEntity[];
  readonly cells: MatrixCells;
  readonly shape?: 'square' | 'wide';
  readonly ramp?: MatrixRamp;
  readonly margin?: MatrixMargin;
  readonly legend?: MatrixLegend;
  readonly tooltip?: (row: MatrixAxisEntity, column: MatrixAxisEntity, cell: MatrixCell, rowIndex: number, columnIndex: number) => FamilyTooltipPayload | null;
  readonly empty?: string;
  readonly emptyDetail?: string;
  readonly emptyKind?: 'empty' | 'unavailable';
  readonly refused?: string;
  readonly notes?: readonly string[];
  readonly warnings?: readonly string[];
}

export interface HeatMatrixProjection<TOptions = Record<string, never>> extends FamilyProjection<HeatMatrixModel, TOptions> {
  readonly family: 'heat-matrix';
  project(readView: BoundedReadView, options: TOptions): HeatMatrixModel;
  nextChange?(readView: BoundedReadView, options: TOptions): number;
}

export interface MatrixGroupSpan {
  readonly id: string;
  readonly label: string;
  readonly color?: string;
  readonly first: number;
  readonly count: number;
}

export function matrixGroupSpans(entities: readonly MatrixAxisEntity[]): readonly MatrixGroupSpan[] {
  const spans: MatrixGroupSpan[] = [];
  for (let i = 0; i < entities.length; i++) {
    const group = entities[i]!.group;
    if (!group) continue;
    const prev = spans.at(-1);
    if (prev && prev.id === group.id && prev.first + prev.count === i) {
      spans[spans.length - 1] = { ...prev, count: prev.count + 1 };
    } else {
      spans.push({ id: group.id, label: group.label, ...(group.color ? { color: group.color } : {}), first: i, count: 1 });
    }
  }
  return spans;
}

export function matrixCellKey(rowIndex: number, columnIndex: number): string {
  return `${rowIndex}:${columnIndex}`;
}

export function parseMatrixCellKey(key: string | null): { readonly row: number; readonly column: number } | null {
  if (!key) return null;
  const sep = key.indexOf(':');
  if (sep <= 0) return null;
  const row = Number(key.slice(0, sep));
  const column = Number(key.slice(sep + 1));
  if (!Number.isInteger(row) || !Number.isInteger(column) || row < 0 || column < 0) return null;
  return { row, column };
}

export function deriveMatrixTooltip(
  model: HeatMatrixModel,
  rowIndex: number,
  columnIndex: number,
): FamilyTooltipPayload | null {
  const row = model.rows[rowIndex];
  if (row && model.margin && columnIndex === model.columns.length) return marginTooltip(model, model.margin, row, rowIndex);
  const col = model.columns[columnIndex];
  if (!row || !col) return null;
  const cell = matrixCellAt(model, rowIndex, columnIndex);
  if (model.tooltip) {
    const built = model.tooltip(row, col, cell, rowIndex, columnIndex);
    if (built) return built;
  }
  if (cell.tooltip) return cell.tooltip;
  const title = `${row.label} → ${col.label}`;
  if (cell.kind === 'diagonal') return null;
  if (cell.kind === 'missing') {
    return { title, hour: model.hour, missingReason: cell.reason };
  }
  if (cell.kind === 'empty') {
    return {
      title,
      hour: model.hour,
      items: [{ label: cell.label ?? 'None', tone: 'muted' }],
    };
  }
  return {
    title,
    hour: model.hour,
    ...(cell.text !== undefined ? { exactValue: cell.text } : {}),
  };
}

function marginTooltip(model: HeatMatrixModel, margin: MatrixMargin, row: MatrixAxisEntity, rowIndex: number): FamilyTooltipPayload {
  const item = margin.cell(rowIndex);
  if (item.kind === 'missing') return { title: row.label, hour: model.hour, missingReason: item.reason };
  if (item.kind === 'none') return { title: row.label, hour: model.hour, items: [{ label: margin.header, value: item.label }] };
  return { title: row.label, hour: model.hour, items: [{ label: margin.header, value: item.exact ?? item.text }] };
}

export function marginAuditValue(item: MatrixMarginCell): string {
  if (item.kind === 'missing') return `Not recorded (${item.reason})`;
  if (item.kind === 'none') return item.label;
  return item.exact ?? item.text;
}

export function cellAuditValue(cell: MatrixCell): string {
  if (cell.kind === 'diagonal') return cell.text ?? '—';
  if (cell.kind === 'missing') return cell.audit ?? `Not recorded (${cell.reason})`;
  if (cell.kind === 'empty') return cell.audit ?? cell.label ?? '0';
  return cell.audit ?? cell.text ?? (cell.value !== undefined ? String(cell.value) : '');
}

export function deriveMatrixAudit(
  model: HeatMatrixModel,
  window?: { readonly firstRow: number; readonly endRow: number; readonly firstCol: number; readonly endCol: number },
): readonly AuditRow[] {
  const out: AuditRow[] = [];
  const push = (r: number, c: number, cell: MatrixCell) => {
    if (cell.kind === 'diagonal' && cell.text === undefined) return;
    const row = model.rows[r]!;
    const col = model.columns[c]!;
    out.push({ key: `${row.id}->${col.id}`, row: `${row.label} → ${col.label}`, value: cellAuditValue(cell) });
  };
  if (!window) {
    const width = model.columns.length;
    for (const [index, cell] of model.cells.present) push(Math.floor(index / width), index % width, cell);
    const absent = model.cells.absent;
    if (typeof absent === 'function') {
      for (let r = 0; r < model.rows.length; r++) out.push({ key: `${model.rows[r]!.id}->rest`, row: `${model.rows[r]!.label} → every other column`, value: cellAuditValue(absent(r)) });
    } else if (model.cells.present.size < model.rows.length * model.columns.length) {
      out.push({ key: 'rest', row: 'Every other cell', value: cellAuditValue(absent) });
    }
    if (model.margin) {
      for (let r = 0; r < model.rows.length; r++) {
        out.push({ key: `${model.rows[r]!.id}->margin`, row: `${model.rows[r]!.label} · ${model.margin.header}`, value: marginAuditValue(model.margin.cell(r)) });
      }
    }
    return out;
  }
  const firstRow = Math.max(0, window.firstRow);
  const endRow = Math.min(model.rows.length, window.endRow);
  const firstCol = Math.max(0, window.firstCol);
  const endCol = Math.min(model.columns.length, window.endCol);
  for (let r = firstRow; r < endRow; r++) {
    for (let c = firstCol; c < endCol; c++) push(r, c, matrixCellAt(model, r, c));
    if (model.margin) {
      out.push({ key: `${model.rows[r]!.id}->margin`, row: `${model.rows[r]!.label} · ${model.margin.header}`, value: marginAuditValue(model.margin.cell(r)) });
    }
  }
  return out;
}

export function deriveMatrixMirror(
  model: HeatMatrixModel,
  firstRow: number,
  endRow: number,
  firstCol: number,
  endCol: number,
): MirrorModel {
  const rows: MirrorRow[] = [];
  const r0 = Math.max(0, firstRow);
  const r1 = Math.min(model.rows.length, endRow);
  const c0 = Math.max(0, firstCol);
  const c1 = Math.min(model.columns.length, endCol);
  for (let r = r0; r < r1; r++) {
    const entity = model.rows[r]!;
    const cells = [{ column: 'Country', spoken: entity.label }];
    for (let c = c0; c < c1; c++) {
      const col = model.columns[c]!;
      const cell = matrixCellAt(model, r, c);
      cells.push({
        column: col.label,
        spoken: `${col.label}: ${cellAuditValue(cell)}`,
      });
    }
    rows.push({
      key: entity.id,
      name: entity.label,
      rowIndex: r,
      cells,
    });
  }
  return { first: r0, total: model.rows.length, rows };
}
