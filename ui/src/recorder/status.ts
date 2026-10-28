// What the recorder panel shows for each recorder state. Pure, so every state
// of the approved design is checked without a running recorder.
import { formatDate, gameDate, monthNames } from '../dashboard/calendar.ts';
import { formatMegabytes } from '../charts/numberFormat.ts';
import type { Compression } from './compression';

export type Phase = 'waiting' | 'notRecording' | 'recording' | 'saving' | 'saved' | 'discarded' | 'unsaved';
export interface Problem { text: string; detail: string }
/** Loads a finished session recorded but left out of its recordings, and why when it is one reason. */
export interface OmittedLoads { loads: number; reason: 'off' | 'elsewhere' | 'settings' | 'waiting' | 'notAdmitted' | null }
/** Straight-alpha RGBA of the viewed country's flag, as the app's flag canvas draws it. */
export interface FlagImage { width: number; height: number; pixels: number[] }

/** The recorder's snapshot, as the tray app publishes it. */
export interface Snapshot {
  phase: Phase;
  campaign: string | null;
  recording: string | null;
  /** The recording's file name; before its first save, the name its file will get. */
  name: string | null;
  continued: boolean;
  hour: number | null;
  firstHour: number | null;
  /** The saved recording's size: the earlier part a recording continues, or the file once saved. */
  savedBytes: number | null;
  /** This session's spool on disk, not saved yet. */
  spoolBytes: number | null;
  progress: number | null;
  startedAt: number | null;
  stopping: boolean;
  problem: Problem | null;
  automatic: boolean;
  session: string | null;
  /** The mode of the session attached to the game, even at its menu; null while none is. */
  captureMode: CaptureMode | null;
  /** The viewed country's flag from the recording's content; null when none resolves. */
  flag: FlagImage | null;
  /** Loads the finished session left out of its recordings; absent or null when none. */
  omitted?: OmittedLoads | null;
}

export type NextLoad = 'continue' | 'newFile';
/** What new recordings hold: everything, or the campaign map with its battle windows. */
export type CaptureMode = 'full' | 'map';
export interface CampaignChoice { record?: boolean; next?: NextLoad; folder?: string; flag?: string }
export interface RecorderSettings {
  compression?: Compression;
  /** Auto record. */
  recordEveryCampaign: boolean;
  autoSave: boolean;
  saveFolder?: string;
  userFolder?: string;
  startWithWindows: boolean;
  startWithReplay: boolean;
  captureMode: CaptureMode;
  campaigns: Record<string, CampaignChoice>;
  /** The folder recordings are saved to, the default when none is chosen. */
  effectiveSaveFolder: string;
}

/** A recording not saved yet: recording now, or waiting in a session folder for Save or Discard. */
export interface UnsavedRecording {
  session: string;
  campaign: string;
  /** The saved recording these hours continue, when there is one. */
  recording: string | null;
  /** Its file name, or the name its file will get; null until known. */
  name: string | null;
  firstHour: number | null;
  lastHour: number | null;
  /** Spool bytes on disk; null when unknown. */
  spoolBytes: number | null;
  status: 'recording' | 'waiting' | 'saving';
  flag: FlagImage | null;
}

export interface SavedRecording {
  path: string;
  firstHour: number | null;
  lastHour: number | null;
  /** The file's size from its metadata. */
  bytes: number;
  flag: FlagImage | null;
}

export interface Recordings { unsaved: UnsavedRecording[]; saved: SavedRecording[] }
export interface RecordingsView { panel: string; visit: number; open: boolean }

export type Dot = 'red' | 'grey' | 'green' | 'spin';
export type HeadAction = 'stop' | 'record' | 'open' | 'recordings';

/** The head row and which parts of the panel follow it. */
export interface Head {
  /** The game glyph instead of a campaign, while no campaign is named. */
  glyph: boolean;
  /** The campaign's flag, when one resolved. */
  flag: FlagImage | null;
  title: string;
  dot: Dot | null;
  /** The emphasised state word. */
  label: string | null;
  /** Plain status text after the label; truncates, never runs under the button. */
  detail: string | null;
  /** Sizes on disk, on their own line: spool and saved apart, never added up. */
  sizes?: string | null;
  action: HeadAction | null;
  campaign: boolean;
  /** Progress bar fill, 0 to 1; null shows work without a known end. */
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

/** This session's spool on disk, never mistaken for a saved file's size. */
export function spoolText(bytes: number | null): string | null {
  return bytes == null ? null : `${formatMegabytes(bytes)} this session`;
}

/** The saved part of a recording still being recorded. */
export function savedText(bytes: number | null): string | null {
  return bytes == null ? null : `${formatMegabytes(bytes)} saved`;
}

/** Spool on disk across everything not saved yet; "at least" when a size is unknown. */
export function spoolTotalText(unsaved: readonly UnsavedRecording[]): string | null {
  const known = unsaved.filter(recording => recording.spoolBytes != null);
  if (!known.length) return null;
  const total = formatMegabytes(known.reduce((sum, recording) => sum + recording.spoolBytes!, 0));
  return `${known.length < unsaved.length ? 'at least ' : ''}${total} on disk`;
}

/** A saved recording's span: its first year to its last month. */
export function spanText(first: number | null, last: number | null): string | null {
  const start = first == null ? null : gameDate(first);
  const end = last == null ? null : gameDate(last);
  if (!start || !end) return dateText(last);
  return `${start.year} – ${monthNames[end.month - 1].slice(0, 3)} ${end.year}`;
}

/** One plain line for loads a session left out, such as "1 load not recorded (Record automatically was off)". */
export function omittedText(omitted: OmittedLoads | null | undefined): string | null {
  if (!omitted || omitted.loads < 1) return null;
  const one = omitted.loads === 1;
  const count = `${omitted.loads} ${one ? 'load' : 'loads'} not recorded`;
  switch (omitted.reason) {
    case 'off': return `${count} (Record automatically was off)`;
    case 'elsewhere': return `${count} (${one ? 'it belongs' : 'they belong'} in another file)`;
    case 'settings': return `${count} (the recorder's settings couldn't be read)`;
    case 'waiting': return `${count} (earlier hours wait to be saved; save or discard them in Recordings)`;
    case 'notAdmitted': return `${count} (${one ? 'it was' : 'they were'} not identified)`;
    default: return count;
  }
}

/** The one name a recording has anywhere in the recorder: its file name, or the one its file will get. */
export function recordingName(name: string | null): string {
  return name ?? 'New recording';
}

/** The recording's name; none until it has one, never the campaign's id. */
export function campaignTitle(snapshot: Snapshot): string {
  return snapshot.name ?? '';
}

export function head(snapshot: Snapshot): Head {
  const title = campaignTitle(snapshot);
  const named = title !== '';
  const plain = { glyph: false, flag: named ? snapshot.flag : null, title, dot: null, label: null, detail: null, action: null, campaign: false } as const;
  // Loads a finished session left out replace its usual status text until shown.
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

/** How long this session has recorded, as the Stop prompt asks about it. */
export function sessionLength(startedAt: number | null, now: number): string {
  const minutes = startedAt == null ? 0 : Math.max(0, Math.floor((now - startedAt) / 60_000));
  const hours = Math.floor(minutes / 60), rest = minutes % 60;
  if (!hours) return `${Math.max(1, rest)} min`;
  return rest ? `${hours} h ${rest} min` : `${hours} h`;
}

/** Milliseconds until the Stop prompt's session length next changes. */
export function untilNextMinute(startedAt: number | null, now: number): number {
  const elapsed = startedAt == null ? 0 : Math.max(0, now - startedAt);
  return 60_000 - elapsed % 60_000;
}

/** What Discard deletes: only this session when the file held an earlier recording. */
export function discardConsequence(continued: boolean): string {
  return continued ? 'The earlier recording in this file is kept.' : "This can't be undone.";
}

/** What Discard's confirmation beside the tray icon deletes, naming the recording once it has a name. */
export function discardDetail(name: string | null, startedAt: number | null, now: number, continued: boolean): string {
  return `Deletes this session's ${sessionLength(startedAt, now)}${name ? ` of ${name}` : ''}. ${discardConsequence(continued)}`;
}

/** Discard's confirmation on a recording waiting to be saved. */
export function waitingDiscardQuestion(continued: boolean): string {
  return `${continued ? 'Discard these hours?' : 'Discard this recording?'} ${discardConsequence(continued)}`;
}

/** The prompt beside the tray icon: Discard's confirmation, or the Save or Discard question; Quit's says it quits. */
export function promptTitle(quitting: boolean, discarding: boolean): string {
  if (discarding) return quitting ? 'Discard and quit?' : 'Discard this session?';
  return quitting ? 'Quit Battle Plan Recorder?' : 'Stop recording?';
}

/** The compact Stop prompt's question, naming the recording once it has a name. */
export function stopQuestion(name: string | null, startedAt: number | null, now: number): string {
  return `Keep this session's ${sessionLength(startedAt, now)}${name ? ` of ${name}` : ''}?`;
}

/** The Stop prompt shows only while recording, until Save or Discard is carried out. */
export function stopPromptVisible(asking: boolean, snapshot: Snapshot): boolean {
  return asking && snapshot.phase === 'recording' && !snapshot.stopping;
}

/** A save can be stored before it fails, so a read follows a failed save. */
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

/** Shows each status from the current one on: read once listening, so none is lost, and dropped if a newer one came during the read. */
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

/** An attached session, even at the game's menu, keeps the mode it attached with: its recording file holds one mode. */
export function captureModeFixed(snapshot: Snapshot): boolean {
  return snapshot.captureMode !== null;
}
