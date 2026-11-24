import {
  LOG_TABLE_BASE,
  logActionColumn,
  logCountryColumn,
  logDateCell,
  logDateColumn,
  logItemColumn,
  logKindColumn,
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
import type { TableCell, TableColumnSpec, TableModel, TableProjection, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import { OFFERED } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';

export interface PeaceTermCount {
  readonly kind: string;
  readonly label: string;
  readonly count: number;
}

export interface PeaceWinnerState {
  readonly id: string;
  readonly stateName: string | null;
  readonly stateMissing?: string | null;
  readonly formerOwner: OccurrenceCountryRole;
}

export interface PeaceWinnerGroup {
  readonly id: string;
  readonly winner: OccurrenceCountryRole;
  readonly summary: string;
  readonly states: readonly PeaceWinnerState[];
}

export interface PeaceDealOccurrenceRow extends OccurrenceRow {
  readonly warName: string | null;
  readonly warNameMissing?: string | null;
  readonly winners: readonly OccurrenceCountryRole[];
  readonly losers: readonly OccurrenceCountryRole[];
  readonly whitePeace: boolean;
  readonly terms: readonly PeaceTermCount[];
  readonly termsMissing?: string | null;
  readonly expandedGroups?: readonly PeaceWinnerGroup[] | null;
  readonly hasTermsDetail: boolean;
}

export interface PeaceDealDetailSubject {
  readonly id: string;
  readonly asOfHour: number;
  readonly warName: string | null;
}

export interface PeaceDealsProjectionOptions {
  readonly filtered: boolean;
  readonly artwork: boolean;
}

export const PEACE_DEALS_COLUMNS: readonly TableColumnSpec[] = [
  logDateColumn(),
  logKindColumn({ minWidth: 34, maxWidth: 42 }),
  logCountryColumn(),
  logItemColumn({ label: 'War', minWidth: 180 }),
  {
    key: 'terms',
    label: 'Terms',
    as: 'plain',
    offered: OFFERED.text,
    priority: 1,
    sortable: false,
    minWidth: 148,
    widest: ['12 states, 2 puppets'],
  },
  logActionColumn({ minWidth: 28, maxWidth: 34 }),
];

function termsCell(row: PeaceDealOccurrenceRow): TableCell {
  if (row.termsMissing) {
    return { value: notRecorded(row.termsMissing), missingFigure: 'Terms' };
  }
  if (row.whitePeace || row.terms.length === 0) {
    return { value: knownValue('White peace') };
  }
  if (row.terms.length === 1) {
    const only = row.terms[0]!;
    return {
      value: knownValue(formatScalar(only.count, 'integer')),
      unitSuffix: only.label,
    };
  }
  const text = row.terms.map(t => `${formatScalar(t.count, 'integer')} ${t.label}`).join(', ');
  return { value: knownValue(text) };
}

export const peaceDealsProjection: TableProjection<PeaceDealsProjectionOptions> = {
  kind: 'peace-deals',
  family: 'table',
  project(readView, options): TableModel {
    const hour = readView.hour;
    return projectOccurrenceWindow<PeaceDealOccurrenceRow>(
      readView,
      {
        columns: PEACE_DEALS_COLUMNS,
        empty: 'No recorded peace deals.',
        emptyWhenFiltered: 'No peace deals match these settings.',
        filtered: options.filtered,
        titleCount: counts => formatCountSummary(counts.total, 'peace deal', 'peace deals'),
      },
      (row, boundary) => {
        const warName = nonBlankText(row.warName);
        const safeName = warName ?? 'War name not recorded';
        const primaryWinner: OccurrenceCountryRole = row.winners[0] ?? { tag: null, atHour: row.hour, missingReason: 'Winner not recorded' };
        const primaryLoser: OccurrenceCountryRole = row.losers[0]
          ? { ...row.losers[0], dimmed: true }
          : { tag: null, atHour: row.hour, dimmed: true, missingReason: 'Loser not recorded' };
        const sprite = options.artwork ? readView.art({ kind: 'sprite', key: 'GFX_relation_truce', frame: 0 }) : null;
        const icon = sprite?.status === 'ready' ? sprite.url : undefined;
        const iconPending = sprite?.status === 'pending' ? true : undefined;

        const children: TableRow[] = [];
        if (row.expandedGroups?.length) {
          const blank: TableCell = { value: notApplicable('Expanded peace term'), blank: true };
          for (const group of row.expandedGroups) {
            children.push({
              key: `${row.id}:winner:${group.id}`,
              name: group.winner.tag ?? 'Winner not recorded',
              rowHour: row.hour,
              cells: {
                date: blank,
                kind: blank,
                country: occurrenceCountryCell(readView, [group.winner], boundary),
                item: { value: knownValue(group.winner.tag ?? 'Winner not recorded'), inlineSubtext: group.summary },
                terms: blank,
                action: blank,
              },
            });
            for (const st of group.states) {
              const stName = nonBlankText(st.stateName);
              children.push({
                key: `${row.id}:state:${group.id}:${st.id}`,
                name: stName ?? 'State not recorded',
                rowHour: row.hour,
                cells: {
                  date: blank,
                  kind: blank,
                  country: occurrenceCountryCell(readView, [st.formerOwner], boundary),
                  item: stName !== null
                    ? { value: knownValue(stName) }
                    : { value: notRecorded(st.stateMissing ?? 'State name not recorded') },
                  terms: blank,
                  action: blank,
                },
              });
            }
          }
        }

        const tooltipExtra = (): TooltipExtraRow[] => {
          const out: TooltipExtraRow[] = [
            { label: 'Signed', value: knownValue(boundedDateLabel(row.hour, row.exact)) },
            { label: 'Winners', value: row.winners.length ? knownValue(row.winners.map(w => w.tag ?? 'Not recorded').join(', ')) : notRecorded('Winners not recorded') },
            { label: 'Losers', value: row.losers.length ? knownValue(row.losers.map(l => l.tag ?? 'Not recorded').join(', ')) : notRecorded('Losers not recorded') },
          ];
          if (row.warNameMissing) out.push({ label: 'War name', value: notRecorded(row.warNameMissing) });
          if (row.termsMissing) out.push({ label: 'Terms', value: notRecorded(row.termsMissing) });
          return out;
        };

        const meta: PeaceDealDetailSubject = { id: row.id, asOfHour: hour, warName };

        return {
          row: {
            key: row.id,
            name: safeName,
            rowHour: row.hour,
            live: false,
            cells: {
              date: logDateCell(row.hour, row.exact ? undefined : 'by'),
              kind: options.artwork
                ? { value: knownValue('Peace deal'), icon, iconPending, iconSlot: true, artShape: 'square', artName: 'Peace deal' }
                : { value: notApplicable('Game artwork hidden'), blank: true },
              country: occurrenceCountryCell(readView, [primaryWinner, primaryLoser], boundary),
              item: warName !== null
                ? { value: knownValue(warName) }
                : { value: notRecorded(row.warNameMissing ?? 'War name was not recorded') },
              terms: termsCell(row),
              action: { value: notApplicable(row.hasTermsDetail ? 'Expand peace terms' : 'No peace terms'), blank: true },
            },
            ...(children.length > 0 ? { children } : {}),
            tooltipTitle: safeName,
            tooltipExtra,
            ...(row.hasTermsDetail ? { tooltipAction: 'Click to view peace terms' } : {}),
            meta,
          },
        };
      },
    );
  },
};

export const PEACE_DEALS_BASE = LOG_TABLE_BASE;
