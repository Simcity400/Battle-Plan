import type { AuditRow } from '../../draw/notifications.ts';
import type { ThresholdTick } from '../../draw/kit.ts';
import type { ArtResult, BoundedReadView } from '../contract.ts';
import type { FamilyNavTuple, FamilyTooltipPayload, ListboxMirrorItem } from '../host/index.ts';

export type MeterLayoutMode = 'rows' | 'columns' | 'ciphers' | 'matrix';

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
  readonly pair?: MeterPairLabel;
  readonly bar: MeterBar;
  readonly figure?: string;
  readonly figureMissing?: boolean;
  readonly figureMissingLabel?: string;
  readonly secondary?: string;
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

export interface MeterMatrixColumn {
  readonly key: string;
  readonly flag: MeterFlag;
  readonly name: string;
}

export interface MeterMatrixCell {
  readonly key: string;
  readonly kind: 'diagonal' | 'broken' | 'progress' | 'missing' | 'none';
  readonly fraction?: number;
  readonly color?: string;
  readonly spoken: string;
  readonly audit?: string;
  readonly tooltip?: FamilyTooltipPayload;
  readonly meta?: unknown;
}

export interface MeterMatrixRow {
  readonly key: string;
  readonly ownerFlag: MeterFlag;
  readonly ownerName: string;
  readonly totalText: string;
  readonly cells: readonly MeterMatrixCell[];
}

export interface MeterMatrix {
  readonly cornerLabel: string;
  readonly columns: readonly MeterMatrixColumn[];
  readonly rows: readonly MeterMatrixRow[];
}

export interface MeterModel {
  readonly hour: number;
  readonly layout: MeterLayoutMode;
  readonly summary?: string | null;
  readonly sections?: readonly MeterSection[];
  readonly matrix?: MeterMatrix;
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

export function deriveMeterNav(model: MeterModel): readonly FamilyNavTuple[] {
  const nav: FamilyNavTuple[] = [];
  let index = 0;
  if (model.sections) {
    for (const section of model.sections) {
      if (section.rows) {
        for (const row of section.rows) {
          nav.push([row.key, row.name, index++, -1, '', row.meta ?? null]);
        }
      }
      if (section.chipLines) {
        for (const line of section.chipLines) {
          for (const chip of line.chips) {
            nav.push([chip.key, `${line.ownerName} to ${chip.targetName}`, index++, -1, '', chip.meta ?? null]);
          }
        }
      }
    }
  }
  if (model.matrix) {
    for (const row of model.matrix.rows) {
      for (const cell of row.cells) {
        if (cell.kind === 'diagonal' || cell.kind === 'none') continue;
        nav.push([cell.key, cell.spoken, index++, -1, '', cell.meta ?? null]);
      }
    }
  }
  return nav;
}

export function deriveMeterMirror(
  model: MeterModel,
  selectedKey: string | null,
): readonly ListboxMirrorItem[] {
  const items: ListboxMirrorItem[] = [];
  let index = 0;
  if (model.sections) {
    for (const section of model.sections) {
      if (section.rows) {
        for (const row of section.rows) {
          items.push({
            key: row.key,
            spoken: row.spoken,
            index: index++,
            selected: selectedKey === row.key,
          });
        }
      }
      if (section.chipLines) {
        for (const line of section.chipLines) {
          for (const chip of line.chips) {
            items.push({
              key: chip.key,
              spoken: chip.spoken,
              index: index++,
              selected: selectedKey === chip.key,
            });
          }
        }
      }
    }
  }
  if (model.matrix) {
    for (const row of model.matrix.rows) {
      for (const cell of row.cells) {
        if (cell.kind === 'diagonal' || cell.kind === 'none') continue;
        items.push({
          key: cell.key,
          spoken: cell.spoken,
          index: index++,
          selected: selectedKey === cell.key,
        });
      }
    }
  }
  return items;
}

export function deriveMeterAudit(model: MeterModel): readonly AuditRow[] {
  const rows: AuditRow[] = [];
  if (model.sections) {
    for (const section of model.sections) {
      if (section.rows) {
        for (const row of section.rows) {
          rows.push({ row: row.key, key: row.key, value: row.audit });
        }
      }
      if (section.chipLines) {
        for (const line of section.chipLines) {
          for (const chip of line.chips) {
            rows.push({ row: chip.key, key: chip.key, value: chip.audit });
          }
        }
      }
    }
  }
  if (model.matrix) {
    for (const row of model.matrix.rows) {
      for (const cell of row.cells) {
        if (cell.audit !== undefined) {
          rows.push({ row: cell.key, key: cell.key, value: cell.audit });
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
  if (model.matrix) {
    for (const row of model.matrix.rows) {
      for (const cell of row.cells) {
        if (cell.key === key) return cell.tooltip ?? null;
      }
    }
  }
  return null;
}

export function nextMeterChange(model: MeterModel, hour: number): number {
  if (model.nextChangeAt !== undefined && model.nextChangeAt > hour) return model.nextChangeAt;
  return Infinity;
}
