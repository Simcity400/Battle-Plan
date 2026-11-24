import {
  LOG_TABLE_BASE,
  logCountryColumn,
  logDateCell,
  logDateColumn,
  logItemColumn,
  logKindColumn,
  logPlaceColumn,
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

export type ShipLossCauseKind = 'battle' | 'airAttack' | 'navalMine' | 'scuttled' | 'other' | 'unknown';

export interface ShipSunkOccurrenceRow extends OccurrenceRow {
  readonly shipName: string | null;
  readonly shipNameMissing?: string | null;
  readonly classToken: string | null;
  readonly className: string | null;
  readonly classMissing?: string | null;
  readonly icon: string | null;
  readonly owner: OccurrenceCountryRole;
  readonly sinker: OccurrenceCountryRole | null;
  readonly seaAreaName: string | null;
  readonly seaAreaMissing?: string | null;
  readonly causeKind: ShipLossCauseKind;
  readonly causeLabel: string | null;
  readonly causeMissing?: string | null;
  readonly battleId?: string | null;
  readonly battleName?: string | null;
}

export interface ShipsSunkProjectionOptions {
  readonly filtered: boolean;
  readonly artwork: boolean;
}

export const SHIPS_SUNK_COLUMNS: readonly TableColumnSpec[] = [
  logDateColumn(),
  logKindColumn({ minWidth: 36, maxWidth: 44 }),
  logCountryColumn(),
  logItemColumn({ label: 'Ship', minWidth: 180 }),
  logPlaceColumn({ label: 'Sea area', minWidth: 136 }),
  {
    key: 'cause',
    label: 'Cause',
    as: 'plain',
    offered: OFFERED.text,
    priority: 1,
    sortable: false,
    minWidth: 160,
    widest: ['Battle of Caribbean Sea'],
  },
];

function shipCauseCell(row: ShipSunkOccurrenceRow): TableCell {
  if (row.causeKind === 'unknown' || row.causeMissing) {
    return {
      value: notRecorded(row.causeMissing ?? 'Cause of sinking was not recorded'),
      missingFigure: 'Cause',
    };
  }
  if (row.causeKind === 'battle') {
    const battle = nonBlankText(row.battleName) ?? nonBlankText(row.causeLabel);
    if (battle) return { value: knownValue(battle) };
    return { value: notRecorded('Battle name was not recorded'), missingFigure: 'Cause' };
  }
  const defaults: Readonly<Record<Exclude<ShipLossCauseKind, 'battle' | 'unknown'>, string>> = {
    airAttack: 'Air attack',
    navalMine: 'Naval mine',
    scuttled: 'Scuttled',
    other: 'Other',
  };
  const label = nonBlankText(row.causeLabel) ?? defaults[row.causeKind];
  return { value: knownValue(label) };
}

export const shipsSunkProjection: TableProjection<ShipsSunkProjectionOptions> = {
  kind: 'ships-sunk',
  family: 'table',
  project(readView, options): TableModel {
    return projectOccurrenceWindow<ShipSunkOccurrenceRow>(
      readView,
      {
        columns: SHIPS_SUNK_COLUMNS,
        empty: 'No ships sunk recorded.',
        emptyWhenFiltered: 'No ships sunk match these settings.',
        filtered: options.filtered,
        titleCount: counts => formatCountSummary(counts.total, 'sunk', 'sunk'),
      },
      (row, boundary) => {
        const shipName = nonBlankText(row.shipName);
        const className = nonBlankText(row.className);
        const safeName = shipName ?? className ?? 'Ship not recorded';
        const seaArea = nonBlankText(row.seaAreaName);
        const artHandle = options.artwork ? urlArtHandle(row.icon) : null;
        const artResult = artHandle ? readView.art(artHandle) : null;
        const icon = artResult?.status === 'ready' ? artResult.url : undefined;
        const iconPending = artResult?.status === 'pending' ? true : undefined;

        const ownerDimmed: OccurrenceCountryRole = { ...row.owner, dimmed: true };
        const roles: OccurrenceCountryRole[] = row.causeKind === 'scuttled' || row.sinker === null
          ? [ownerDimmed]
          : [row.sinker, ownerDimmed];

        const tooltipExtra = (): TooltipExtraRow[] => {
          const out: TooltipExtraRow[] = [
            { label: 'Sunk', value: knownValue(boundedDateLabel(row.hour, row.exact)) },
          ];
          if (className) out.push({ label: 'Class', value: knownValue(className) });
          if (row.classMissing) out.push({ label: 'Class', value: notRecorded(row.classMissing) });
          if (row.seaAreaMissing) out.push({ label: 'Sea area', value: notRecorded(row.seaAreaMissing) });
          if (row.causeMissing) out.push({ label: 'Cause', value: notRecorded(row.causeMissing) });
          return out;
        };

        return {
          row: {
            key: row.id,
            name: safeName,
            rowHour: row.hour,
            live: false,
            cells: {
              date: logDateCell(row.hour, row.exact ? undefined : 'by'),
              kind: options.artwork
                ? {
                    value: className !== null ? knownValue(className) : notRecorded(row.classMissing ?? 'Ship class icon not recorded'),
                    icon,
                    iconPending,
                    iconSlot: true,
                    artShape: 'equipment',
                    artName: className ?? safeName,
                  }
                : { value: notApplicable('Game artwork hidden'), blank: true },
              country: occurrenceCountryCell(readView, roles, boundary),
              item: shipName !== null
                ? { value: knownValue(shipName), ...(className ? { inlineSubtext: className } : {}) }
                : { value: notRecorded(row.shipNameMissing ?? 'Ship name was not recorded') },
              place: seaArea !== null
                ? { value: knownValue(seaArea) }
                : { value: notRecorded(row.seaAreaMissing ?? 'Sea area was not recorded'), missingFigure: 'Sea area' },
              cause: shipCauseCell(row),
            },
            tooltipTitle: safeName,
            tooltipExtra,
          },
        };
      },
    );
  },
};

export const SHIPS_SUNK_BASE = LOG_TABLE_BASE;
