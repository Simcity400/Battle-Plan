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

export const CLOCK_NOTE = 'Real recorded time for the selected branch. Playing time includes idle time while unpaused. Totals use recorded clock intervals. During replay, totals advance by recorded game hour, not by time spent watching the replay.';

export interface ClockTotals { recorded: number | null; playing: number | null; paused: number | null; complete: boolean }
export const UNCLASSIFIED = 'Some recorded intervals have unknown pause state; playing and paused totals are unavailable.';
export const OVERLAP_REASON = 'Overlapping recordings prevent a unique campaign clock total.';
export const OVERLAP_WARNING = 'Recording intervals overlap; a unique campaign duration cannot be established.';
export const INCOMPLETE_WARNING = 'Coverage is incomplete. Totals include measured clock intervals only.';

export const clockWarnings = (totals: Pick<ClockTotals, 'playing' | 'complete'>): string[] =>
  [...(totals.complete ? [] : [INCOMPLETE_WARNING]), ...(totals.playing === null ? [UNCLASSIFIED] : [])];
