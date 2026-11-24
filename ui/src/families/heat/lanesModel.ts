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
  readonly exact?: boolean;
  readonly cell: HeatLaneCell;
  readonly tooltip?: FamilyTooltipPayload;
}

export type HeatLaneReadout =
  | {
      readonly kind: 'value';
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
  readonly readoutWidth?: number;
  readonly rowHeight?: number;
  readonly tooltip?: (row: HeatLaneRow, run: HeatLaneRun | null, hour: number) => FamilyTooltipPayload | null;
  readonly empty?: string;
  readonly emptyDetail?: string;
  readonly emptyKind?: 'empty' | 'unavailable';
  readonly refused?: string;
  readonly notes?: readonly string[];
  readonly warnings?: readonly string[];
}

export interface HeatLanesProjection<TOptions = Record<string, never>> extends FamilyProjection<HeatLanesModel, TOptions> {
  readonly family: 'timeline';
  project(readView: BoundedReadView, options: TOptions): HeatLanesModel;
  nextChange?(readView: BoundedReadView, options: TOptions): number;
}

export function laneInspectKey(rowIndex: number, hour: number | null): string {
  return hour === null ? `${rowIndex}` : `${rowIndex}@${Math.floor(hour)}`;
}

export function parseLaneInspectKey(key: string | null): { readonly rowIndex: number; readonly hour: number | null } | null {
  if (!key) return null;
  const at = key.indexOf('@');
  if (at < 0) {
    const rowIndex = Number(key);
    return Number.isInteger(rowIndex) && rowIndex >= 0 ? { rowIndex, hour: null } : null;
  }
  const rowIndex = Number(key.slice(0, at));
  const hour = Number(key.slice(at + 1));
  if (!Number.isInteger(rowIndex) || rowIndex < 0 || !Number.isFinite(hour)) return null;
  return { rowIndex, hour: Math.floor(hour) };
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

export function readoutAuditText(readout: HeatLaneReadout | undefined): string {
  if (!readout) return '';
  if (readout.audit !== undefined) return readout.audit;
  if (readout.kind === 'value') {
    return readout.secondary ? `${readout.primary} (${readout.secondary})` : readout.primary;
  }
  if (readout.kind === 'clear') return readout.label;
  return readout.figure ? `${readout.figure} not recorded` : `Not recorded (${readout.reason})`;
}

export function deriveLanesTooltip(
  model: HeatLanesModel,
  rowIndex: number,
  hour: number | null,
): FamilyTooltipPayload | null {
  const row = model.rows[rowIndex];
  if (!row) return null;
  const effectiveHour = hour === null ? model.hour : Math.min(hour, model.hour);
  const run = findRunAtHour(row.runs, effectiveHour, model.hour);
  if (model.tooltip) {
    const custom = model.tooltip(row, run, effectiveHour);
    if (custom) return custom;
  }
  if (run?.tooltip) return run.tooltip;
  if (model.gaps?.some(g => g.firstHour <= effectiveHour && effectiveHour <= Math.min(g.lastHour, model.hour))) {
    const gap = model.gaps.find(g => g.firstHour <= effectiveHour && effectiveHour <= Math.min(g.lastHour, model.hour));
    return {
      title: row.label,
      hour: effectiveHour,
      missingReason: gap?.reason ?? 'Recording gap',
    };
  }
  if (!run) {
    if (row.readout) {
      if (row.readout.kind === 'missing') {
        return { title: row.label, hour: effectiveHour, missingReason: row.readout.reason };
      }
      if (row.readout.kind === 'clear') {
        return { title: row.label, hour: effectiveHour, items: [{ label: row.readout.label, tone: 'muted' }] };
      }
      return {
        title: row.label,
        hour: effectiveHour,
        exactValue: row.readout.primary,
        ...(row.readout.secondary ? { note: row.readout.secondary } : {}),
      };
    }
    return {
      title: row.label,
      hour: effectiveHour,
      missingReason: 'Not recorded at this hour',
    };
  }
  const cell = run.cell;
  if (cell.kind === 'missing' || cell.kind === 'gap') {
    return {
      title: row.label,
      hour: run.readHour ?? effectiveHour,
      missingReason: cell.reason ?? 'Not recorded',
    };
  }
  if (cell.kind === 'clear') {
    return {
      title: row.label,
      hour: run.readHour ?? effectiveHour,
      items: [{ label: cell.label ?? 'Clear', tone: 'muted' }],
    };
  }
  if (cell.kind === 'solid') {
    return {
      title: row.label,
      hour: run.readHour ?? effectiveHour,
      ...(cell.label ? { exactValue: cell.label } : {}),
    };
  }
  return {
    title: row.label,
    hour: run.readHour ?? effectiveHour,
    items: cell.parts.map(part => ({
      label: part.label,
      value: part.text ?? `${Math.round(part.share * 100)}%`,
      swatch: part.color,
    })),
  };
}

export function deriveLanesAudit(
  model: HeatLanesModel,
  window?: { readonly firstRow: number; readonly endRow: number },
): readonly AuditRow[] {
  const first = window ? Math.max(0, window.firstRow) : 0;
  const end = window ? Math.min(model.rows.length, window.endRow) : model.rows.length;
  const out: AuditRow[] = [];
  for (let i = first; i < end; i++) {
    const row = model.rows[i]!;
    out.push({
      key: row.id,
      row: row.label,
      value: row.audit ?? readoutAuditText(row.readout),
    });
  }
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
        {
          column: formatDateOnly(model.hour),
          spoken: readoutAuditText(row.readout) || 'Recorded',
        },
      ],
    });
  }
  return { first: r0, total: model.rows.length, rows };
}
