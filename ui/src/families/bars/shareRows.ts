import type { FamilyTooltipPayload } from '../host/tooltips.ts';
import type { Value } from '../value.ts';
import type { ValueFormat } from '../valueFormat.ts';
import type { BarsDataRow, BarsModel, BarsRow, BarsSegment } from './model.ts';

export interface ShareRowEntry {
  readonly key: string;
  readonly subject?: string;
  readonly label: string;
  readonly labelMissing?: boolean;
  readonly group?: 'picked' | 'auto';
  readonly removable?: boolean;
  readonly reorderable?: boolean;
  readonly segments: readonly BarsSegment[];
  readonly countText?: string;
  readonly missing?: Value;
  readonly format?: ValueFormat;
  readonly tooltip?: FamilyTooltipPayload;
  readonly meta?: unknown;
}

export interface ShareRowsInput {
  readonly hour: number;
  readonly picked?: readonly ShareRowEntry[];
  readonly automatic?: readonly ShareRowEntry[];
  readonly rows?: readonly ShareRowEntry[];
  readonly pickedHeading?: string;
  readonly automaticHeading?: string;
  readonly addRegionLabel?: string | null;
  readonly summaryText?: string;
  readonly empty?: string;
  readonly emptyDetail?: string;
  readonly emptyKind?: 'empty' | 'unavailable';
  readonly notes?: readonly string[];
  readonly warnings?: readonly string[];
  readonly nextChangeAt?: number;
}

function toShareDataRow(entry: ShareRowEntry, group?: 'picked' | 'auto'): BarsDataRow {
  return {
    kind: 'data',
    key: entry.key,
    subject: entry.subject ?? entry.key,
    label: entry.label,
    labelMissing: entry.labelMissing,
    group: entry.group ?? group,
    removable: entry.removable,
    reorderable: entry.reorderable,
    segments: entry.segments,
    ...(entry.missing ? { value: { value: entry.missing, format: entry.format } } : {}),
    ...(entry.countText !== undefined ? { readout: { primary: entry.countText } } : {}),
    ...(entry.tooltip ? { tooltip: entry.tooltip } : {}),
    ...(entry.meta !== undefined ? { meta: entry.meta } : {}),
  };
}

export function buildShareRowsModel(input: ShareRowsInput): BarsModel {
  const rows: BarsRow[] = [];
  if (input.picked?.length || input.automatic?.length) {
    if (input.picked?.length) {
      rows.push({ kind: 'section', key: '__section:picked', label: input.pickedHeading ?? 'Your regions', count: input.picked.length });
      for (const entry of input.picked) rows.push(toShareDataRow(entry, 'picked'));
    }
    if (input.automatic?.length) {
      rows.push({ kind: 'section', key: '__section:auto', label: input.automaticHeading ?? 'Most active', count: input.automatic.length });
      for (const entry of input.automatic) rows.push(toShareDataRow(entry, 'auto'));
    }
  } else if (input.rows?.length) {
    for (const entry of input.rows) rows.push(toShareDataRow(entry));
  }
  if (rows.length > 0 && input.addRegionLabel) {
    rows.push({ kind: 'add-region', key: '__add-region', label: input.addRegionLabel });
  }
  return {
    form: 'share',
    hour: input.hour,
    rows,
    showAxis: false,
    ...(input.summaryText !== undefined ? { summaryText: input.summaryText } : {}),
    ...(input.empty !== undefined ? { empty: input.empty } : {}),
    ...(input.emptyDetail !== undefined ? { emptyDetail: input.emptyDetail } : {}),
    ...(input.emptyKind !== undefined ? { emptyKind: input.emptyKind } : {}),
    ...(input.notes ? { notes: input.notes } : {}),
    ...(input.warnings ? { warnings: input.warnings } : {}),
    ...(input.nextChangeAt !== undefined ? { nextChangeAt: input.nextChangeAt } : {}),
  };
}
