import type { StatusMarkKind } from '../../draw/kit.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import {
  LOG_TABLE_BASE,
  logCountryColumn,
  logDateCell,
  logDateColumn,
  logItemColumn,
  logKindColumn,
  logPlaceColumn,
  logStatusCell,
  logStatusColumn,
  urlArtHandle,
} from '../../families/table/logHelpers.ts';
import {
  boundedDateLabel,
  formatCountSummary,
  nonBlankText,
  occurrenceCountryCell,
  projectOccurrenceWindow,
  type OccurrenceCountryRole,
  type OccurrenceRow,
} from '../../families/table/logOccurrence.ts';
import type { TableCell, TableColumnSpec, TableModel, TableProjection, TooltipExtraRow } from '../../families/table/model.ts';
import { OFFERED } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded } from '../../families/value.ts';
import { formatProgress, formatScalar } from '../../families/valueFormat.ts';

export type ConstructionOccurrenceStatus =
  | 'building'
  | 'repairing'
  | 'waiting'
  | 'built'
  | 'repaired'
  | 'completed'
  | 'cancelled'
  | 'removed'
  | string;

export interface ConstructionOccurrenceRow extends OccurrenceRow {
  readonly buildingToken: string | null;
  readonly buildingName: string | null;
  readonly buildingMissing?: string | null;
  readonly icon: string | null;
  readonly repair: boolean;
  readonly country: OccurrenceCountryRole;
  readonly placeName: string | null;
  readonly placeMissing?: string | null;
  readonly builtLevels: number | null;
  readonly orderedLevels: number | null;
  readonly possibleRange?: readonly [number, number] | null;
  readonly levelsMissing?: string | null;
  readonly levelProgressFraction?: number | null;
  readonly factoriesAssigned: number | null;
  readonly factoriesMax: number | null;
  readonly factoriesMissing?: string | null;
  readonly status: ConstructionOccurrenceStatus;
  readonly statusLabel?: string | null;
  readonly statusMissing?: string | null;
  readonly startedHour?: number | null;
  readonly endedHour?: number | null;
}

export interface ConstructionLogProjectionOptions {
  readonly filtered: boolean;
  readonly artwork: boolean;
}

export const CONSTRUCTION_LOG_COLUMNS: readonly TableColumnSpec[] = [
  logDateColumn(),
  logKindColumn({ minWidth: 34, maxWidth: 42 }),
  logCountryColumn(),
  logItemColumn({ label: 'Building', minWidth: 160 }),
  logPlaceColumn({ minWidth: 112 }),
  {
    key: 'levels',
    label: 'Levels',
    as: 'plain',
    offered: OFFERED.text,
    priority: 2,
    sortable: false,
    numeric: true,
    minWidth: 116,
    widest: ['Levels not recorded'],
  },
  {
    key: 'factories',
    label: 'Factories',
    as: 'plain',
    offered: OFFERED.text,
    priority: 3,
    sortable: false,
    numeric: true,
    minWidth: 128,
    widest: ['15 / 15 factories'],
  },
  logStatusColumn({ minWidth: 108, widest: ['Repairing'] }),
];

const KNOWN_STATUS: Readonly<Record<string, { readonly label: string; readonly mark: StatusMarkKind; readonly muted?: boolean }>> = {
  building: { label: 'Building', mark: 'running' },
  repairing: { label: 'Repairing', mark: 'running' },
  waiting: { label: 'Waiting', mark: 'waiting' },
  built: { label: 'Built', mark: 'finished' },
  repaired: { label: 'Repaired', mark: 'finished' },
  completed: { label: 'Completed', mark: 'finished' },
  cancelled: { label: 'Cancelled', mark: 'ended', muted: true },
  removed: { label: 'Removed', mark: 'ended', muted: true },
};

function levelsCell(row: ConstructionOccurrenceRow): TableCell {
  if (row.possibleRange) {
    const [lo, hi] = row.possibleRange;
    return {
      value: knownValue(`${formatScalar(lo, 'integer')} or ${formatScalar(hi, 'integer')}`),
      unitSuffix: hi === 1 ? 'level' : 'levels',
    };
  }
  if (row.builtLevels !== null && row.orderedLevels !== null) {
    return {
      value: knownValue(`${formatScalar(row.builtLevels, 'integer')} / ${formatScalar(row.orderedLevels, 'integer')}`),
      unitSuffix: row.orderedLevels === 1 ? 'level' : 'levels',
    };
  }
  if (row.builtLevels !== null) {
    return {
      value: knownValue(formatScalar(row.builtLevels, 'integer')),
      unitSuffix: row.builtLevels === 1 ? 'level' : 'levels',
    };
  }
  return {
    value: notRecorded(row.levelsMissing ?? 'Construction levels were not recorded'),
    missingFigure: 'Levels',
  };
}

function factoriesCell(readView: BoundedReadView, row: ConstructionOccurrenceRow): TableCell {
  if (!row.live) {
    return { value: notApplicable('No longer in queue'), blank: true };
  }
  if (row.factoriesAssigned === null) {
    return {
      value: notRecorded(row.factoriesMissing ?? 'Assigned factories were not recorded'),
      missingFigure: 'Factories',
    };
  }
  const sprite = readView.art({ kind: 'sprite', key: 'GFX_civ_factory_icon', frame: 0 });
  const iconFields = sprite.status === 'ready'
    ? { unitIcon: sprite.url }
    : sprite.status === 'pending'
      ? { unitIconPending: true }
      : {};
  if (row.factoriesMax !== null && row.factoriesMax > 0) {
    return {
      value: knownValue(`${formatScalar(row.factoriesAssigned, 'integer')} / ${formatScalar(row.factoriesMax, 'integer')}`),
      unitSuffix: 'factories',
      ...iconFields,
    };
  }
  return {
    value: knownValue(formatScalar(row.factoriesAssigned, 'integer')),
    unitSuffix: row.factoriesAssigned === 1 ? 'factory' : 'factories',
    ...iconFields,
  };
}

function constructionStatus(row: ConstructionOccurrenceRow): { readonly cell: TableCell; readonly label: string; readonly muted: boolean } {
  if (row.statusMissing) {
    return {
      cell: { value: notRecorded(row.statusMissing), missingFigure: 'Status' },
      label: 'Status not recorded',
      muted: false,
    };
  }
  const known = KNOWN_STATUS[row.status];
  if (known) {
    return {
      cell: logStatusCell(known.label, known.mark, known.muted ? 'muted' : undefined),
      label: known.label,
      muted: known.muted === true,
    };
  }
  const label = nonBlankText(row.statusLabel) ?? nonBlankText(row.status) ?? 'Status not recorded';
  return {
    cell: logStatusCell(label, 'finished'),
    label,
    muted: false,
  };
}

export const constructionLogProjection: TableProjection<ConstructionLogProjectionOptions> = {
  kind: 'construction-log',
  family: 'table',
  project(readView, options): TableModel {
    return projectOccurrenceWindow<ConstructionOccurrenceRow>(
      readView,
      {
        columns: CONSTRUCTION_LOG_COLUMNS,
        empty: 'No recorded construction projects.',
        emptyWhenFiltered: 'No construction projects match these settings.',
        filtered: options.filtered,
        titleCount: counts => formatCountSummary(counts.live, 'building', 'building'),
      },
      (row, boundary) => {
        const buildingName = nonBlankText(row.buildingName);
        const safeName = buildingName ?? nonBlankText(row.buildingToken) ?? 'Building not recorded';
        const placeName = nonBlankText(row.placeName);
        const statusInfo = constructionStatus(row);
        const artHandle = options.artwork ? urlArtHandle(row.icon) : null;
        const artResult = artHandle ? readView.art(artHandle) : null;
        const icon = artResult?.status === 'ready' ? artResult.url : undefined;
        const iconPending = artResult?.status === 'pending' ? true : undefined;

        const tooltipExtra = (): TooltipExtraRow[] => {
          const out: TooltipExtraRow[] = [
            { label: 'Recorded', value: knownValue(boundedDateLabel(row.hour, row.exact)) },
            { label: 'Status', value: row.statusMissing ? notRecorded(row.statusMissing) : knownValue(statusInfo.label) },
          ];
          if (row.levelProgressFraction != null) {
            out.push({ label: 'Current level progress', value: knownValue(formatProgress(row.levelProgressFraction)) });
          }
          if (row.startedHour != null) out.push({ label: 'Started', value: knownValue(boundedDateLabel(row.startedHour, true)) });
          if (row.endedHour != null) out.push({ label: 'Ended', value: knownValue(boundedDateLabel(row.endedHour, row.exact)) });
          if (row.buildingMissing) out.push({ label: 'Building type', value: notRecorded(row.buildingMissing) });
          if (row.placeMissing) out.push({ label: 'Place', value: notRecorded(row.placeMissing) });
          if (row.levelsMissing) out.push({ label: 'Levels', value: notRecorded(row.levelsMissing) });
          if (row.factoriesMissing) out.push({ label: 'Factories', value: notRecorded(row.factoriesMissing) });
          return out;
        };

        return {
          row: {
            key: row.id,
            name: safeName,
            rowHour: row.hour,
            live: row.live,
            muted: statusInfo.muted,
            cells: {
              date: logDateCell(row.hour, row.exact ? undefined : 'by'),
              kind: options.artwork
                ? {
                    value: buildingName !== null ? knownValue(buildingName) : notRecorded(row.buildingMissing ?? 'Building icon not recorded'),
                    icon,
                    iconPending,
                    iconSlot: true,
                    artShape: 'square',
                    artName: safeName,
                  }
                : { value: notApplicable('Game artwork hidden'), blank: true },
              country: occurrenceCountryCell(readView, [row.country], boundary),
              item: buildingName !== null
                ? { value: knownValue(buildingName), ...(row.repair && !row.live ? { inlineSubtext: 'Repair' } : {}) }
                : { value: notRecorded(row.buildingMissing ?? 'Building type was not recorded') },
              place: placeName !== null
                ? { value: knownValue(placeName) }
                : { value: notRecorded(row.placeMissing ?? 'Place not recorded'), missingFigure: 'Place' },
              levels: levelsCell(row),
              factories: factoriesCell(readView, row),
              status: statusInfo.cell,
            },
            tooltipTitle: safeName,
            tooltipExtra,
          },
        };
      },
    );
  },
};

export const CONSTRUCTION_LOG_BASE = LOG_TABLE_BASE;
