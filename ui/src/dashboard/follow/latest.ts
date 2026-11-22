import { decode } from '@msgpack/msgpack';
import type { FollowUpdate, TimedReading, WirePanel } from '../../draw/followed';
import type { FollowedPanel } from './message';
import type { PageReading } from './store';

export type Shown = Omit<TimedReading, 'panel'> & { panel: FollowedPanel | null };
interface Line { epoch: number; readings: Shown[]; shown?: Shown; held: boolean; through?: number; coverage?: Shown['coverage'] }
export type Show = (reading: PageReading, quiet: boolean) => void;

export function pagePanel(wire: WirePanel | null): FollowedPanel | null {
  if (!wire?.bytes) return wire as FollowedPanel | null;
  const bytes = wire.bytes;
  let data: unknown, read = false;
  return Object.defineProperty({ refused: wire.refused }, 'data', { enumerable: true, get: () => {
    if (!read) { data = decode(bytes); read = true; }
    return data;
  } });
}

export const coversHour = (reading: Pick<Shown, 'panel' | 'answer' | 'coverage'>, hour: number) => !reading.panel && !reading.answer
  || !!reading.coverage && reading.coverage.start <= hour && hour <= reading.coverage.through;

const from = (reading: Shown) => Math.max(reading.due, reading.coverage?.start ?? -Infinity);

export class LatestReadings {
  private readonly lines = new Map<number, Line>();
  private readonly epoch: (id: number) => number | undefined;
  private due = new Set<number>();
  private next = Infinity;

  constructor(epoch: (id: number) => number | undefined) { this.epoch = epoch; }

  get waiting() { return this.due.size > 0; }

  land(update: FollowUpdate, hour: number, paused: boolean, show: Show) {
    if (this.epoch(update.id) !== update.epoch) return;
    let line = this.lines.get(update.id);
    if (line?.epoch !== update.epoch) this.lines.set(update.id, line = { epoch: update.epoch, readings: [], held: false });
    const drop = new Set(update.drop);
    const readings = line.readings.filter(r => r.due >= update.keep && !drop.has(r.due));
    for (const [due, through] of update.cover ?? []) {
      const held = readings.find(r => r.due === due);
      if (held?.coverage) held.coverage = { start: held.coverage.start, through };
    }
    for (const put of update.put ?? []) {
      const reading: Shown = { ...put, panel: pagePanel(put.panel) };
      let at = readings.length;
      while (at > 0 && readings[at - 1]!.due >= reading.due) at--;
      readings.splice(at, readings[at]?.due === reading.due ? 1 : 0, reading);
    }
    line.readings = readings;
    this.place(update.id, line, hour, paused, show);
  }

  advance(hour: number, paused: boolean, show: Show) {
    if (hour < this.next) return;
    const due = this.due;
    this.due = new Set();
    this.next = Infinity;
    for (const id of due) {
      const line = this.lines.get(id);
      if (line) this.place(id, line, hour, paused, show);
    }
  }

  refresh(hour: number, paused: boolean, show: Show) {
    this.due = new Set();
    this.next = Infinity;
    for (const [id, line] of this.lines) this.place(id, line, hour, paused, show);
  }

  forget(ids: Iterable<number>) {
    for (const id of ids) { this.lines.delete(id); this.due.delete(id); }
  }

  private place(id: number, line: Line, hour: number, paused: boolean, show: Show) {
    if (this.epoch(id) !== line.epoch) return;
    let candidate: Shown | undefined, next = Infinity;
    for (const reading of line.readings) {
      const at = from(reading);
      if (at <= hour) candidate = reading;
      else if (at < next) next = at;
    }
    const coverage = candidate?.coverage, through = coverage ? Math.min(coverage.through, hour) : candidate?.hour ?? hour;
    if (coverage && through < coverage.through && through + 1 < next) next = through + 1;
    if (next < Infinity) { this.due.add(id); if (next < this.next) this.next = next; } else this.due.delete(id);
    if (!candidate) return;
    if (paused && !coversHour(candidate, hour)) {
      if (!line.held) show({ id, hour, through: hour, panel: null, answer: null, values: null }, false);
      line.held = true; line.shown = candidate; line.through = line.coverage = undefined;
      return;
    }
    const changed = line.held || line.shown !== candidate;
    if (!changed && line.through === through && line.coverage === coverage) return;
    line.held = false; line.shown = candidate; line.through = through; line.coverage = coverage;
    show({ id, hour: candidate.hour, through, coverage, panel: candidate.panel, answer: candidate.answer, values: candidate.values }, !changed);
  }
}
