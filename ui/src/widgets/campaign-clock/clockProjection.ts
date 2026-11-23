import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { CampaignClockReply } from '../../generated/dashboard';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { declareTableProjection, type DeclarativeColumnSpec, type TableProjection } from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { OFFERED, type TableBase } from '../../families/table/settings.ts';
import { knownValue, notRecorded, valueFromNullable, wireFloat, wireWhole, type Value } from '../../families/value.ts';
import { CLOCK_NOTE, OVERLAP_REASON, OVERLAP_WARNING, UNCLASSIFIED, clockWarnings, validateClock } from './clockData.ts';

interface ClockRow { readonly id: string; readonly label: string; readonly value: Value; readonly note: string }
interface ParsedClock { readonly refusal?: string; readonly recorded: Value<number>; readonly playing: Value<number>; readonly paused: Value<number>; readonly complete: boolean; readonly warnings: readonly string[] }

const CLOCK = { recorded: 0, playing: 1, paused: 2, complete: 3 } as const;
const EMPTY_OPTIONS: Record<string, never> = {};

export const CLOCK_COLUMNS: readonly DeclarativeColumnSpec<ClockRow, Record<string, never>>[] = [
  { key: 'clock', label: 'Clock', as: 'plain', offered: OFFERED.text, priority: 0, text: true, sortable: false, hideable: false, widest: ['Total recorded time'], path: 'label' },
  { key: 'time', label: 'Real time (h:mm:ss)', as: 'plain', offered: ['plain'], priority: 1, numeric: true, format: 'clockSeconds', sortable: false, hideable: false, widest: ['99:59:59', 'Gaps excluded'], value: row => row.value },
];

export const CLOCK_TABLE_BASE: TableBase = { density: 'comfortable', overflow: 'tooltip', sort: { column: 'clock', direction: 'ascending' } };

const refusedClock = (reason: string, warnings: readonly string[] = []): ParsedClock => {
  const missing = notRecorded(reason);
  return { refusal: reason, recorded: missing, playing: missing, paused: missing, complete: false, warnings };
};

function parseAnswer(answer: FollowedAnswer): ParsedClock {
  if (answer.kind === 'notRecorded') {
    return refusedClock(answer.reason, answer.reason === OVERLAP_REASON ? [OVERLAP_WARNING] : []);
  }
  if (!answer.rows) return refusedClock('Not recorded at this hour');
  const recorded = wireFloat(answer, CLOCK.recorded, { missingReason: OVERLAP_REASON });
  const playing = wireFloat(answer, CLOCK.playing, { missingReason: UNCLASSIFIED, unrecordedReason: UNCLASSIFIED });
  const paused = wireFloat(answer, CLOCK.paused, { missingReason: UNCLASSIFIED, unrecordedReason: UNCLASSIFIED });
  const completeVal = wireWhole(answer, CLOCK.complete);
  const complete = completeVal.kind === 'known' && completeVal.value === 1n;
  return { recorded, playing, paused, complete, warnings: clockWarnings({ playing: playing.kind === 'known' || playing.kind === 'zero' ? Number(playing.value) : null, complete }) };
}

function parseReply(raw: unknown): ParsedClock {
  const data: CampaignClockReply = validateClock(raw);
  if (!data.available) return refusedClock('Playing and paused time were not recorded in this campaign.', data.warnings);
  if (data.recordedSeconds === null) return refusedClock(OVERLAP_REASON, data.warnings);
  return {
    recorded: valueFromNullable(data.recordedSeconds, { kind: 'notRecorded', reason: 'Recorded time unavailable.' }),
    playing: valueFromNullable(data.playingSeconds, { kind: 'notRecorded', reason: UNCLASSIFIED }),
    paused: valueFromNullable(data.pausedSeconds, { kind: 'notRecorded', reason: UNCLASSIFIED }),
    complete: data.complete,
    warnings: data.warnings,
  };
}

export const campaignClockProjection: TableProjection<Record<string, never>> = registerTableProjection(
  declareTableProjection<ParsedClock, ClockRow, Record<string, never>>({
    kind: 'campaign-clock',
    validate: parseReply,
    fromAnswer: parseAnswer,
    unsorted: true,
    columns: CLOCK_COLUMNS,
    rows: parsed => {
      if (parsed.refusal) return { rows: [], empty: parsed.refusal, emptyKind: 'unavailable', warnings: parsed.warnings };
      const note = parsed.warnings.length ? `${CLOCK_NOTE} ${parsed.warnings.join(' ')}` : CLOCK_NOTE;
      return {
        rows: [
          { id: 'total', label: 'Total recorded time', value: parsed.recorded, note },
          { id: 'playing', label: 'Playing time', value: parsed.playing, note },
          { id: 'paused', label: 'Paused time', value: parsed.paused, note },
          { id: 'coverage', label: 'Coverage', value: knownValue(parsed.complete ? 'Complete' : 'Gaps excluded'), note },
        ],
        warnings: parsed.warnings,
      };
    },
    rowKey: row => row.id,
    rowName: row => row.label,
    tooltipNote: row => row.note,
  }),
);

export const campaignClockDeclaration: TableWidgetDeclaration<Record<string, never>> = {
  family: 'table',
  kind: 'campaign-clock',
  label: 'Recorded campaign clock times',
  subject: 'campaign clock',
  emptyCampaign: 'Load a campaign to see its recorded clock times.',
  icon: 'clock',
  countries: { mode: 'none' },
  source: {
    kind: 'head',
    product: 'campaign-clock',
    timeShape: 'branch-to-hour',
    followQuery: ctx => ({ head: 'campaign-clock', params: { branchId: ctx.branch.id, firstHour: ctx.branch.firstHour, lastHour: ctx.branch.lastHour } }),
    productParams: (ctx, span) => ({ branchId: ctx.branch.id, ...span }),
    dynamicKeys: ['firstHour', 'lastHour'],
    validate: validateClock,
  },
  projection: campaignClockProjection,
  columns: CLOCK_COLUMNS,
  settings: { kind: 'fixed', base: CLOCK_TABLE_BASE },
  projectionOptions: () => EMPTY_OPTIONS,
};
