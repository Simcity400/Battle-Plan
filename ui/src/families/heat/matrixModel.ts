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
  readonly meta?: unknown;
}

export type MatrixCell =
  | {
      readonly kind: 'value';
      readonly color: string;
      readonly text?: string;
      readonly value?: number | bigint;
      readonly tooltip?: FamilyTooltipPayload;
      readonly audit?: string;
      readonly selectable?: boolean;
    }
  | {
      readonly kind: 'empty';
      readonly label?: string;
      readonly tooltip?: FamilyTooltipPayload;
      readonly audit?: string;
      readonly selectable?: boolean;
    }
  | {
      readonly kind: 'diagonal';
      readonly label?: string;
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
  readonly entries?: readonly { readonly id: string; readonly label: string; readonly color: string; readonly hatched?: boolean }[];
  readonly gradient?: {
    readonly lowLabel: string;
    readonly highLabel: string;
    readonly stops: readonly string[];
  };
  readonly rightNote?: string;
}

export interface HeatMatrixModel {
  readonly hour: number;
  readonly rows: readonly MatrixAxisEntity[];
  readonly columns: readonly MatrixAxisEntity[];
  readonly cell: (rowIndex: number, columnIndex: number) => MatrixCell;
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
  readonly family: 'pairs';
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
  const col = model.columns[columnIndex];
  if (!row || !col) return null;
  const cell = model.cell(rowIndex, columnIndex);
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

export function cellAuditValue(cell: MatrixCell): string {
  if (cell.kind === 'diagonal') return '—';
  if (cell.kind === 'missing') return cell.audit ?? `Not recorded (${cell.reason})`;
  if (cell.kind === 'empty') return cell.audit ?? cell.label ?? '0';
  return cell.audit ?? cell.text ?? (cell.value !== undefined ? String(cell.value) : '');
}

export function deriveMatrixAudit(
  model: HeatMatrixModel,
  window?: { readonly firstRow: number; readonly endRow: number; readonly firstCol: number; readonly endCol: number },
): readonly AuditRow[] {
  const firstRow = window ? Math.max(0, window.firstRow) : 0;
  const endRow = window ? Math.min(model.rows.length, window.endRow) : model.rows.length;
  const firstCol = window ? Math.max(0, window.firstCol) : 0;
  const endCol = window ? Math.min(model.columns.length, window.endCol) : model.columns.length;
  const out: AuditRow[] = [];
  for (let r = firstRow; r < endRow; r++) {
    const row = model.rows[r]!;
    for (let c = firstCol; c < endCol; c++) {
      const col = model.columns[c]!;
      const cell = model.cell(r, c);
      if (cell.kind === 'diagonal') continue;
      out.push({
        key: `${row.id}->${col.id}`,
        row: `${row.label} → ${col.label}`,
        value: cellAuditValue(cell),
      });
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
    const cells = [];
    for (let c = c0; c < c1; c++) {
      const col = model.columns[c]!;
      const cell = model.cell(r, c);
      cells.push({
        column: col.label,
        spoken: cellAuditValue(cell),
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
