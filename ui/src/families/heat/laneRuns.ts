import type { HeatLaneRun } from './lanesModel.ts';

export type LaneRuns = readonly HeatLaneRun[];

export const noRuns: LaneRuns = [];

export interface LanePieces {
  readonly pieces: readonly LaneRuns[];
  readonly runs: LaneRuns;
}

export function joinLanePieces(pieces: readonly LaneRuns[], previous: LanePieces | undefined): LanePieces {
  let part = 0;
  let retained = 0;
  while (previous && part < pieces.length && pieces[part] === previous.pieces[part]) {
    retained += pieces[part++]!.length;
  }
  if (previous && part === pieces.length && part === previous.pieces.length) return previous;
  if (pieces.length === 1) return { pieces, runs: pieces[0]! };
  const runs = retained ? previous!.runs.slice(0, retained) : [];
  for (; part < pieces.length; part++) {
    for (const run of pieces[part]!) runs.push(run);
  }
  return { pieces, runs };
}

export function laneRun(firstHour: number, lastHour: number, readHour: number | null, cell: HeatLaneRun['cell'], previous?: HeatLaneRun): HeatLaneRun {
  return previous && previous.firstHour === firstHour && previous.lastHour === lastHour && previous.readHour === readHour && previous.cell === cell
    ? previous : { firstHour, lastHour, readHour, cell };
}

export function solidCell(color: string, previous?: HeatLaneRun['cell']): HeatLaneRun['cell'] {
  return previous?.kind === 'solid' && previous.color === color && previous.label === undefined ? previous : { kind: 'solid', color };
}

export function missingCell(reason: string, previous?: HeatLaneRun['cell']): HeatLaneRun['cell'] {
  return previous?.kind === 'missing' && previous.reason === reason ? previous : { kind: 'missing', reason };
}

export class LaneRunBuilder {
  private readonly previous: LaneRuns;
  private changed?: HeatLaneRun[];
  private count = 0;

  constructor(previous: LaneRuns) {
    this.previous = previous;
  }

  get next(): HeatLaneRun | undefined {
    return this.previous[this.count];
  }

  add(run: HeatLaneRun): void {
    if (!this.changed && run !== this.next) this.changed = this.previous.slice(0, this.count);
    this.changed?.push(run);
    this.count++;
  }

  finish(): LaneRuns {
    return this.changed ?? (this.count === this.previous.length ? this.previous : this.previous.slice(0, this.count));
  }
}

export function extendLastRun(runs: LaneRuns, lastHour: number, readHour: number | null): LaneRuns {
  const last = runs.at(-1);
  if (!last || last.lastHour === lastHour && last.readHour === readHour) return runs;
  const extended = runs.slice();
  extended[extended.length - 1] = { ...last, lastHour, readHour };
  return extended;
}
