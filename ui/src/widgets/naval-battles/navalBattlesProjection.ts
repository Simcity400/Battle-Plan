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
} from '../../families/table/logHelpers.ts';
import {
  boundedDateLabel,
  formatCountSummary,
  nonBlankText,
  occurrenceCountryCell,
  projectOccurrenceWindow,
  type OccurrenceCountryRole,
  type OccurrenceRow,
  type OccurrenceSort,
} from '../../families/table/logOccurrence.ts';
import type { TableCell, TableColumnSpec, TableModel, TableProjection, TooltipExtraRow } from '../../families/table/model.ts';
import { OFFERED } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { BattleResultKind, BattleSideFigure } from '../land-battles/landBattlesProjection.ts';

export interface NavalBattleOccurrenceRow extends OccurrenceRow {
  readonly battleName: string | null;
  readonly battleNameMissing?: string | null;
  readonly oceanName: string | null;
  readonly oceanMissing?: string | null;
  readonly attacker: OccurrenceCountryRole;
  readonly defender: OccurrenceCountryRole;
  readonly attackerSunk: BattleSideFigure;
  readonly defenderSunk: BattleSideFigure;
  readonly attackerIcLost: BattleSideFigure;
  readonly defenderIcLost: BattleSideFigure;
  readonly attackerShips: BattleSideFigure;
  readonly defenderShips: BattleSideFigure;
  readonly result: BattleResultKind;
  readonly resultMissing?: string | null;
  readonly endedHour: number | null;
}

export interface NavalBattlesProjectionOptions {
  readonly filtered: boolean;
  readonly artwork: boolean;
  readonly sort: OccurrenceSort;
  readonly icLostRecorded: boolean;
}

export function navalBattlesColumns(icLostRecorded: boolean): readonly TableColumnSpec[] {
  return [
    logDateColumn({ sortable: true }),
    logKindColumn({ minWidth: 32, maxWidth: 38 }),
    logCountryColumn(),
    logItemColumn({ label: 'Battle', minWidth: 184 }),
    logPlaceColumn({ label: 'Ocean', minWidth: 124 }),
    {
      key: 'attackerSunk',
      label: 'Attacker',
      group: 'Ships sunk',
      as: 'plain',
      offered: OFFERED.text,
      priority: 1,
      sortable: true,
      numeric: true,
      minWidth: 70,
      widest: ['999'],
    },
    {
      key: 'defenderSunk',
      label: 'Defender',
      group: 'Ships sunk',
      as: 'plain',
      offered: OFFERED.text,
      priority: 1,
      sortable: true,
      numeric: true,
      minWidth: 70,
      widest: ['999'],
    },
    {
      key: 'attackerIcLost',
      label: 'Attacker',
      group: 'IC lost',
      as: 'plain',
      offered: OFFERED.text,
      priority: 2,
      sortable: icLostRecorded,
      numeric: true,
      minWidth: 74,
      widest: ['99,999'],
    },
    {
      key: 'defenderIcLost',
      label: 'Defender',
      group: 'IC lost',
      as: 'plain',
      offered: OFFERED.text,
      priority: 2,
      sortable: icLostRecorded,
      numeric: true,
      minWidth: 74,
      widest: ['99,999'],
    },
    {
      key: 'attackerShips',
      label: 'Attacker',
      group: 'Ships',
      as: 'plain',
      offered: OFFERED.text,
      priority: 3,
      sortable: true,
      numeric: true,
      minWidth: 62,
      widest: ['999'],
    },
    {
      key: 'defenderShips',
      label: 'Defender',
      group: 'Ships',
      as: 'plain',
      offered: OFFERED.text,
      priority: 3,
      sortable: true,
      numeric: true,
      minWidth: 62,
      widest: ['999'],
    },
    logStatusColumn({ label: 'Result', minWidth: 112, widest: ['Attacker won'] }),
  ];
}

function sideColor(readView: BoundedReadView, role: OccurrenceCountryRole): string | null {
  if (!role.tag) return null;
  const who = readView.who('country', role.tag, role.atHour);
  return who.colour?.status === 'ready' ? who.colour.value : null;
}

function navalFigureCell(
  figure: BattleSideFigure,
  other: BattleSideFigure | null,
  ownColor: string | null,
  otherColor: string | null,
): TableCell {
  if (figure.value === null) {
    return { value: notRecorded(figure.missing ?? 'Not recorded') };
  }
  const text = formatScalar(Math.round(figure.value), 'integer');
  const splitBar = other && ownColor
    ? (() => {
        if (other.value === null || !otherColor) {
          return [{ fraction: 1, fill: ownColor, hatched: true }];
        }
        const total = figure.value + other.value;
        const frac = total > 0 ? figure.value / total : 0.5;
        return [
          { fraction: frac, fill: ownColor },
          { fraction: 1 - frac, fill: otherColor },
        ];
      })()
    : undefined;
  return {
    value: knownValue(text),
    ...(splitBar ? { splitBar } : {}),
  };
}

function navalResultCell(result: BattleResultKind, missing?: string | null): { readonly cell: TableCell; readonly label: string } {
  const map: Readonly<Record<Exclude<BattleResultKind, 'unknown'>, { readonly label: string; readonly mark: StatusMarkKind }>> = {
    ongoing: { label: 'Ongoing', mark: 'running' },
    attackerWon: { label: 'Attacker won', mark: 'finished' },
    defenderWon: { label: 'Defender won', mark: 'finished' },
  };
  if (result === 'unknown' || missing) {
    return {
      cell: { value: notRecorded(missing ?? 'Battle result was not recorded'), missingFigure: 'Result' },
      label: 'Result not recorded',
    };
  }
  const info = map[result];
  return { cell: logStatusCell(info.label, info.mark), label: info.label };
}

export const navalBattlesProjection: TableProjection<NavalBattlesProjectionOptions> = {
  kind: 'naval-battles',
  family: 'table',
  project(readView, options): TableModel {
    const columns = navalBattlesColumns(options.icLostRecorded);
    const dateGrouped = options.sort.column === 'date';
    return projectOccurrenceWindow<NavalBattleOccurrenceRow>(
      readView,
      {
        columns,
        hideHeader: false,
        unsorted: false,
        dateGrouped,
        empty: 'No recorded naval battles.',
        emptyWhenFiltered: 'No naval battles match these settings.',
        filtered: options.filtered,
        titleCount: counts => (counts.live && counts.live > 0
          ? formatCountSummary(counts.live, 'ongoing', 'ongoing')
          : formatCountSummary(counts.total, 'battle', 'battles')),
      },
      (row, boundary) => {
        const battleName = nonBlankText(row.battleName);
        const safeName = battleName ?? 'Naval battle not recorded';
        const oceanName = nonBlankText(row.oceanName);
        const resultInfo = navalResultCell(row.result, row.resultMissing);
        const attColor = sideColor(readView, row.attacker);
        const defColor = sideColor(readView, row.defender);
        const sprite = options.artwork ? readView.art({ kind: 'sprite', key: 'GFX_naval_combat_icon', frame: 0 }) : null;
        const icon = sprite?.status === 'ready' ? sprite.url : undefined;
        const iconPending = sprite?.status === 'pending' ? true : undefined;

        const tooltipExtra = (): TooltipExtraRow[] => {
          const out: TooltipExtraRow[] = [
            { label: 'Started', value: knownValue(boundedDateLabel(row.hour, row.exact)) },
            { label: 'Result', value: row.resultMissing ? notRecorded(row.resultMissing) : knownValue(resultInfo.label) },
          ];
          if (row.endedHour !== null) out.push({ label: 'Ended', value: knownValue(boundedDateLabel(row.endedHour, true)) });
          if (row.oceanMissing) out.push({ label: 'Ocean', value: notRecorded(row.oceanMissing) });
          return out;
        };

        return {
          row: {
            key: row.id,
            name: safeName,
            rowHour: row.hour,
            live: row.live,
            cells: {
              date: logDateCell(row.hour, row.exact ? undefined : 'by'),
              kind: options.artwork
                ? { value: knownValue('Naval battle'), icon, iconPending, iconSlot: true, artShape: 'square', artName: 'Naval battle' }
                : { value: notApplicable('Game artwork hidden'), blank: true },
              country: occurrenceCountryCell(readView, [
                { ...row.attacker, dimmed: false },
                { ...row.defender, dimmed: false },
              ], boundary),
              item: battleName !== null
                ? { value: knownValue(battleName) }
                : { value: notRecorded(row.battleNameMissing ?? 'Battle name was not recorded') },
              place: oceanName !== null
                ? { value: knownValue(oceanName) }
                : { value: notRecorded(row.oceanMissing ?? 'Ocean was not recorded'), missingFigure: 'Ocean' },
              attackerSunk: navalFigureCell(row.attackerSunk, row.defenderSunk, attColor, defColor),
              defenderSunk: navalFigureCell(row.defenderSunk, row.attackerSunk, defColor, attColor),
              attackerIcLost: navalFigureCell(row.attackerIcLost, row.defenderIcLost, attColor, defColor),
              defenderIcLost: navalFigureCell(row.defenderIcLost, row.attackerIcLost, defColor, attColor),
              attackerShips: navalFigureCell(row.attackerShips, null, null, null),
              defenderShips: navalFigureCell(row.defenderShips, null, null, null),
              status: resultInfo.cell,
            },
            tooltipTitle: safeName,
            tooltipExtra,
          },
        };
      },
    );
  },
};

export const NAVAL_BATTLES_BASE = LOG_TABLE_BASE;
