import { decode } from '@msgpack/msgpack';
import type { FollowUpdate, TimedReading, WirePanel } from '../../draw/followed';
import type { FollowedPanel } from './message';
import type { PageReading, Placed } from './store';

export type Shown = Omit<TimedReading, 'panel' | 'kept'> & { panel: FollowedPanel | null };
export type Show = (reading: PageReading, placed: Placed) => void;
interface Line {
  epoch: number; readings: Shown[]; carried?: Shown; model?: { structure: string; panel: FollowedPanel | null };
  shown?: Shown; placed: Placed; through?: number; coverage?: Shown['coverage'];
}

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

export const readingFrom = (reading: Pick<Shown, 'due' | 'coverage'>) => Math.max(reading.due, reading.coverage?.start ?? -Infinity);

export class LatestReadings {
  private readonly lines = new Map<number, Line>();
  private readonly epoch: (id: number) => number | undefined;
  private due = new Set<number>();
  private next = Infinity;

  constructor(epoch: (id: number) => number | undefined) { this.epoch = epoch; }

  get waiting() { return this.due.size > 0; }

  land(update: FollowUpdate, hour: number, paused: boolean, show: Show) {
    const epoch = this.epoch(update.id);
    if (epoch !== update.epoch) return;
    let line = this.lines.get(update.id);
    if (!line) this.lines.set(update.id, line = { epoch, readings: [], placed: 'empty' });
    else if (line.epoch !== epoch) this.renewLine(line, epoch, false);
    const drop = new Set(update.drop);
    const readings = drop.size ? line.readings.filter(r => !drop.has(r.due)) : line.readings;
    for (const [due, through] of update.cover ?? []) {
      const held = readings.find(r => r.due === due);
      if (held?.coverage) held.coverage = { start: held.coverage.start, through };
    }
    for (const put of update.put ?? []) {
      let panel: FollowedPanel | null, structure = put.structure;
      if (put.kept) {
        if (!line.model) throw new Error('A followed reading keeps a panel the page does not hold');
        ({ panel, structure } = line.model);
      } else {
        panel = pagePanel(put.panel);
        if (structure !== undefined) line.model = { structure, panel };
      }
      const reading: Shown = { due: put.due, hour: put.hour, coverage: put.coverage, panel, answer: put.answer, values: put.values,
        ...(structure !== undefined && { structure }) };
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

  renew(ids: Iterable<number>, carry: boolean) {
    for (const id of ids) {
      const line = this.lines.get(id), epoch = this.epoch(id);
      if (line && epoch !== undefined) this.renewLine(line, epoch, carry);
    }
  }

  forget(ids: Iterable<number>) {
    for (const id of ids) { this.lines.delete(id); this.due.delete(id); }
  }

  private renewLine(line: Line, epoch: number, carry: boolean) {
    line.carried = carry ? line.shown ?? line.carried : undefined;
    line.epoch = epoch; line.readings = []; line.model = undefined;
  }

  private place(id: number, line: Line, hour: number, paused: boolean, show: Show) {
    if (this.epoch(id) !== line.epoch) return;
    let at = -1, next = Infinity;
    line.readings.forEach((reading, index) => {
      const from = readingFrom(reading);
      if (from <= hour) at = index;
      else if (from < next) next = from;
    });
    if (at > 0) line.readings = line.readings.slice(at);
    if (at >= 0) line.carried = undefined;
    const candidate = at >= 0 ? line.readings[0] : line.carried;
    const coverage = candidate?.coverage, through = coverage ? Math.min(coverage.through, hour) : candidate?.hour ?? hour;
    if (coverage && through < coverage.through && through + 1 < next) next = through + 1;
    if (next < Infinity) { this.due.add(id); if (next < this.next) this.next = next; } else this.due.delete(id);
    if (!candidate) {
      if (line.placed === 'empty') return;
      line.placed = 'empty'; line.shown = line.through = line.coverage = undefined;
      show({ id, hour, through: hour, panel: null, answer: null, values: null }, 'empty');
      return;
    }
    const reading = { id, hour: candidate.hour, through, coverage, panel: candidate.panel, answer: candidate.answer, values: candidate.values,
      ...(candidate.structure !== undefined && { structure: candidate.structure }) };
    const placed: Placed = paused && !coversHour(candidate, hour) ? 'held' : 'current';
    const changed = line.placed !== placed || line.shown !== candidate;
    if (!changed && line.through === through && line.coverage === coverage) return;
    line.placed = placed; line.shown = candidate; line.through = through; line.coverage = coverage;
    show(reading, changed ? placed : 'dated');
  }
}
