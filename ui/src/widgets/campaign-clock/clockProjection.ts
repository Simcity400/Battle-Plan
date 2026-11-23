import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { CampaignClockReply } from '../../generated/dashboard';
import { OFFERED, type TableBase } from '../../draw/tables/settings.ts';
import type { FamilySources } from '../../families/contract.ts';
import { knownValue, notRecorded, valueFromNullable, wireFloat, wireWhole, type Value } from '../../families/value.ts';
import type { TableColumnSpec, TableModel, TableProjection, TableRow } from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { CLOCK_NOTE, UNCLASSIFIED, clockWarnings, validateClock } from './clockData.ts';

export const CLOCK_COLUMNS: readonly TableColumnSpec[] = [
  { key: 'clock', label: 'Clock', as: 'plain', offered: OFFERED.text, priority: 0, text: true, sortable: false, hideable: false, widest: ['Total recorded time'] },
  { key: 'time', label: 'Real time (h:mm:ss)', as: 'plain', offered: ['plain'], priority: 1, numeric: true, format: 'clockSeconds', sortable: false, hideable: false, widest: ['99:59:59', 'Gaps excluded'] },
];

export const CLOCK_TABLE_BASE: TableBase = {
  density: 'comfortable',
  overflow: 'tooltip',
  sort: { column: 'clock', direction: 'ascending' },
};

const CLOCK = { recorded: 0, playing: 1, paused: 2, complete: 3 } as const;

interface ParsedClock {
  readonly available: boolean;
  readonly recorded: Value<number>;
  readonly playing: Value<number>;
  readonly paused: Value<number>;
  readonly complete: boolean;
  readonly warnings: readonly string[];
  readonly refusal?: string;
}

function fromAnswer(answer: FollowedAnswer): ParsedClock {
  if (answer.kind === 'notRecorded') {
    return {
      available: false,
      recorded: notRecorded(answer.reason),
      playing: notRecorded(answer.reason),
      paused: notRecorded(answer.reason),
      complete: false,
      warnings: [],
      refusal: answer.reason,
    };
  }
  if (!answer.rows) {
    const reason = 'Not recorded at this hour';
    return {
      available: false,
      recorded: notRecorded(reason),
      playing: notRecorded(reason),
      paused: notRecorded(reason),
      complete: false,
      warnings: [],
      refusal: reason,
    };
  }
  const recorded = wireFloat(answer, CLOCK.recorded, { missingReason: 'Overlapping recordings prevent a unique campaign clock total.' });
  const playing = wireFloat(answer, CLOCK.playing, { missingReason: UNCLASSIFIED, unrecordedReason: UNCLASSIFIED });
  const paused = wireFloat(answer, CLOCK.paused, { missingReason: UNCLASSIFIED, unrecordedReason: UNCLASSIFIED });
  const completeVal = wireWhole(answer, CLOCK.complete);
  const complete = completeVal.kind === 'known' && completeVal.value === 1n;
  const warnings = clockWarnings({ playing: playing.kind === 'known' || playing.kind === 'zero' ? Number(playing.value) : null, complete });
  return { available: true, recorded, playing, paused, complete, warnings };
}

function fromReply(raw: unknown): ParsedClock {
  const data: CampaignClockReply = validateClock(raw);
  if (!data.available) {
    const reason = 'Playing and paused time were not recorded in this campaign.';
    return { available: false, recorded: notRecorded(reason), playing: notRecorded(reason), paused: notRecorded(reason), complete: false, warnings: data.warnings, refusal: reason };
  }
  if (data.recordedSeconds === null) {
    const reason = 'Overlapping recordings prevent a unique campaign clock total.';
    return { available: false, recorded: notRecorded(reason), playing: notRecorded(reason), paused: notRecorded(reason), complete: false, warnings: data.warnings, refusal: reason };
  }
  return {
    available: true,
    recorded: valueFromNullable(data.recordedSeconds, { kind: 'notRecorded', reason: 'Recorded time unavailable.' }),
    playing: valueFromNullable(data.playingSeconds, { kind: 'notRecorded', reason: UNCLASSIFIED }),
    paused: valueFromNullable(data.pausedSeconds, { kind: 'notRecorded', reason: UNCLASSIFIED }),
    complete: data.complete,
    warnings: data.warnings,
  };
}

export const campaignClockProjection: TableProjection<Record<string, never>> = registerTableProjection({
  kind: 'campaign-clock',
  family: 'table',
  project(sources: FamilySources, hour: number): TableModel {
    const answer = sources.answers.get('main');
    const panel = sources.panels.get('main');
    const parsed = answer !== undefined ? fromAnswer(answer) : panel !== undefined ? fromReply(panel) : null;
    if (!parsed) {
      return { columns: CLOCK_COLUMNS, rows: [], hour, unsorted: true };
    }
    if (!parsed.available || parsed.refusal) {
      return {
        columns: CLOCK_COLUMNS,
        rows: [],
        hour,
        unsorted: true,
        empty: parsed.refusal ?? 'Playing and paused time were not recorded in this campaign.',
        emptyKind: 'unavailable',
        warnings: parsed.warnings,
      };
    }
    const note = parsed.warnings.length ? `${CLOCK_NOTE} ${parsed.warnings.join(' ')}` : CLOCK_NOTE;
    const rows: TableRow[] = [
      { key: 'total', name: 'Total recorded time', cells: { clock: { value: knownValue('Total recorded time') }, time: { value: parsed.recorded, format: 'clockSeconds' } }, tooltipNote: note },
      { key: 'playing', name: 'Playing time', cells: { clock: { value: knownValue('Playing time') }, time: { value: parsed.playing, format: 'clockSeconds' } }, tooltipNote: note },
      { key: 'paused', name: 'Paused time', cells: { clock: { value: knownValue('Paused time') }, time: { value: parsed.paused, format: 'clockSeconds' } }, tooltipNote: note },
      { key: 'coverage', name: 'Coverage', cells: { clock: { value: knownValue('Coverage') }, time: { value: knownValue(parsed.complete ? 'Complete' : 'Gaps excluded') } }, tooltipNote: note },
    ];
    return {
      columns: CLOCK_COLUMNS,
      rows,
      hour,
      unsorted: true,
      warnings: parsed.warnings,
    };
  },
});
