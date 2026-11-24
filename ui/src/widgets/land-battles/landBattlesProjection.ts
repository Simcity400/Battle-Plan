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

export type BattleResultKind = 'ongoing' | 'attackerWon' | 'defenderWon' | 'unknown';

export interface BattleSideFigure {
  readonly value: number | null;
  readonly missing?: string | null;
  readonly partial?: string | null;
}

export interface LandBattleOccurrenceRow extends OccurrenceRow {
  readonly battleName: string | null;
  readonly battleNameMissing?: string | null;
  readonly provinceName: string | null;
  readonly provinceMissing?: string | null;
  readonly attacker: OccurrenceCountryRole;
  readonly defender: OccurrenceCountryRole;
  readonly attackerCoalition?: readonly OccurrenceCountryRole[];
  readonly defenderCoalition?: readonly OccurrenceCountryRole[];
  readonly attackerCasualties: BattleSideFigure;
  readonly defenderCasualties: BattleSideFigure;
  readonly attackerIcLost: BattleSideFigure;
  readonly defenderIcLost: BattleSideFigure;
  readonly attackerDivisions: BattleSideFigure;
  readonly defenderDivisions: BattleSideFigure;
  readonly result: BattleResultKind;
  readonly resultMissing?: string | null;
  readonly endedHour: number | null;
}

export interface LandBattlesProjectionOptions {
  readonly filtered: boolean;
  readonly artwork: boolean;
  readonly sort: OccurrenceSort;
  readonly icLostRecorded: boolean;
}

export function landBattlesColumns(icLostRecorded: boolean): readonly TableColumnSpec[] {
  return [
    logDateColumn({ sortable: true }),
    logKindColumn({ minWidth: 32, maxWidth: 38 }),
    logCountryColumn(),
    logItemColumn({ label: 'Battle', minWidth: 168 }),
    logPlaceColumn({ label: 'Province', minWidth: 104 }),
    {
      key: 'attackerCasualties',
      label: 'Attacker',
      group: 'Casualties',
      as: 'plain',
      offered: OFFERED.text,
      priority: 1,
      sortable: true,
      numeric: true,
      minWidth: 78,
      widest: ['99,999'],
    },
    {
      key: 'defenderCasualties',
      label: 'Defender',
      group: 'Casualties',
      as: 'plain',
      offered: OFFERED.text,
      priority: 1,
      sortable: true,
      numeric: true,
      minWidth: 78,
      widest: ['99,999'],
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
      minWidth: 68,
      widest: ['9,999'],
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
      minWidth: 68,
      widest: ['9,999'],
    },
    {
      key: 'attackerDivisions',
      label: 'Attacker',
      group: 'Divisions',
      as: 'plain',
      offered: OFFERED.text,
      priority: 3,
      sortable: true,
      numeric: true,
      minWidth: 62,
      widest: ['99'],
    },
    {
      key: 'defenderDivisions',
      label: 'Defender',
      group: 'Divisions',
      as: 'plain',
      offered: OFFERED.text,
      priority: 3,
      sortable: true,
      numeric: true,
      minWidth: 62,
      widest: ['99'],
    },
    logStatusColumn({ label: 'Result', minWidth: 112, widest: ['Attacker won'] }),
  ];
}

function sideColor(readView: BoundedReadView, role: OccurrenceCountryRole): string | null {
  if (!role.tag) return null;
  const who = readView.who('country', role.tag, role.atHour);
  return who.colour?.status === 'ready' ? who.colour.value : null;
}

function battleFigureCell(
  figure: BattleSideFigure,
  other: BattleSideFigure | null,
  ownColor: string | null,
  otherColor: string | null,
): TableCell {
  if (figure.value === null) {
    return { value: notRecorded(figure.missing ?? 'Not recorded') };
  }
  const text = formatScalar(Math.round(figure.value), 'integer');
  const splitBar = other && ownColor && otherColor
    ? (() => {
        if (other.value === null) {
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

function battleResultCell(result: BattleResultKind, missing?: string | null): { readonly cell: TableCell; readonly label: string } {
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

export const landBattlesProjection: TableProjection<LandBattlesProjectionOptions> = {
  kind: 'land-battles',
  family: 'table',
  project(readView, options): TableModel {
    const columns = landBattlesColumns(options.icLostRecorded);
    const dateGrouped = options.sort.column === 'date';
    return projectOccurrenceWindow<LandBattleOccurrenceRow>(
      readView,
      {
        columns,
        hideHeader: false,
        unsorted: false,
        dateGrouped,
        empty: 'No recorded land battles.',
        emptyWhenFiltered: 'No land battles match these settings.',
        filtered: options.filtered,
        titleCount: counts => (counts.live && counts.live > 0
          ? formatCountSummary(counts.live, 'ongoing', 'ongoing')
          : formatCountSummary(counts.total, 'battle', 'battles')),
      },
      (row, boundary) => {
        const battleName = nonBlankText(row.battleName);
        const safeName = battleName ?? 'Battle name not recorded';
        const provinceName = nonBlankText(row.provinceName);
        const resultInfo = battleResultCell(row.result, row.resultMissing);
        const attColor = sideColor(readView, row.attacker);
        const defColor = sideColor(readView, row.defender);
        const sprite = options.artwork ? readView.art({ kind: 'sprite', key: 'GFX_land_battle_icon', frame: 0 }) : null;
        const icon = sprite?.status === 'ready' ? sprite.url : undefined;
        const iconPending = sprite?.status === 'pending' ? true : undefined;

        const tooltipExtra = (): TooltipExtraRow[] => {
          const out: TooltipExtraRow[] = [
            { label: 'Started', value: knownValue(boundedDateLabel(row.hour, row.exact)) },
            { label: 'Result', value: row.resultMissing ? notRecorded(row.resultMissing) : knownValue(resultInfo.label) },
          ];
          if (row.endedHour !== null) out.push({ label: 'Ended', value: knownValue(boundedDateLabel(row.endedHour, true)) });
          if (row.attackerCoalition?.length) {
            out.push({ label: 'Attackers', value: knownValue(row.attackerCoalition.map(c => c.tag ?? 'Not recorded').join(', ')) });
          }
          if (row.defenderCoalition?.length) {
            out.push({ label: 'Defenders', value: knownValue(row.defenderCoalition.map(c => c.tag ?? 'Not recorded').join(', ')) });
          }
          if (row.provinceMissing) out.push({ label: 'Province', value: notRecorded(row.provinceMissing) });
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
                ? { value: knownValue('Land battle'), icon, iconPending, iconSlot: true, artShape: 'square', artName: 'Land battle' }
                : { value: notApplicable('Game artwork hidden'), blank: true },
              country: occurrenceCountryCell(readView, [
                { ...row.attacker, dimmed: false },
                { ...row.defender, dimmed: false },
              ], boundary),
              item: battleName !== null
                ? { value: knownValue(battleName) }
                : { value: notRecorded(row.battleNameMissing ?? 'Battle name was not recorded') },
              place: provinceName !== null
                ? { value: knownValue(provinceName) }
                : { value: notRecorded(row.provinceMissing ?? 'Province was not recorded'), missingFigure: 'Province' },
              attackerCasualties: battleFigureCell(row.attackerCasualties, row.defenderCasualties, attColor, defColor),
              defenderCasualties: battleFigureCell(row.defenderCasualties, row.attackerCasualties, defColor, attColor),
              attackerIcLost: battleFigureCell(row.attackerIcLost, row.defenderIcLost, attColor, defColor),
              defenderIcLost: battleFigureCell(row.defenderIcLost, row.attackerIcLost, defColor, attColor),
              attackerDivisions: battleFigureCell(row.attackerDivisions, null, null, null),
              defenderDivisions: battleFigureCell(row.defenderDivisions, null, null, null),
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

export const LAND_BATTLES_BASE = LOG_TABLE_BASE;
