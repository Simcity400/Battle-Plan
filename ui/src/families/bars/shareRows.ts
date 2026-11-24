import type { FamilyTooltipPayload } from '../host/tooltips.ts';
import { REGION_ADD_KEY, type RegionRowRef, type RegionRowsField } from '../regions/regionRows.ts';
import type { Value } from '../value.ts';
import type { ValueFormat } from '../valueFormat.ts';
import type { BarsDataRow, BarsLegendItem, BarsModel, BarsRow, BarsRowMedia, BarsSegment } from './model.ts';

export interface ShareRowEntry {
  readonly key: string;
  readonly subject?: string;
  readonly label: string;
  readonly labelMissing?: boolean;
  readonly media?: BarsRowMedia;
  readonly group?: 'picked' | 'auto';
  readonly region?: number;
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
  readonly legend?: readonly BarsLegendItem[];
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
    ...(entry.media ? { media: entry.media } : {}),
    group: entry.group ?? group,
    ...(entry.region !== undefined ? { region: entry.region } : {}),
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
    rows.push({ kind: 'add-region', key: REGION_ADD_KEY, label: input.addRegionLabel });
  }
  return {
    form: 'share',
    hour: input.hour,
    rows,
    showAxis: false,
    ...(input.summaryText !== undefined ? { summaryText: input.summaryText } : {}),
    ...(input.legend?.length ? { legend: input.legend } : {}),
    ...(input.empty !== undefined ? { empty: input.empty } : {}),
    ...(input.emptyDetail !== undefined ? { emptyDetail: input.emptyDetail } : {}),
    ...(input.emptyKind !== undefined ? { emptyKind: input.emptyKind } : {}),
    ...(input.notes ? { notes: input.notes } : {}),
    ...(input.warnings ? { warnings: input.warnings } : {}),
    ...(input.nextChangeAt !== undefined ? { nextChangeAt: input.nextChangeAt } : {}),
  };
}

export function barsRegionRows(model: BarsModel, layout: { readonly bodyTop: number; readonly rowTops: readonly number[]; readonly rowHeights: readonly number[]; readonly nameX: number }): RegionRowsField | null {
  const rows: RegionRowRef[] = [];
  let add: RegionRowsField['add'] = null;
  model.rows.forEach((row, index) => {
    const top = layout.bodyTop + layout.rowTops[index]!;
    const height = layout.rowHeights[index]!;
    if (row.kind === 'add-region') add = { top, height };
    if (row.kind !== 'data' || row.region === undefined) return;
    const lead = row.segments?.reduce<BarsSegment | null>((best, segment) => (segment.share > (best?.share ?? 0) ? segment : best), null);
    rows.push({
      key: row.key,
      id: row.region,
      label: row.label,
      picked: row.group === 'picked',
      top,
      height,
      ...(lead ? { activity: { color: lead.color, text: `${Math.round(lead.share * 100)}% ${lead.label}` } } : {}),
    });
  });
  return rows.length || add ? { rows, add, nameLeft: layout.nameX } : null;
}
