import { formatDate, gameDate, monthNames } from '../dashboard/calendar.ts';
import { formatMegabytes } from '../charts/numberFormat.ts';
import type { Compression } from './compression';

export type Phase = 'waiting' | 'notRecording' | 'recording' | 'saving' | 'saved' | 'discarded' | 'unsaved';
export interface Problem { text: string; detail: string; failed: boolean }
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
  startedAt: number | null;
  stopping: boolean;
  problem: Problem | null;
  automatic: boolean;
  session: string | null;
  captureMode: CaptureMode | null;
  flag: FlagImage | null;
  omitted?: OmittedLoads | null;
  preparation?: Preparation | null;
  sessionKey?: number | null;
  prompt?: PanelPrompt | null;
  revision: number;
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
  warning?: string | null;
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

export type CompressJob = { state: 'queued' } | { state: 'compressing'; units: number; total: number } | { state: 'kept'; reason: string }
  | { state: 'unconfirmed'; reason: string };

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
  return settings?.liveCompression === 'low' && row.level !== COMPRESSED_LEVEL && (row.job == null || row.job.state === 'kept' || row.job.state === 'unconfirmed');
}

export function compressText(job: CompressJob | null): string | null {
  if (!job) return null;
  if (job.state === 'queued') return 'Waiting to compress';
  if (job.state === 'compressing') return `Compressing ${Math.floor(job.units * 100 / Math.max(job.total, 1))}%`;
  if (job.state === 'unconfirmed') return `Compression not confirmed: ${keptClause(job.reason).replace(/\.$/u, '')}`;
  return `Kept as recorded: ${keptClause(job.reason).replace(/\.$/u, '')}`;
}

const keptClause = (reason: string) => reason.split(': ', 1)[0];

export function compressDetail(job: CompressJob | null): string | undefined {
  return (job?.state === 'kept' || job?.state === 'unconfirmed') && keptClause(job.reason) !== job.reason ? job.reason : undefined;
}

export interface Recordings { unsaved: UnsavedRecording[]; saved: SavedRecording[]; unreadable?: { whole: boolean; detail: string } | null }
export interface RecordingsView { panel: string; visit: number; open: boolean }

export type Dot = 'red' | 'grey' | 'green' | 'spin';
export type HeadAction = 'stop' | 'record' | 'open';

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
  busy?: boolean;
  problem?: Problem;
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
    case 'off': return `${count} (Auto record was off)`;
    case 'elsewhere': return `${count} (${one ? 'it belongs' : 'they belong'} in another file)`;
    case 'settings': return `${count} (the recorder’s settings couldn’t be read)`;
    case 'notAdmitted': return `${count} (${one ? 'it was' : 'they were'} not identified)`;
    default: return count;
  }
}

export function recordingName(name: string | null): string {
  return name ?? 'New recording';
}

export function firstSentence(text: string): [string, string] {
  const end = /[.!?](?:\s+|$)/u.exec(text);
  if (!end) return [text, ''];
  return [text.slice(0, end.index), text.slice(end.index + end[0].length)];
}

export function campaignTitle(snapshot: Snapshot): string {
  return snapshot.name ?? '';
}

export function head(snapshot: Snapshot): Head {
  const title = campaignTitle(snapshot);
  const named = title !== '';
  const plain = { glyph: false, flag: named ? snapshot.flag : null, title, dot: null, label: null, detail: null, action: null, campaign: false } as const;
  const omitted = omittedText(snapshot.omitted);
  const problem = snapshot.problem;
  if (problem && (snapshot.phase === 'waiting' || (snapshot.phase === 'notRecording' && !named))) {
    const action = snapshot.phase === 'waiting' ? null : 'record';
    if (problem.failed) {
      const [headline, rest] = firstSentence(problem.text);
      return { ...plain, glyph: true, flag: null, title: headline, problem: { ...problem, text: rest }, action };
    }
    return { ...plain, glyph: true, flag: null, title: action ? 'Couldn’t connect to the game' : 'Waiting for the game', problem, action };
  }
  switch (snapshot.phase) {
    case 'waiting':
      return { ...plain, glyph: true, flag: null, title: 'Waiting for the game', detail: snapshot.automatic ? 'Records when you load a campaign.' : 'Auto record is off.' };
    case 'notRecording':
      return named
        ? { ...plain, dot: 'grey', detail: omitted ?? joined(['Not recording', dateText(snapshot.hour)]), action: 'record', campaign: snapshot.campaign != null }
        : { ...plain, glyph: true, title: 'Not recording', detail: omitted, action: 'record' };
    case 'recording':
      return { ...plain, glyph: !named, dot: 'red', label: 'Recording', detail: dateText(snapshot.hour),
        sizes: joined([spoolText(snapshot.spoolBytes), savedText(snapshot.savedBytes)]), action: snapshot.stopping ? null : 'stop', campaign: snapshot.campaign != null };
    case 'saving':
      return { ...plain, glyph: !named, dot: 'spin', label: 'Saving recording…', busy: true };
    case 'saved':
      return { ...plain, glyph: !named, dot: 'green', label: 'Saved', detail: omitted ?? joined([spanText(snapshot.firstHour, snapshot.hour), sizeText(snapshot.savedBytes)]),
        action: snapshot.recording ? 'open' : null };
    case 'discarded':
      return { ...plain, glyph: !named, dot: 'grey', detail: 'Session discarded', action: 'record' };
    case 'unsaved':
      return { ...plain, glyph: !named, dot: 'grey', label: 'Not saved yet', detail: omitted ?? dateText(snapshot.hour), sizes: spoolText(snapshot.spoolBytes) };
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
  return continued ? 'The earlier recording in this file is kept.' : 'This can’t be undone.';
}

export function discardDetail(name: string | null, startedAt: number | null, now: number): string {
  return `Deletes this session’s ${sessionLength(startedAt, now)}${name ? ` of ${name}` : ''}.`;
}

export function waitingDiscardQuestion(continued: boolean): string {
  return continued ? 'Discard these hours?' : 'Discard this recording?';
}

export function promptTitle(quitting: boolean, discarding: boolean): string {
  if (discarding) return quitting ? 'Discard and quit?' : 'Discard this session?';
  return quitting ? 'Quit BattlePlan Recorder?' : 'Stop recording?';
}

export type EscapeStep = 'update' | 'waiting' | 'discard' | 'prompt' | 'up' | 'close' | 'none';

export function escapeStep(screen: { updateAsked: boolean; confirmingWaiting: boolean; prompting: boolean; discarding: boolean; promptBusy: boolean; up: boolean }): EscapeStep {
  if (screen.updateAsked) return 'update';
  if (screen.confirmingWaiting) return 'waiting';
  if (screen.prompting) return screen.promptBusy ? 'none' : screen.discarding ? 'discard' : 'prompt';
  return screen.up ? 'up' : 'close';
}

export function stopQuestion(name: string | null, startedAt: number | null, now: number): string {
  return `Save this session${name ? ` of ${name}` : ''} (${sessionLength(startedAt, now)})?`;
}

export function askQuestion(quitting: boolean, discarding: boolean, length: string): string {
  if (discarding) return `Discard this session (${length})${quitting ? ' and quit' : ''}?`;
  return `Save this session (${length})${quitting ? ' before quitting' : ''}?`;
}

export function panelStage(snapshot: Snapshot | null, compact: boolean | null): 'panel' | 'pending' {
  return snapshot && compact != null ? 'panel' : 'pending';
}

export function stopPromptVisible(asking: boolean, snapshot: Snapshot): boolean {
  return asking && snapshot.phase === 'recording' && !snapshot.stopping;
}

export function settingsQueue<T extends { warning?: string | null }>(read: () => Promise<T>, show: (value: T) => void, warn: (warning: string) => void = () => {},
  unread: (cause: unknown) => void = () => {}) {
  let saving = 0, overlapped = false, started = 0;
  const refresh = () => {
    const asked = started;
    const current = () => asked === started && saving === 0;
    return read().then(value => { if (current()) show(value); }, (cause: unknown) => { if (current()) throw cause; });
  };
  const save = async (work: () => Promise<T | null>): Promise<void> => {
    overlapped ||= saving > 0;
    saving++;
    started++;
    let reply: T | null = null;
    try {
      reply = await work();
      if (reply?.warning) warn(reply.warning);
    } finally {
      saving--;
      const drained = saving === 0, refreshing = drained && (overlapped || reply === null);
      if (drained) overlapped = false;
      if (refreshing) void refresh().catch(unread);
      else if (drained && reply) show(reply);
    }
  };
  return { refresh, save };
}

export function followStatus<T>(listen: (show: (value: T) => void) => Promise<() => void>, read: () => Promise<T>, show: (value: T) => void, failed: (cause: unknown) => void,
  revision: (value: T) => number) {
  let attempt = 0, listening: Promise<() => void> | null = null, latest = -Infinity;
  const newest = (value: T) => {
    if (revision(value) <= latest) return;
    latest = revision(value);
    show(value);
  };
  const stop = () => {
    attempt += 1;
    void listening?.then(unlisten => unlisten(), () => {});
    listening = null;
  };
  const start = () => {
    stop();
    const own = attempt;
    listening = listen(value => { if (own === attempt) newest(value); });
    return listening.then(async () => {
      const value = await read();
      if (own === attempt) newest(value);
    }).catch((cause: unknown) => { if (own === attempt) failed(cause); });
  };
  const first = start();
  return { first, retry: () => { void start(); }, stop };
}

export interface PanelPrompt { id: number; quitting: boolean; discarding: boolean; sessionKey: number }
export interface AskedPrompt { quitting: boolean; discarding: boolean; sessionKey: number }
export interface PanelOpening { compact: boolean }
export interface PanelFailure { title: string; detail: string; retry(): void }
export interface PromptFailure { text: string; detail?: string; retry(): void }
export const PANEL_UNOPENED = 'Couldn’t reach the recorder.';

export function pendingFailure(opened: PanelFailure | null, fit: PromptFailure | null): PanelFailure | null {
  return opened ?? (fit && { title: fit.text, detail: fit.detail || fit.text, retry: fit.retry });
}

export function askedPrompt(snapshot: Snapshot | null, opened: number | null): AskedPrompt | null {
  if (!snapshot || snapshot.phase !== 'recording' || snapshot.stopping || snapshot.sessionKey == null) return null;
  if (snapshot.prompt) return snapshot.prompt;
  return opened === snapshot.sessionKey ? { quitting: false, discarding: false, sessionKey: snapshot.sessionKey } : null;
}

export function promptDiscards(asked: AskedPrompt | null, stepped: number | null): boolean {
  return asked !== null && (asked.discarding || stepped === asked.sessionKey);
}

export function stopConfirmed(asked: AskedPrompt | null, stepped: number | null, discard: boolean, sessionKey: number): boolean {
  return asked !== null && asked.sessionKey === sessionKey && promptDiscards(asked, stepped) === discard;
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
