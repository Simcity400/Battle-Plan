import type { StatusMarkKind } from '../../draw/kit.ts';
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
import { formatScalar } from '../../families/valueFormat.ts';

export type RaidOccurrenceStatus =
  | 'assembling'
  | 'preparing'
  | 'ready'
  | 'succeeded'
  | 'cancelled'
  | 'failed'
  | string;

export interface RaidOccurrenceRow extends OccurrenceRow {
  readonly raidTypeToken: string | null;
  readonly raidName: string | null;
  readonly raidTypeMissing?: string | null;
  readonly icon: string | null;
  readonly raider: OccurrenceCountryRole;
  readonly victim: OccurrenceCountryRole;
  readonly placeName: string | null;
  readonly placeMissing?: string | null;
  readonly status: RaidOccurrenceStatus;
  readonly statusLabel?: string | null;
  readonly statusMissing?: string | null;
  readonly prepStartedHour: number | null;
  readonly prepDays: number | null;
  readonly prepMissing?: string | null;
  readonly endedHour: number | null;
  readonly endedExact?: boolean;
}

export interface RaidLogProjectionOptions {
  readonly filtered: boolean;
  readonly artwork: boolean;
}

export const RAID_LOG_COLUMNS: readonly TableColumnSpec[] = [
  logDateColumn(),
  logKindColumn({ minWidth: 34, maxWidth: 42 }),
  logCountryColumn(),
  logItemColumn({ label: 'Raid', minWidth: 168 }),
  logPlaceColumn({ minWidth: 112 }),
  {
    key: 'preparation',
    label: 'Preparation',
    as: 'plain',
    offered: OFFERED.text,
    priority: 2,
    sortable: false,
    numeric: true,
    minWidth: 132,
    widest: ['43 days preparing'],
  },
  logStatusColumn({ minWidth: 130, widest: ['Ready to launch'] }),
];

const KNOWN_STATUS: Readonly<Record<string, { readonly label: string; readonly mark: StatusMarkKind; readonly muted?: boolean }>> = {
  assembling: { label: 'Assembling', mark: 'waiting' },
  preparing: { label: 'Preparing', mark: 'running' },
  ready: { label: 'Ready to launch', mark: 'waiting' },
  succeeded: { label: 'Succeeded', mark: 'succeeded' },
  cancelled: { label: 'Cancelled', mark: 'ended', muted: true },
  failed: { label: 'Failed', mark: 'ended', muted: true },
};

function raidPreparationCell(row: RaidOccurrenceRow, atHour: number): { readonly cell: TableCell; readonly nextChangeAt: number } {
  if (row.status === 'preparing') {
    if (row.prepDays === null && row.prepStartedHour === null) {
      return {
        cell: { value: notRecorded(row.prepMissing ?? 'Preparation time was not recorded'), missingFigure: 'Preparation' },
        nextChangeAt: Infinity,
      };
    }
    const days = row.prepDays !== null
      ? Math.max(0, row.prepDays)
      : Math.max(0, Math.floor((atHour - row.prepStartedHour!) / 24));
    const nextDay = row.prepStartedHour !== null ? row.prepStartedHour + (days + 1) * 24 : Infinity;
    return {
      cell: {
        value: knownValue(formatScalar(days, 'integer')),
        unitSuffix: days === 1 ? 'day preparing' : 'days preparing',
      },
      nextChangeAt: nextDay > atHour ? nextDay : Infinity,
    };
  }
  if (row.status === 'ready') {
    if (row.prepDays === null) {
      return {
        cell: { value: notRecorded(row.prepMissing ?? 'Preparation time was not recorded'), missingFigure: 'Preparation' },
        nextChangeAt: Infinity,
      };
    }
    const days = Math.max(0, row.prepDays);
    return {
      cell: {
        value: knownValue(formatScalar(days, 'integer')),
        unitSuffix: days === 1 ? 'day prepared' : 'days prepared',
      },
      nextChangeAt: Infinity,
    };
  }
  return {
    cell: { value: notApplicable('Not preparing'), blank: true },
    nextChangeAt: Infinity,
  };
}

function raidStatusInfo(row: RaidOccurrenceRow): { readonly cell: TableCell; readonly label: string; readonly muted: boolean } {
  if (row.statusMissing) {
    return {
      cell: { value: notRecorded(row.statusMissing), missingFigure: 'Outcome' },
      label: 'Outcome not recorded',
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
  const label = nonBlankText(row.statusLabel) ?? nonBlankText(row.status) ?? 'Outcome not recorded';
  return {
    cell: logStatusCell(label, 'finished'),
    label,
    muted: false,
  };
}

export const raidLogProjection: TableProjection<RaidLogProjectionOptions> = {
  kind: 'raid-log',
  family: 'table',
  project(readView, options): TableModel {
    const hour = readView.hour;
    return projectOccurrenceWindow<RaidOccurrenceRow>(
      readView,
      {
        columns: RAID_LOG_COLUMNS,
        empty: 'No recorded raids.',
        emptyWhenFiltered: 'No raids match these settings.',
        filtered: options.filtered,
        titleCount: counts => formatCountSummary(counts.live, 'in preparation', 'in preparation'),
      },
      (row, boundary) => {
        const raidName = nonBlankText(row.raidName);
        const safeName = raidName ?? nonBlankText(row.raidTypeToken) ?? 'Raid not recorded';
        const placeName = nonBlankText(row.placeName);
        const statusInfo = raidStatusInfo(row);
        const prep = raidPreparationCell(row, hour);
        const artHandle = options.artwork ? urlArtHandle(row.icon) : null;
        const artResult = artHandle ? readView.art(artHandle) : null;
        const icon = artResult?.status === 'ready' ? artResult.url : undefined;
        const iconPending = artResult?.status === 'pending' ? true : undefined;
        const victimRole: OccurrenceCountryRole = row.status === 'succeeded'
          ? { ...row.victim, dimmed: true }
          : row.victim;

        const tooltipExtra = (): TooltipExtraRow[] => {
          const out: TooltipExtraRow[] = [
            { label: 'Started', value: knownValue(boundedDateLabel(row.hour, row.exact)) },
            { label: 'Status', value: row.statusMissing ? notRecorded(row.statusMissing) : knownValue(statusInfo.label) },
          ];
          if (row.endedHour !== null) {
            out.push({ label: 'Ended', value: knownValue(boundedDateLabel(row.endedHour, row.endedExact !== false)) });
          }
          if (row.raidTypeMissing) out.push({ label: 'Raid type', value: notRecorded(row.raidTypeMissing) });
          if (row.placeMissing) out.push({ label: 'Target place', value: notRecorded(row.placeMissing) });
          if (row.prepMissing) out.push({ label: 'Preparation', value: notRecorded(row.prepMissing) });
          return out;
        };

        return {
          nextChangeAt: prep.nextChangeAt,
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
                    value: raidName !== null ? knownValue(raidName) : notRecorded(row.raidTypeMissing ?? 'Raid icon not recorded'),
                    icon,
                    iconPending,
                    iconSlot: true,
                    artShape: 'square',
                    artName: safeName,
                  }
                : { value: notApplicable('Game artwork hidden'), blank: true },
              country: occurrenceCountryCell(readView, [row.raider, victimRole], boundary),
              item: raidName !== null
                ? { value: knownValue(raidName) }
                : { value: notRecorded(row.raidTypeMissing ?? 'Raid type was not recorded') },
              place: placeName !== null
                ? { value: knownValue(placeName) }
                : { value: notRecorded(row.placeMissing ?? 'Target place not recorded'), missingFigure: 'Place' },
              preparation: prep.cell,
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

export const RAID_LOG_BASE = LOG_TABLE_BASE;
