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
