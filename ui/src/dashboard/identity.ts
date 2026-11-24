import type { IdentityChange, MembershipRun } from '../generated/dashboard';

export class Lineage {
  private readonly held = new Map<string, { hour: number; country: string }[]>();
  private readonly tags = new Map<string, { hour: number; tag: string }[]>();

  constructor(changes: readonly IdentityChange[]) {
    const byHour = new Map<number, IdentityChange[]>();
    for (const change of changes) {
      const list = byHour.get(change.hour);
      if (list) list.push(change); else byHour.set(change.hour, [change]);
    }
    for (const hour of [...byHour.keys()].sort((a, b) => a - b)) {
      const moves = byHour.get(hour)!;
      const moving = moves.map(change => ({ change, country: this.countryAt(change.from, hour - 1) }));
      const renamedTo = new Set(moves.map(change => change.to));
      for (const { change } of moving) {
        if (!renamedTo.has(change.from)) this.segments(change.from).push({ hour, country: JSON.stringify([change.from, hour]) });
      }
      for (const { change, country } of moving) {
        this.segments(change.to).push({ hour, country });
        const history = this.tags.get(country) ?? [{ hour: Number.NEGATIVE_INFINITY, tag: change.from }];
        history.push({ hour, tag: change.to });
        this.tags.set(country, history);
      }
    }
  }

  private segments(tag: string): { hour: number; country: string }[] {
    let list = this.held.get(tag);
    if (!list) { list = [{ hour: Number.NEGATIVE_INFINITY, country: JSON.stringify([tag, null]) }]; this.held.set(tag, list); }
    return list;
  }

  countryAt(tag: string, hour: number): string {
    const list = this.held.get(tag);
    return list ? entryAt(list, hour).country : this.unchanged(tag);
  }

  holderStart(tag: string, hour: number): number {
    const list = this.held.get(tag);
    return list ? entryAt(list, hour).hour : Number.NEGATIVE_INFINITY;
  }

  changeHours(tag: string, firstHour: number, lastHour: number): number[] {
    const list = this.held.get(tag);
    if (!list) return [];
    let low = 0;
    let high = list.length;
    while (low < high) {
      const middle = (low + high) >> 1;
      if (list[middle].hour < firstHour) low = middle + 1; else high = middle;
    }
    const hours: number[] = [];
    for (let index = low; index < list.length && list[index].hour <= lastHour; index++) {
      if (index > 0 && list[index].country !== list[index - 1].country) hours.push(list[index].hour);
    }
    return hours;
  }

  private readonly ids = new Map<string, string>();
  private unchanged(tag: string): string {
    let id = this.ids.get(tag);
    if (id === undefined) { id = JSON.stringify([tag, null]); this.ids.set(tag, id); }
    return id;
  }

  tagOf(country: string, hour: number): string {
    const history = this.tags.get(country);
    if (history) return entryAt(history, hour).tag;
    const [tag] = JSON.parse(country) as [string, number | null];
    return tag;
  }

  aliasRuns(countries: ReadonlySet<string>, firstHour: number, lastHour: number): MembershipRun[] {
    const runs: MembershipRun[] = [];
    for (const country of countries) {
      const history = this.tags.get(country) ?? [{ hour: Number.NEGATIVE_INFINITY, tag: this.tagOf(country, firstHour) }];
      for (let index = 0; index < history.length; index++) {
        const { tag } = history[index];
        const first = Math.max(firstHour, history[index].hour);
        const last = Math.min(lastHour, (history[index + 1]?.hour ?? Infinity) - 1);
        if (first > last) continue;
        const starts = [first, ...this.changeHours(tag, first + 1, last)];
        for (let piece = 0; piece < starts.length; piece++) {
          const start = starts[piece];
          if (this.countryAt(tag, start) === country) runs.push({ tag, firstHour: start, lastHour: (starts[piece + 1] ?? last + 1) - 1 });
        }
      }
    }
    return runs.sort((a, b) => a.firstHour - b.firstHour || a.tag.localeCompare(b.tag));
  }
}

function entryAt<E extends { hour: number }>(list: readonly E[], hour: number): E {
  let low = 0;
  let high = list.length - 1;
  while (low < high) {
    const middle = (low + high + 1) >> 1;
    if (list[middle].hour <= hour) low = middle; else high = middle - 1;
  }
  return list[low];
}

export function joinTagChanges<R extends { tag: string; firstHour: number; lastHour: number }>(runs: readonly R[], lineage: Lineage,
  relatedTags: (run: R) => readonly string[] = () => []): R[][] {
  const groups = new Map<string, R[]>();
  for (const run of runs) {
    const changes = [run.tag, ...relatedTags(run)].flatMap(tag => lineage.changeHours(tag, run.firstHour + 1, run.lastHour));
    const starts = [run.firstHour, ...new Set(changes.sort((a, b) => a - b))];
    for (let index = 0; index < starts.length; index++) {
      const firstHour = starts[index];
      const lastHour = (starts[index + 1] ?? run.lastHour + 1) - 1;
      const piece = starts.length === 1 ? run : { ...run, firstHour, lastHour };
      const country = lineage.countryAt(run.tag, firstHour);
      const group = groups.get(country);
      if (group) group.push(piece); else groups.set(country, [piece]);
    }
  }
  return [...groups.values()].map(group => group.sort((a, b) => a.firstHour - b.firstHour))
    .sort((a, b) => a[0].firstHour - b[0].firstHour);
}

