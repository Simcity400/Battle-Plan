export type PreparationPhase = 'idle' | 'queued' | 'preparing' | 'paused' | 'ready' | 'cancelled' | 'failed';
export type PreparationPauseReason = 'requested' | 'newerRecording' | 'diskSpace';
export type PreparationAction = 'status' | 'start' | 'cancel' | 'clear';
export interface PreparationRequest { sourceId: string; revision: string; requestId: string }
export interface PreparationSection { id: string; label: string; state: PreparationPhase; done: string; total: string | null; error: string | null }
export interface PreparationPreferences { backgroundEnabled: boolean }
export interface PreparedCoverage { firstHour: number; lastHour: number; complete: boolean }
export const isPreparedCoverage = (value: unknown): value is PreparedCoverage => !!value && typeof value === 'object'
  && Number.isSafeInteger((value as PreparedCoverage).firstHour) && Number.isSafeInteger((value as PreparedCoverage).lastHour)
  && (value as PreparedCoverage).firstHour <= (value as PreparedCoverage).lastHour && typeof (value as PreparedCoverage).complete === 'boolean';
/** `coverage` is the hourly walk's by branch; `families` each family prepared on its own's, by family then branch. */
export interface PreparationReply extends PreparationRequest { backgroundEnabled: boolean; state: PreparationPhase; sections: PreparationSection[]; coverage?: Record<string, PreparedCoverage>;
  families?: Record<string, Record<string, PreparedCoverage>>; bytes: string; error: string | null; pauseReason?: PreparationPauseReason | null }
const coverageMap = (value: unknown) => !!value && typeof value === 'object' && Object.values(value).every(isPreparedCoverage);
const phases = new Set(['idle', 'queued', 'preparing', 'paused', 'ready', 'cancelled', 'failed']);
const pauseReasons = new Set(['requested', 'newerRecording', 'diskSpace']);
const count = (value: unknown): value is string => typeof value === 'string' && /^(0|[1-9][0-9]*)$/.test(value);
export function validatePreparationReply(reply: PreparationReply, request: PreparationRequest): PreparationReply {
  if (!reply || reply.sourceId !== request.sourceId || reply.revision !== request.revision || reply.requestId !== request.requestId)
    throw new Error('Preparation reply does not match this recording and request.');
  if (typeof reply.backgroundEnabled !== 'boolean' || !phases.has(reply.state) || !count(reply.bytes) || !(reply.error === null || typeof reply.error === 'string') || !Array.isArray(reply.sections)
    || !(reply.pauseReason === undefined || reply.pauseReason === null || pauseReasons.has(reply.pauseReason))
    || (reply.coverage !== undefined && !coverageMap(reply.coverage))
    || (reply.families !== undefined && (!reply.families || typeof reply.families !== 'object' || !Object.values(reply.families).every(coverageMap)))
    || reply.sections.some(section => !section || typeof section.id !== 'string' || typeof section.label !== 'string' || !phases.has(section.state)
      || !count(section.done) || !(section.total === null || count(section.total)) || !(section.error === null || typeof section.error === 'string')))
    throw new Error('Invalid preparation progress.');
  return reply;
}
export const pauseExplanation = (reason: PreparationPauseReason | null | undefined) => `Paused ${({
  requested: 'while preparation you asked for runs', newerRecording: 'while a newer recording prepares first',
  diskSpace: 'because less than a tenth of the disk holding prepared data is free',
} as Record<string, string>)[reason ?? ''] ?? 'for now'}. It resumes automatically.`;
export const preparationActive = (reply: PreparationReply | null) => reply?.state === 'queued' || reply?.state === 'preparing';
export const preparationLabel = (state: PreparationPhase) => ({ idle: 'Not prepared', queued: 'Waiting to prepare', preparing: 'Preparing', paused: 'Paused', ready: 'Ready', cancelled: 'Cancelled', failed: 'Preparation failed' })[state];
export function progressPercent(done: string, total: string | null): number | undefined {
  if (total === null || total === '0') return undefined;
  return Number(BigInt(done) * 10000n / BigInt(total) > 10000n ? 10000n : BigInt(done) * 10000n / BigInt(total)) / 100;
}

export function validatePreparationPreferences(reply: PreparationPreferences): PreparationPreferences {
  if (!reply || typeof reply.backgroundEnabled !== 'boolean') throw new Error('Invalid background preparation preference.');
  return reply;
}
