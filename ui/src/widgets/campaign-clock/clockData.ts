import type { CampaignClockReply } from '../../generated/dashboard';

export function validateClock(value: unknown): CampaignClockReply {
  const data = value as CampaignClockReply;
  const seconds = (value: unknown) => value === null || typeof value === 'number' && Number.isFinite(value) && value >= 0;
  if (!data || typeof data.available !== 'boolean' || typeof data.complete !== 'boolean'
    || !seconds(data.recordedSeconds) || !seconds(data.playingSeconds) || !seconds(data.pausedSeconds)
    || !Array.isArray(data.warnings) || !data.warnings.every(note => typeof note === 'string')) {
    throw new Error('The campaign clock reply is malformed.');
  }
  return data;
}

export function clockText(seconds: number | null): string {
  if (seconds === null) return 'Unknown';
  const total = Math.floor(seconds);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor(total / 60) % 60;
  return `${String(hours).padStart(2, '0')}:${String(minutes).padStart(2, '0')}:${String(total % 60).padStart(2, '0')}`;
}

export const CLOCK_NOTE = 'Real recorded time for the selected branch. Playing time includes idle time while unpaused. Recording gaps are excluded. During replay, totals advance by recorded game hour, not by time spent watching the replay.';

export interface ClockTotals { recorded: number | null; playing: number | null; paused: number | null; complete: boolean }
export interface ClockRow { id: string; name: string; seconds: number | null; text?: string; note: string }
export const UNCLASSIFIED = 'Some recorded intervals have unknown pause state; playing and paused totals are unavailable.';
export const OVERLAP_REASON = 'Overlapping recordings prevent a unique campaign clock total.';
export const OVERLAP_WARNING = 'Recording intervals overlap; a unique campaign duration cannot be established.';
const GAPS = 'Only recorded intervals are included. Recording gaps are not counted as paused or playing time.';

export const clockWarnings = (totals: Pick<ClockTotals, 'playing' | 'complete'>): string[] =>
  [...(totals.complete ? [] : [GAPS]), ...(totals.playing === null ? [UNCLASSIFIED] : [])];

export function clockRows(totals: ClockTotals, warnings: readonly string[]): ClockRow[] {
  const note = warnings.length ? `${CLOCK_NOTE} ${warnings.join(' ')}` : CLOCK_NOTE;
  return [
    { id: 'total', name: 'Total recorded time', seconds: totals.recorded, note },
    { id: 'playing', name: 'Playing time', seconds: totals.playing, note },
    { id: 'paused', name: 'Paused time', seconds: totals.paused, note },
    { id: 'coverage', name: 'Coverage', seconds: null, text: totals.complete ? 'Complete' : 'Gaps excluded', note },
  ];
}

export function clockStructure(head: ClockTotals | { reason: string } | null): string {
  return !head ? 'none' : 'reason' in head ? `reason:${head.reason}` : `totals:${clockWarnings(head).join('|')}`;
}
