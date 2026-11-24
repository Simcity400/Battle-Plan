import type { StatusMarkKind } from '../../draw/kit.ts';
import {
  LOG_TABLE_BASE,
  logCountryColumn,
  logDateCell,
  logDateColumn,
  logItemColumn,
  logKindColumn,
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

export type EspionageOccurrenceStatus =
  | 'running'
  | 'preparing'
  | 'succeeded'
  | 'failed'
  | 'cancelled'
  | 'ended'
  | string;

export interface EspionageOccurrenceRow extends OccurrenceRow {
  readonly operationToken: string | null;
  readonly operationName: string | null;
  readonly operationMissing?: string | null;
  readonly icon: string | null;
  readonly owner: OccurrenceCountryRole;
  readonly target: OccurrenceCountryRole;
  readonly status: EspionageOccurrenceStatus;
  readonly statusLabel?: string | null;
  readonly outcomeMissing?: string | null;
  readonly startedHour: number | null;
  readonly endedHour: number | null;
  readonly durationDays: number | null;
  readonly elapsedDays: number | null;
  readonly preparationFraction: number | null;
  readonly progressMissing?: string | null;
  readonly assignedSlots?: number | null;
}

export interface EspionageDetailSubject {
  readonly id: string;
  readonly asOfHour: number;
  readonly operationName: string | null;
  readonly ownerTag: string | null;
  readonly targetTag: string | null;
  readonly statusLabel: string;
}

export interface EspionageLogProjectionOptions {
  readonly filtered: boolean;
  readonly artwork: boolean;
}

export const ESPIONAGE_LOG_COLUMNS: readonly TableColumnSpec[] = [
  logDateColumn(),
  logKindColumn({ minWidth: 42, maxWidth: 48 }),
  logCountryColumn(),
  logItemColumn({ label: 'Operation', minWidth: 180 }),
  {
    key: 'progress',
    label: 'Progress',
    as: 'plain',
    cellKind: 'track',
    trackTone: 'accent',
    offered: OFFERED.text,
    priority: 2,
    sortable: false,
    numeric: true,
    minWidth: 144,
    widest: ['157 days left'],
  },
  logStatusColumn({ minWidth: 148, widest: ['Outcome not recorded'] }),
];

const KNOWN_STATUS: Readonly<Record<string, { readonly label: string; readonly mark: StatusMarkKind; readonly muted?: boolean }>> = {
  running: { label: 'Running', mark: 'running' },
  preparing: { label: 'Preparing', mark: 'running' },
  succeeded: { label: 'Succeeded', mark: 'succeeded' },
  failed: { label: 'Failed', mark: 'ended', muted: true },
  cancelled: { label: 'Cancelled', mark: 'ended', muted: true },
  ended: { label: 'Ended', mark: 'ended', muted: true },
};

function espionageProgressCell(row: EspionageOccurrenceRow, atHour: number): { readonly cell: TableCell; readonly nextChangeAt: number } {
  if (row.status === 'running') {
    if (row.durationDays === null || row.durationDays <= 0) {
      return {
        cell: { value: notRecorded(row.progressMissing ?? 'Operation duration was not recorded'), missingFigure: 'Duration' },
        nextChangeAt: Infinity,
      };
    }
    const baseHour = row.startedHour ?? row.hour;
    const elapsed = row.elapsedDays !== null ? row.elapsedDays : Math.max(0, Math.floor((atHour - baseHour) / 24));
    const left = Math.max(0, row.durationDays - elapsed);
    const fraction = Math.max(0, Math.min(1, left / row.durationDays));
    const nextDay = baseHour + (elapsed + 1) * 24;
    return {
      cell: {
        value: knownValue(formatScalar(left, 'integer')),
        unitSuffix: left === 1 ? 'day left' : 'days left',
        bar: { fraction: knownValue(fraction) },
      },
      nextChangeAt: left > 0 && nextDay > atHour ? nextDay : Infinity,
    };
  }
  if (row.status === 'preparing') {
    if (row.preparationFraction === null) {
      return {
        cell: { value: notRecorded(row.progressMissing ?? 'Preparation progress was not recorded'), missingFigure: 'Preparation' },
        nextChangeAt: Infinity,
      };
    }
    return {
      cell: {
        value: knownValue(formatProgress(row.preparationFraction)),
        unitSuffix: 'prepared',
      },
      nextChangeAt: Infinity,
    };
  }
  if (row.durationDays !== null && row.durationDays >= 0) {
    return {
      cell: {
        value: knownValue(formatScalar(row.durationDays, 'integer')),
        unitSuffix: row.durationDays === 1 ? 'day' : 'days',
      },
      nextChangeAt: Infinity,
    };
  }
  return {
    cell: { value: notRecorded(row.progressMissing ?? 'Operation length was not recorded'), missingFigure: 'Length' },
    nextChangeAt: Infinity,
  };
}

function espionageStatus(row: EspionageOccurrenceRow): { readonly cell: TableCell; readonly label: string; readonly muted: boolean } {
  if (row.outcomeMissing) {
    return {
      cell: { value: notRecorded(row.outcomeMissing), missingFigure: 'Outcome' },
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

export const espionageLogProjection: TableProjection<EspionageLogProjectionOptions> = {
  kind: 'espionage-log',
  family: 'table',
  project(readView, options): TableModel {
    const hour = readView.hour;
    return projectOccurrenceWindow<EspionageOccurrenceRow>(
      readView,
      {
        columns: ESPIONAGE_LOG_COLUMNS,
        empty: 'No recorded espionage operations.',
        emptyWhenFiltered: 'No espionage operations match these settings.',
        filtered: options.filtered,
        titleCount: counts => formatCountSummary(counts.live, 'under way', 'under way'),
      },
      (row, boundary) => {
        const opName = nonBlankText(row.operationName);
        const safeName = opName ?? nonBlankText(row.operationToken) ?? 'Operation not recorded';
        const statusInfo = espionageStatus(row);
        const progress = espionageProgressCell(row, hour);
        const artHandle = options.artwork ? urlArtHandle(row.icon) : null;
        const artResult = artHandle ? readView.art(artHandle) : null;
        const icon = artResult?.status === 'ready' ? artResult.url : undefined;
        const iconPending = artResult?.status === 'pending' ? true : undefined;

        const meta: EspionageDetailSubject = {
          id: row.id,
          asOfHour: hour,
          operationName: opName,
          ownerTag: row.owner.tag,
          targetTag: row.target.tag,
          statusLabel: statusInfo.label,
        };

        const tooltipExtra = (): TooltipExtraRow[] => {
          const out: TooltipExtraRow[] = [
            { label: 'Recorded', value: knownValue(boundedDateLabel(row.hour, row.exact)) },
            { label: 'Status', value: row.outcomeMissing ? notRecorded(row.outcomeMissing) : knownValue(statusInfo.label) },
          ];
          if (row.startedHour !== null) out.push({ label: 'Started', value: knownValue(boundedDateLabel(row.startedHour, true)) });
          if (row.endedHour !== null) out.push({ label: 'Ended', value: knownValue(boundedDateLabel(row.endedHour, row.exact)) });
          if (row.assignedSlots != null) out.push({ label: 'Operatives', value: knownValue(formatScalar(row.assignedSlots, 'integer')) });
          if (row.operationMissing) out.push({ label: 'Operation type', value: notRecorded(row.operationMissing) });
          if (row.progressMissing) out.push({ label: 'Progress', value: notRecorded(row.progressMissing) });
          return out;
        };

        return {
          nextChangeAt: progress.nextChangeAt,
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
                    value: opName !== null ? knownValue(opName) : notRecorded(row.operationMissing ?? 'Operation picture not recorded'),
                    icon,
                    iconPending,
                    iconSlot: true,
                    artShape: 'wide',
                    artName: safeName,
                  }
                : { value: notApplicable('Game artwork hidden'), blank: true },
              country: occurrenceCountryCell(readView, [row.owner, row.target], boundary),
              item: opName !== null
                ? { value: knownValue(opName) }
                : { value: notRecorded(row.operationMissing ?? 'Operation name was not recorded') },
              progress: progress.cell,
              status: statusInfo.cell,
            },
            tooltipTitle: safeName,
            tooltipExtra,
            tooltipAction: 'Click to view operation',
            meta,
          },
        };
      },
    );
  },
};

export const ESPIONAGE_LOG_BASE = LOG_TABLE_BASE;
