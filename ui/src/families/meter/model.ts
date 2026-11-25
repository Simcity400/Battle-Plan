import { auditRow, type FamilyAuditRow } from '../auditRow.ts';
import type { ThresholdTick } from '../../draw/kit.ts';
import type { ArtResult, BoundedReadView, MirrorModel, MirrorRow } from '../contract.ts';
import type { FamilyNavTuple } from '../host/keyboard.ts';
import type { FamilyTooltipPayload } from '../host/tooltips.ts';

export type MeterLayoutMode = 'rows' | 'columns' | 'ciphers';

export interface MeterArt {
  readonly result: ArtResult;
  readonly resolvedName?: string;
  readonly radius?: number;
  readonly idle?: boolean;
}

export interface MeterFlag {
  readonly tag: string;
  readonly result: ArtResult;
  readonly dimmed?: boolean;
}

export type MeterBar =
  | { readonly kind: 'none'; readonly dash?: boolean }
  | { readonly kind: 'track'; readonly fraction: number | null; readonly color?: string; readonly height?: number }
  | {
      readonly kind: 'threshold';
      readonly fraction: number | null;
      readonly ticks: readonly ThresholdTick[];
      readonly ends?: boolean;
      readonly color?: string;
      readonly height?: number;
    };

export interface MeterPairLabel {
  readonly ownerFlag: MeterFlag;
  readonly ownerName: string;
  readonly targetFlag: MeterFlag;
  readonly targetName: string;
}

export interface MeterRow {
  readonly key: string;
  readonly name: string;
  readonly subtitle?: string;
  readonly muted?: boolean;
  readonly art?: MeterArt;
  readonly flag?: MeterFlag;
  readonly pair?: MeterPairLabel;
  readonly bar: MeterBar;
  readonly figure?: string;
  readonly figureMissing?: boolean;
  readonly figureMissingLabel?: string;
  readonly secondary?: string;
  readonly sparkline?: readonly (number | null)[];
  readonly compactFigure?: string;
  readonly clickable?: boolean;
  readonly spoken: string;
  readonly audit: string;
  readonly tooltip?: FamilyTooltipPayload;
  readonly meta?: unknown;
}

export interface MeterChip {
  readonly key: string;
  readonly targetFlag: MeterFlag;
  readonly targetName: string;
  readonly dateText: string | null;
  readonly spoken: string;
  readonly audit: string;
  readonly tooltip?: FamilyTooltipPayload;
  readonly meta?: unknown;
}

export interface MeterChipLine {
  readonly key: string;
  readonly ownerFlag: MeterFlag;
  readonly ownerName: string;
  readonly chips: readonly MeterChip[];
}

export interface MeterSection {
  readonly key: string;
  readonly title: string;
  readonly count?: string;
  readonly rightCaption?: string;
  readonly rows?: readonly MeterRow[];
  readonly chipLines?: readonly MeterChipLine[];
}

export interface MeterModel {
  readonly hour: number;
  readonly layout: MeterLayoutMode;
  readonly summaryText?: string | null;
  readonly sections?: readonly MeterSection[];
  readonly notes?: readonly string[];
  readonly warnings?: readonly string[];
  readonly empty?: string | null;
  readonly emptyDetail?: string | null;
  readonly emptyKind?: 'empty' | 'unavailable';
  readonly refused?: string;
  readonly nextChangeAt?: number;
}

export interface MeterProjection<TOptions = unknown> {
  readonly family: 'meters';
  readonly kind: string;
  project(readView: BoundedReadView, options: TOptions): MeterModel;
  nextChange?(readView: BoundedReadView, options: TOptions): number;
}

export function deriveMeterNav(
  model: MeterModel,
  bands?: ReadonlyMap<string, readonly [top: number, height: number]>,
): readonly FamilyNavTuple[] {
  const nav: FamilyNavTuple[] = [];
  let index = 0;
  const push = (key: string, name: string, meta: unknown) => {
    const band = bands?.get(key);
    nav.push(band ? [key, name, index++, -1, '', meta, band[0], band[1]] : [key, name, index++, -1, '', meta]);
  };
  for (const section of model.sections ?? []) {
    for (const row of section.rows ?? []) push(row.key, row.name, row.meta ?? null);
    for (const line of section.chipLines ?? []) {
      for (const chip of line.chips) push(chip.key, `${line.ownerName} to ${chip.targetName}`, chip.meta ?? null);
    }
  }
  return nav;
}

export function deriveMeterMirror(model: MeterModel): MirrorModel {
  const rows: MirrorRow[] = [];
  for (const section of model.sections ?? []) {
    for (const row of section.rows ?? []) {
      rows.push({ key: row.key, name: row.spoken, rowIndex: rows.length, cells: [] });
    }
    for (const line of section.chipLines ?? []) {
      for (const chip of line.chips) {
        rows.push({ key: chip.key, name: chip.spoken, rowIndex: rows.length, cells: [] });
      }
    }
  }
  return { first: 0, total: rows.length, rows };
}

export function deriveMeterAudit(model: MeterModel): readonly FamilyAuditRow[] {
  const rows: FamilyAuditRow[] = [];
  if (model.summaryText) rows.push(auditRow('summary', model.summaryText));
  if (model.sections) {
    for (const section of model.sections) {
      rows.push(auditRow(`section:${section.key}`, [section.title, section.count ?? '', section.rightCaption ?? ''].join('|')));
      if (section.rows) {
        for (const row of section.rows) {
          rows.push(auditRow(row.key, row.audit));
        }
      }
      if (section.chipLines) {
        for (const line of section.chipLines) {
          for (const chip of line.chips) {
            rows.push(auditRow(chip.key, chip.audit));
          }
        }
      }
    }
  }
  return rows;
}

export function findMeterTooltip(model: MeterModel, key: string): FamilyTooltipPayload | null {
  if (model.sections) {
    for (const section of model.sections) {
      if (section.rows) {
        for (const row of section.rows) {
          if (row.key === key) return row.tooltip ?? null;
        }
      }
      if (section.chipLines) {
        for (const line of section.chipLines) {
          for (const chip of line.chips) {
            if (chip.key === key) return chip.tooltip ?? null;
          }
        }
      }
    }
  }
  return null;
}

export function nextMeterChange(model: MeterModel, hour: number): number {
  if (model.nextChangeAt !== undefined && model.nextChangeAt > hour) return model.nextChangeAt;
  return Infinity;
}
