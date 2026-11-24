import type { AuditRow } from '../../draw/notifications';
import type { ArtHandle, BoundedReadView, FamilyProjection, MirrorModel, MirrorRow } from '../contract.ts';
import type { FamilyTooltipPayload } from '../host/tooltips.ts';
import { formatDateOnly } from '../valueFormat.ts';
import type { HeatPart } from './scale.ts';

export type HeatLaneCell =
  | {
      readonly kind: 'parts';
      readonly parts: readonly HeatPart[];
      readonly look: 'margin' | 'split';
      readonly label?: string;
      readonly color?: string;
    }
  | {
      readonly kind: 'solid';
      readonly color: string;
      readonly label?: string;
      readonly textColor?: string;
    }
  | {
      readonly kind: 'clear';
      readonly label?: string;
    }
  | {
      readonly kind: 'missing';
      readonly reason: string;
    }
  | {
      readonly kind: 'gap';
      readonly reason?: string;
    };

export interface HeatLaneRun {
  readonly firstHour: number;
  readonly lastHour: number;
  readonly readHour: number | null;
  readonly cell: HeatLaneCell;
}

export type HeatLaneReadout =
  | {
      readonly kind: 'value';
      readonly form?: 'value' | 'word';
      readonly swatch?: string | 'hollow';
      readonly primary: string;
      readonly secondary?: string;
      readonly tone?: 'positive' | 'negative' | 'neutral' | 'muted';
      readonly color?: string;
      readonly audit?: string;
    }
  | {
      readonly kind: 'missing';
      readonly figure?: string;
      readonly reason: string;
      readonly audit?: string;
    }
  | {
      readonly kind: 'clear';
      readonly label: string;
      readonly audit?: string;
    };

export interface HeatLaneRow {
  readonly id: string;
  readonly label: string;
  readonly nameMissing?: string;
  readonly tag?: string;
  readonly flag?: ArtHandle | null;
  readonly flagPending?: boolean;
  readonly pinned?: boolean;
  readonly region?: number;
  readonly runs: readonly HeatLaneRun[];
  readonly readout?: HeatLaneReadout;
  readonly meta?: unknown;
  readonly audit?: string;
}

export interface HeatLaneLegendItem {
  readonly id: string;
  readonly label: string;
  readonly color: string;
  readonly dashed?: boolean;
  readonly hatched?: boolean;
  readonly muted?: boolean;
}

export interface HeatLaneLegend {
  readonly entries?: readonly HeatLaneLegendItem[];
  readonly gradient?: {
    readonly lowLabel: string;
    readonly highLabel: string;
    readonly stops: readonly string[];
  };
  readonly rightNote?: string;
}

export interface HeatLaneRegions {
  readonly pickedHeading: string;
  readonly automaticHeading: string;
  readonly addLabel: string;
}

export interface HeatLaneGap {
  readonly firstHour: number;
  readonly lastHour: number;
  readonly reason?: string;
}

export interface HeatLanesModel {
  readonly hour: number;
  readonly firstHour: number;
  readonly lastHour: number;
  readonly dayHours: number;
  readonly playhead: number | null;
  readonly gaps?: readonly HeatLaneGap[];
  readonly legend?: HeatLaneLegend;
  readonly rows: readonly HeatLaneRow[];
  readonly regions?: HeatLaneRegions;
  readonly readoutWidth?: number;
  readonly rowHeight?: number;
  readonly inspect?: (rowIndex: number, hour: number) => FamilyTooltipPayload | null;
  readonly empty?: string;
  readonly emptyDetail?: string;
  readonly emptyKind?: 'empty' | 'unavailable';
  readonly refused?: string;
  readonly notes?: readonly string[];
  readonly warnings?: readonly string[];
}

export interface HeatLanesProjection<TOptions = Record<string, never>> extends FamilyProjection<HeatLanesModel, TOptions> {
  readonly family: 'heat-lanes';
  project(readView: BoundedReadView, options: TOptions): HeatLanesModel;
  nextChange?(readView: BoundedReadView, options: TOptions): number;
}

export function laneInspectKey(rowIndex: number, hour: number | null): string {
  return hour === null ? `#${rowIndex}` : `#${rowIndex}@${Math.floor(hour)}`;
}

export function parseLaneInspectKey(key: string | null): { readonly rowIndex: number; readonly hour: number | null } | null {
  if (!key || !key.startsWith('#')) return null;
  const at = key.indexOf('@');
  const rowIndex = Number(at < 0 ? key.slice(1) : key.slice(1, at));
  if (!Number.isInteger(rowIndex) || rowIndex < 0) return null;
  if (at < 0) return { rowIndex, hour: null };
  const hour = Number(key.slice(at + 1));
  return Number.isFinite(hour) ? { rowIndex, hour: Math.floor(hour) } : null;
}

export interface HeatLaneIdentity {
  readonly id: string;
  readonly label: string;
  readonly meta?: unknown;
}

export function laneIdentities(rows: readonly HeatLaneRow[]): readonly HeatLaneIdentity[] {
  return rows.map(row => ({ id: row.id, label: row.label, ...(row.meta !== undefined ? { meta: row.meta } : {}) }));
}

export function findRunAtHour(runs: readonly HeatLaneRun[], hour: number, maxHour: number): HeatLaneRun | null {
  if (hour > maxHour || !runs.length) return null;
  let lo = 0;
  let hi = runs.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (runs[mid]!.firstHour <= hour) lo = mid + 1;
    else hi = mid;
  }
  const idx = lo - 1;
  if (idx < 0) return null;
  const candidate = runs[idx]!;
  if (candidate.readHour !== null && candidate.readHour > maxHour) return null;
  const effectiveLast = Math.min(candidate.lastHour, maxHour);
  return candidate.firstHour <= hour && hour <= effectiveLast ? candidate : null;
}

export function spanIndexAt<T>(items: readonly T[], hour: number, firstOf: (item: T) => number): number {
  let lo = 0;
  let hi = items.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (firstOf(items[mid]!) <= hour) lo = mid + 1;
    else hi = mid;
  }
  return lo - 1;
}

export function readoutAuditText(readout: HeatLaneReadout | undefined): string {
  if (!readout) return '';
  if (readout.audit !== undefined) return readout.audit;
  if (readout.kind === 'value') {
    return readout.secondary ? `${readout.primary} (${readout.secondary})` : readout.primary;
  }
  if (readout.kind === 'clear') return readout.label;
  return readout.figure ? `${readout.figure} not recorded` : `Not recorded (${readout.reason})`;
}

function readoutTooltip(row: HeatLaneRow, hour: number): FamilyTooltipPayload {
  const readout = row.readout;
  if (!readout) return { title: row.label, hour, missingReason: 'Not recorded at this hour' };
  if (readout.kind === 'missing') return { title: row.label, hour, missingReason: readout.reason };
  if (readout.kind === 'clear') return { title: row.label, hour, items: [{ label: readout.label, tone: 'muted' }] };
  return { title: row.label, hour, exactValue: readout.primary, ...(readout.secondary ? { note: readout.secondary } : {}) };
}

export function laneCellTooltip(title: string, hour: number, cell: HeatLaneCell): FamilyTooltipPayload {
  if (cell.kind === 'missing' || cell.kind === 'gap') return { title, hour, missingReason: cell.reason ?? 'Not recorded' };
  if (cell.kind === 'clear') return { title, hour, items: [{ label: cell.label ?? 'Clear', tone: 'muted' }] };
  if (cell.kind === 'solid') return { title, hour, ...(cell.label ? { exactValue: cell.label } : {}) };
  return {
    title,
    hour,
    items: cell.parts.map(part => ({ label: part.label, value: part.text ?? `${Math.round(part.share * 100)}%`, swatch: part.color })),
  };
}

export function deriveLanesTooltip(
  model: HeatLanesModel,
  rowIndex: number,
  hour: number | null,
): FamilyTooltipPayload | null {
  const row = model.rows[rowIndex];
  if (!row) return null;
  if (hour === null) return readoutTooltip(row, model.hour);
  const at = Math.min(hour, model.hour);
  const gap = model.gaps?.find(g => g.firstHour <= at && at <= Math.min(g.lastHour, model.hour));
  if (gap) return { title: row.label, hour: at, missingReason: gap.reason ?? 'Recording gap' };
  const own = model.inspect?.(rowIndex, at);
  if (own) return own;
  const run = findRunAtHour(row.runs, at, model.hour);
  return run ? laneCellTooltip(row.label, at, run.cell) : { title: row.label, hour: at, missingReason: 'Not recorded at this hour' };
}

function cellAuditText(cell: HeatLaneCell): string {
  if (cell.kind === 'missing') return `Not recorded (${cell.reason})`;
  if (cell.kind === 'gap') return `Gap (${cell.reason ?? 'Recording gap'})`;
  if (cell.kind === 'clear') return cell.label ?? 'Clear';
  if (cell.kind === 'solid') return cell.label ? `${cell.label} ${cell.color}` : cell.color;
  return cell.parts.map(part => `${part.label} ${part.text ?? `${Math.round(part.share * 100)}%`} ${part.color}`).join(', ');
}

function laneAuditText(row: HeatLaneRow, hour: number): string {
  const out: string[] = [];
  for (const run of row.runs) {
    if (run.firstHour > hour) break;
    if (run.readHour !== null && run.readHour > hour) continue;
    out.push(`${run.firstHour}-${Math.min(run.lastHour, hour)} ${cellAuditText(run.cell)}`);
  }
  return out.join('; ');
}

export function deriveLanesAudit(
  model: HeatLanesModel,
  window: { readonly firstRow: number; readonly endRow: number },
): readonly AuditRow[] {
  const first = Math.max(0, window.firstRow);
  const end = Math.min(model.rows.length, window.endRow);
  const out: AuditRow[] = [];
  for (let i = first; i < end; i++) {
    const row = model.rows[i]!;
    out.push({ key: `${row.id}:readout`, row: row.id, value: row.audit ?? readoutAuditText(row.readout) });
    out.push({ key: `${row.id}:lane`, row: row.id, value: laneAuditText(row, model.hour) });
  }
  const gaps = (model.gaps ?? []).filter(gap => gap.firstHour <= model.hour).map(gap => `${gap.firstHour}-${Math.min(gap.lastHour, model.hour)}`);
  if (gaps.length) out.push({ key: 'gaps', row: 'gaps', value: gaps.join('; ') });
  return out;
}

export function deriveLanesMirror(
  model: HeatLanesModel,
  firstRow: number,
  endRow: number,
): MirrorModel {
  const r0 = Math.max(0, firstRow);
  const r1 = Math.min(model.rows.length, endRow);
  const rows: MirrorRow[] = [];
  for (let i = r0; i < r1; i++) {
    const row = model.rows[i]!;
    rows.push({
      key: row.id,
      name: row.label,
      rowIndex: i,
      cells: [
        { column: 'Name', spoken: row.label },
        {
          column: formatDateOnly(model.hour),
          spoken: `${formatDateOnly(model.hour)}: ${readoutAuditText(row.readout) || 'Recorded'}`,
        },
      ],
    });
  }
  return { first: r0, total: model.rows.length, rows };
}
