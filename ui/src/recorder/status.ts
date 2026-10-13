// What the recorder panel shows for each recorder state. Pure, so every state
// of the approved design is checked without a running recorder.
import { formatDate, gameDate, monthNames } from '../dashboard/calendar.ts';
import { recordingDisplayName } from '../dashboard/recordingNames.ts';
import type { RecordingNames } from '../dashboard/recordingNames.ts';
import { formatMegabytes } from '../charts/numberFormat.ts';

export type Phase = 'waiting' | 'notRecording' | 'recording' | 'saving' | 'saved' | 'discarded';
export interface Problem { text: string; detail: string }
/** Loads a finished session recorded but left out of its recordings, and why when it is one reason. */
export interface OmittedLoads { loads: number; reason: 'off' | 'elsewhere' | 'settings' | 'notAdmitted' | null }
/** Straight-alpha RGBA of the viewed country's flag, as the app's flag canvas draws it. */
export interface FlagImage { width: number; height: number; pixels: number[] }

/** The recorder's snapshot, as the tray app publishes it. */
export interface Snapshot {
  phase: Phase;
  campaign: string | null;
  recording: string | null;
  continued: boolean;
  hour: number | null;
  firstHour: number | null;
  bytes: number | null;
  progress: number | null;
  startedAt: number | null;
  stopping: boolean;
  problem: Problem | null;
  automatic: boolean;
  session: string | null;
  /** The viewed country's flag from the recording's content; null when none resolves. */
  flag: FlagImage | null;
  /** Loads the finished session left out of its recordings; absent or null when none. */
  omitted?: OmittedLoads | null;
}

export type NextLoad = 'continue' | 'newFile';
export interface CampaignChoice { record?: boolean; next?: NextLoad; folder?: string; flag?: string }
export interface RecorderSettings {
  recordEveryCampaign: boolean;
  saveFolder?: string;
  userFolder?: string;
  startWithWindows: boolean;
  startWithReplay: boolean;
  gameScore: boolean;
  operationOutcomes: boolean;
  politics: boolean;
  campaigns: Record<string, CampaignChoice>;
  recordingNames: RecordingNames;
  /** The folder recordings are saved to, the default when none is chosen. */
  effectiveSaveFolder: string;
}

export type Dot = 'red' | 'grey' | 'green' | 'spin';
export type HeadAction = 'stop' | 'record' | 'open';

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
  action: HeadAction | null;
  campaign: boolean;
  footer: boolean;
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
    case 'notAdmitted': return `${count} (${one ? 'it was' : 'they were'} not identified)`;
    default: return count;
  }
}

export function campaignTitle(snapshot: Snapshot, names: RecordingNames): string {
  if (snapshot.recording) return recordingDisplayName(snapshot.recording, names);
  return snapshot.campaign ?? '';
}

export function head(snapshot: Snapshot, names: RecordingNames): Head {
  const title = campaignTitle(snapshot, names);
  const named = title !== '';
  const plain = { glyph: false, flag: named ? snapshot.flag : null, title, dot: null, label: null, detail: null, action: null, campaign: false, footer: true } as const;
  // Loads a finished session left out replace its usual status text until shown.
  const omitted = omittedText(snapshot.omitted);
  switch (snapshot.phase) {
    case 'waiting':
      return { ...plain, glyph: true, flag: null, title: 'Waiting for the game', detail: snapshot.automatic ? 'Records when you load a campaign' : null };
    case 'notRecording':
      return named
        ? { ...plain, dot: 'grey', detail: omitted ?? joined(['Not recording', dateText(snapshot.hour)]), action: 'record', campaign: snapshot.campaign != null }
        : { ...plain, glyph: true, title: 'Not recording', detail: omitted, action: 'record' };
    case 'recording':
      return { ...plain, glyph: !named, dot: 'red', label: 'Recording', detail: joined([dateText(snapshot.hour), sizeText(snapshot.bytes)]),
        action: snapshot.stopping ? null : 'stop', campaign: snapshot.campaign != null };
    case 'saving':
      return { ...plain, glyph: !named, dot: 'spin', label: 'Saving recording…', footer: false, bar: snapshot.progress };
    case 'saved':
      return { ...plain, glyph: !named, dot: 'green', label: 'Saved', detail: omitted ?? joined([spanText(snapshot.firstHour, snapshot.hour), sizeText(snapshot.bytes)]),
        action: snapshot.recording ? 'open' : null };
    case 'discarded':
      return { ...plain, glyph: !named, dot: 'grey', detail: 'Session discarded', action: 'record', footer: false };
  }
}

/** How long this session has recorded, as the Stop prompt asks about it. */
export function sessionLength(startedAt: number | null, now: number): string {
  const minutes = startedAt == null ? 0 : Math.max(0, Math.floor((now - startedAt) / 60_000));
  const hours = Math.floor(minutes / 60), rest = minutes % 60;
  if (!hours) return `${Math.max(1, rest)} min`;
  return rest ? `${hours} h ${rest} min` : `${hours} h`;
}

/** What Discard deletes: only this session when the file held an earlier recording. */
export function discardTooltip(continued: boolean): string {
  return continued ? 'Deletes this session only. Earlier recording in this file is kept.' : 'Deletes this recording.';
}

/** The Stop prompt shows only while recording, until Save or Discard is carried out. */
export function stopPromptVisible(asking: boolean, snapshot: Snapshot): boolean {
  return asking && snapshot.phase === 'recording' && !snapshot.stopping;
}

export function recordsAutomatically(settings: RecorderSettings, campaign: string): boolean {
  return settings.campaigns[campaign]?.record ?? settings.recordEveryCampaign;
}

export function nextLoad(settings: RecorderSettings, campaign: string): NextLoad {
  return settings.campaigns[campaign]?.next ?? 'continue';
}

export function whatToRecord(settings: RecorderSettings): string {
  return settings.gameScore && settings.operationOutcomes && settings.politics ? 'Everything' : 'Custom';
}
