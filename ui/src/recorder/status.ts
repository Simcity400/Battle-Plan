import { formatDate, gameDate, monthNames } from '../dashboard/calendar.ts';
import { formatMegabytes } from '../charts/numberFormat.ts';
import type { Compression } from './compression';

export type Phase = 'waiting' | 'notRecording' | 'recording' | 'saving' | 'saved' | 'discarded' | 'unsaved';
export interface Problem { text: string; detail: string }
export interface OmittedLoads { loads: number; reason: 'off' | 'elsewhere' | 'settings' | 'notAdmitted' | null }
export interface FlagImage { width: number; height: number; pixels: number[] }

export interface Snapshot {
  phase: Phase;
  campaign: string | null;
  recording: string | null;
  name: string | null;
  continued: boolean;
  hour: number | null;
  firstHour: number | null;
  savedBytes: number | null;
  spoolBytes: number | null;
  progress: number | null;
  startedAt: number | null;
  stopping: boolean;
  problem: Problem | null;
  automatic: boolean;
  session: string | null;
  captureMode: CaptureMode | null;
  flag: FlagImage | null;
  omitted?: OmittedLoads | null;
  buildWarning?: string | null;
  preparation?: Preparation | null;
}

export interface Preparation { text: string; failed: boolean }

export type NextLoad = 'continue' | 'newFile';
export type CaptureMode = 'full' | 'map';
export interface CampaignChoice { record?: boolean; next?: NextLoad; folder?: string; flag?: string }
export interface RecorderSettings {
  liveCompression: Compression;
  autoCompress: boolean;
  recordEveryCampaign: boolean;
  autoSave: boolean;
  saveFolder?: string;
  userFolder?: string;
  startWithWindows: boolean;
  startWithReplay: boolean;
  captureMode: CaptureMode;
  campaigns: Record<string, CampaignChoice>;
  effectiveSaveFolder: string;
}

export interface UnsavedRecording {
  session: string;
  campaign: string;
  recording: string | null;
  name: string | null;
  firstHour: number | null;
  lastHour: number | null;
  spoolBytes: number | null;
  status: 'recording' | 'waiting' | 'saving';
  flag: FlagImage | null;
}

export type CompressJob = { state: 'queued' } | { state: 'compressing'; units: number; total: number } | { state: 'kept'; reason: string };

export interface SavedRecording {
  path: string;
  firstHour: number | null;
  lastHour: number | null;
  bytes: number;
  flag: FlagImage | null;
  level: number | null;
  job: CompressJob | null;
}

export const COMPRESSED_LEVEL = 19;

export function compressOffered(settings: Pick<RecorderSettings, 'liveCompression'> | null, row: SavedRecording): boolean {
  return settings?.liveCompression === 'low' && row.level !== COMPRESSED_LEVEL && (row.job == null || row.job.state === 'kept');
}

export function compressText(job: CompressJob | null): string | null {
  if (!job) return null;
  if (job.state === 'queued') return 'Compress waiting';
  if (job.state === 'compressing') return `Compressing ${Math.floor(job.units * 100 / Math.max(job.total, 1))}%`;
  return `Kept as recorded: ${job.reason}`;
}

export interface Recordings { unsaved: UnsavedRecording[]; saved: SavedRecording[] }
export interface RecordingsView { panel: string; visit: number; open: boolean }

export type Dot = 'red' | 'grey' | 'green' | 'spin';
export type HeadAction = 'stop' | 'record' | 'open' | 'recordings';

export interface Head {
  glyph: boolean;
  flag: FlagImage | null;
  title: string;
  dot: Dot | null;
  label: string | null;
  detail: string | null;
  sizes?: string | null;
  action: HeadAction | null;
  campaign: boolean;
  bar?: number | null;
}

const joined = (parts: readonly (string | null)[]) => parts.filter(part => part).join(' · ') || null;

export function dateText(hour: number | null): string | null {
  const date = hour == null ? null : gameDate(hour);
  return date ? formatDate(date) : null;
}

export function sizeText(bytes: number | null): string | null {
  return bytes == null ? null : formatMegabytes(bytes);
}

export function spoolText(bytes: number | null): string | null {
  return bytes == null ? null : `${formatMegabytes(bytes)} this session`;
}

export function savedText(bytes: number | null): string | null {
  return bytes == null ? null : `${formatMegabytes(bytes)} saved`;
}

export function spoolTotalText(unsaved: readonly UnsavedRecording[]): string | null {
  const known = unsaved.filter(recording => recording.spoolBytes != null);
  if (!known.length) return null;
  const total = formatMegabytes(known.reduce((sum, recording) => sum + recording.spoolBytes!, 0));
  return `${known.length < unsaved.length ? 'at least ' : ''}${total} on disk`;
}

export function spanText(first: number | null, last: number | null): string | null {
  const start = first == null ? null : gameDate(first);
  const end = last == null ? null : gameDate(last);
  if (!start || !end) return dateText(last);
  return `${start.year} – ${monthNames[end.month - 1].slice(0, 3)} ${end.year}`;
}

export function omittedText(omitted: OmittedLoads | null | undefined): string | null {
  if (!omitted || omitted.loads < 1) return null;
  const one = omitted.loads === 1;
  const count = `${omitted.loads} ${one ? 'load' : 'loads'} not recorded`;
  switch (omitted.reason) {
    case 'off': return `${count} (Record automatically was off)`;
    case 'elsewhere': return `${count} (${one ? 'it belongs' : 'they belong'} in another file)`;
    case 'settings': return `${count} (the recorder's settings couldn't be read)`;
    case 'notAdmitted': return `${count} (${one ? 'it was' : 'they were'} not identified)`;
    default: return count;
  }
}

export function recordingName(name: string | null): string {
  return name ?? 'New recording';
}

export function campaignTitle(snapshot: Snapshot): string {
  return snapshot.name ?? '';
}

export function head(snapshot: Snapshot): Head {
  const title = campaignTitle(snapshot);
  const named = title !== '';
  const plain = { glyph: false, flag: named ? snapshot.flag : null, title, dot: null, label: null, detail: null, action: null, campaign: false } as const;
  const omitted = omittedText(snapshot.omitted);
  switch (snapshot.phase) {
    case 'waiting':
      return { ...plain, glyph: true, flag: null, title: 'Waiting for the game', detail: snapshot.automatic ? 'Records when you load a campaign' : 'Auto record is off' };
    case 'notRecording':
      return named
        ? { ...plain, dot: 'grey', detail: omitted ?? joined(['Not recording', dateText(snapshot.hour)]), action: 'record', campaign: snapshot.campaign != null }
        : { ...plain, glyph: true, title: 'Not recording', detail: omitted, action: 'record' };
    case 'recording':
      return { ...plain, glyph: !named, dot: 'red', label: 'Recording', detail: dateText(snapshot.hour),
        sizes: joined([spoolText(snapshot.spoolBytes), savedText(snapshot.savedBytes)]), action: snapshot.stopping ? null : 'stop', campaign: snapshot.campaign != null };
    case 'saving':
      return { ...plain, glyph: !named, dot: 'spin', label: 'Saving recording…', bar: snapshot.progress };
    case 'saved':
      return { ...plain, glyph: !named, dot: 'green', label: 'Saved', detail: omitted ?? joined([spanText(snapshot.firstHour, snapshot.hour), sizeText(snapshot.savedBytes)]),
        action: snapshot.recording ? 'open' : null };
    case 'discarded':
      return { ...plain, glyph: !named, dot: 'grey', detail: 'Session discarded', action: 'record' };
    case 'unsaved':
      return { ...plain, glyph: !named, dot: 'grey', label: 'Not saved yet', detail: omitted ?? dateText(snapshot.hour), sizes: spoolText(snapshot.spoolBytes),
        action: 'recordings' };
  }
}

export function sessionLength(startedAt: number | null, now: number): string {
  const minutes = startedAt == null ? 0 : Math.max(0, Math.floor((now - startedAt) / 60_000));
  const hours = Math.floor(minutes / 60), rest = minutes % 60;
  if (!hours) return `${Math.max(1, rest)} min`;
  return rest ? `${hours} h ${rest} min` : `${hours} h`;
}

export function untilNextMinute(startedAt: number | null, now: number): number {
  const elapsed = startedAt == null ? 0 : Math.max(0, now - startedAt);
  return 60_000 - elapsed % 60_000;
}

export function discardConsequence(continued: boolean): string {
  return continued ? 'The earlier recording in this file is kept.' : "This can't be undone.";
}

export function discardDetail(name: string | null, startedAt: number | null, now: number, continued: boolean): string {
  return `Deletes this session's ${sessionLength(startedAt, now)}${name ? ` of ${name}` : ''}. ${discardConsequence(continued)}`;
}

export function waitingDiscardQuestion(continued: boolean): string {
  return `${continued ? 'Discard these hours?' : 'Discard this recording?'} ${discardConsequence(continued)}`;
}

export function promptTitle(quitting: boolean, discarding: boolean): string {
  if (discarding) return quitting ? 'Discard and quit?' : 'Discard this session?';
  return quitting ? 'Quit Battle Plan Recorder?' : 'Stop recording?';
}

export function stopQuestion(name: string | null, startedAt: number | null, now: number): string {
  return `Keep this session's ${sessionLength(startedAt, now)}${name ? ` of ${name}` : ''}?`;
}

export function stopPromptVisible(asking: boolean, snapshot: Snapshot): boolean {
  return asking && snapshot.phase === 'recording' && !snapshot.stopping;
}

export function settingsQueue<T>(read: () => Promise<T>, show: (value: T) => void) {
  let last: Promise<unknown> = Promise.resolve();
  const run = (work: () => Promise<T | null>): Promise<void> => {
    const reply = last.then(work);
    last = reply.catch(() => {});
    return reply.then(value => { if (value != null) show(value); });
  };
  const refresh = () => run(read);
  return {
    refresh,
    save: (work: () => Promise<T | null>) => run(work).catch((error: unknown) => {
      void refresh().catch(() => {});
      throw error;
    }),
  };
}

export function followStatus<T, Stop>(listen: (show: (value: T) => void) => Promise<Stop>, read: () => Promise<T>, show: (value: T) => void) {
  let published = 0;
  const listening = listen(value => { published += 1; show(value); });
  const first = listening.then(async () => {
    const before = published;
    const value = await read();
    if (before === published) show(value);
  });
  return { listening, first };
}

export function recordsAutomatically(settings: RecorderSettings, campaign: string): boolean {
  return settings.campaigns[campaign]?.record ?? settings.recordEveryCampaign;
}

export function nextLoad(settings: RecorderSettings, campaign: string): NextLoad {
  return settings.campaigns[campaign]?.next ?? 'continue';
}

export function whatToRecord(settings: RecorderSettings): string {
  return settings.captureMode === 'map' ? 'Map' : 'Full';
}

export function captureModeFixed(snapshot: Snapshot): boolean {
  return snapshot.captureMode !== null;
}
