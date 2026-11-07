import type { CountryFlagChoice, CountryFlagSize } from '../generated/dashboard';
import type { DashboardClient, DashboardSource } from '../dashboard/client';
import { isFailedFact } from '../dashboard/countryFacts.ts';
import { askableAt, askTimeline, flagIn, stylesIn, watchTimeline, type CountryTimeline } from '../dashboard/countryTimeline.ts';
import { drawFlags, drawnFlag, watchFlagDrawings, type FlagRead } from '../dashboard/countryFlags.ts';

/** A country as a live row shows it. `name` undefined: being read; null: the recording has none
 * (`nameReason` says why). `flag` is an image URL; `flagPending` while it is read or drawn. */
export interface LiveLabel { name: string | null | undefined; nameReason?: string; flag?: string; flagPending: boolean; flagReason?: string;
  /** The recorded map colour (null: none recorded; undefined: being read). */
  color: readonly [number, number, number] | null | undefined }

/** Names and flags as the recording showed them, read from the branch's country timeline and the
 * flag drawings without React, for painters. A country whose name or flag at a new hour is still
 * read keeps the last one shown for it, so a known label never flips to "loading" and back. */
export class LiveLabels {
  private timeline: CountryTimeline | undefined;
  private readonly lastName = new Map<string, string>();
  private readonly lastColor = new Map<string, readonly [number, number, number]>();
  private readonly lastFlag = new Map<string, FlagRead>();
  private readonly undrawn = new Set<CountryFlagChoice>();
  private stops: (() => void)[] = [];
  // A wake for when a timeline read that failed may be asked again: paused, nothing else asks.
  private retryTimer: ReturnType<typeof setTimeout> | null = null;
  private readonly changed: () => void;
  private readonly client: DashboardClient;
  private readonly recording: DashboardSource;
  private readonly source: { sourceId: string; revision: string; branchId: string; content: number };
  private readonly size: CountryFlagSize;

  constructor(client: DashboardClient, recording: DashboardSource, branchId: string, content: number, size: CountryFlagSize, changed: () => void) {
    this.client = client; this.recording = recording; this.size = size;
    this.source = { sourceId: recording.id, revision: recording.revision, branchId, content };
    this.changed = changed;
  }

  /** Hears the timeline and flag drawings (from an effect); `stop` lets go, and it may start again. */
  start(): void {
    if (this.stops.length) return;
    this.stops = [watchTimeline(this.client, this.source, this.changed), watchFlagDrawings(this.client, this.changed)];
  }

  stop(): void {
    for (const stop of this.stops) stop();
    this.stops = [];
    if (this.retryTimer !== null) clearTimeout(this.retryTimer);
    this.retryTimer = null;
  }

  /** Asks for the timeline around `hour`; call once per frame that reads labels, before `label`. */
  ask(hour: number): void {
    this.timeline = askTimeline(this.client, this.source, this.recording, hour);
    const resolved = this.timeline && stylesIn(this.timeline, hour, []) !== undefined;
    const due = resolved || this.retryTimer !== null || !this.stops.length ? undefined : askableAt(this.client, this.source);
    if (due !== undefined && due > Date.now()) {
      this.retryTimer = setTimeout(() => { this.retryTimer = null; this.changed(); }, due - Date.now() + 16);
    }
  }

  /** Draws the flags `label` found undrawn since the last call. */
  draw(): void {
    if (!this.undrawn.size) return;
    drawFlags(this.client, this.recording, this.source.content, this.size, this.undrawn);
    this.undrawn.clear();
  }

  label(hour: number, tag: string): LiveLabel {
    const style = this.timeline && stylesIn(this.timeline, hour, [tag])?.get(tag);
    let name: string | null | undefined = style ? style.name ?? null : undefined;
    const nameReason = style && style.name == null ? style.warnings.at(-1) : undefined;
    if (typeof name === 'string') this.lastName.set(tag, name);
    else if (name === undefined) name = this.lastName.get(tag);
    let color: readonly [number, number, number] | null | undefined = style ? style.color : undefined;
    if (color) this.lastColor.set(tag, color);
    else if (color === undefined) color = this.lastColor.get(tag);
    const found = this.timeline && flagIn(this.timeline, hour, tag);
    let read: FlagRead | null | undefined;
    if (found && 'reason' in found) read = { reason: found.reason };
    else if (found) {
      const drawn = drawnFlag(this.client, this.recording, this.source.content, this.size, found.choice);
      if (drawn === undefined) this.undrawn.add(found.choice);
      else read = isFailedFact(drawn) ? { reason: drawn.message } : drawn;
    }
    if (read?.url) this.lastFlag.set(tag, read);
    else if (read === undefined) read = this.lastFlag.get(tag);
    return { name, nameReason, flag: read?.url, flagPending: read === undefined, flagReason: read?.reason, color };
  }
}
