import type { IdentityChange } from '../generated/dashboard';

/** Which country a tag named at an hour, from the recorded changes of tag
 * (`country-groups`' `identityChanges`: an id naming another tag from one recorded
 * hour to the next) alone, so every widget keeping things by tag resolves them
 * through one timeline: a tag switch is never read as one country leaving and another
 * joining, and a tag a country gave up and another later took names two countries. */
export class Lineage {
  /** Per tag, from which hour on which country held it, in hour order. */
  private readonly held = new Map<string, { hour: number; country: string }[]>();
  /** Per country that was renamed, from which hour on which tag it had, in hour order
   * (a country never renamed has the tag its id names throughout). */
  private readonly tags = new Map<string, { hour: number; tag: string }[]>();

  constructor(changes: readonly IdentityChange[]) {
    const byHour = new Map<number, IdentityChange[]>();
    for (const change of changes) {
      const list = byHour.get(change.hour);
      if (list) list.push(change); else byHour.set(change.hour, [change]);
    }
    for (const hour of [...byHour.keys()].sort((a, b) => a - b)) {
      const moves = byHour.get(hour)!;
      // Every country is found as it was the hour before, so swaps and chains in one
      // hour move whole countries.
      const moving = moves.map(change => ({ change, country: this.countryAt(change.from, hour - 1) }));
      const renamedTo = new Set(moves.map(change => change.to));
      for (const { change } of moving) {
        // The old tag is left: from here on it names whoever takes it next, a new country
        // (unless another country is renamed into it this same hour).
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

  /** The country `tag` named at `hour`: an opaque id, equal for one country across
   * every tag it had. Binary search over the tag's changes. */
  countryAt(tag: string, hour: number): string {
    const list = this.held.get(tag);
    return list ? entryAt(list, hour).country : this.unchanged(tag);
  }

  /** Hours when `tag` names another country inside the inclusive range. Find the
   * first change by binary search, then visit changes only, never every hour. */
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

  /** The id of the country a never-renamed tag names, made once per tag. */
  private readonly ids = new Map<string, string>();
  private unchanged(tag: string): string {
    let id = this.ids.get(tag);
    if (id === undefined) { id = JSON.stringify([tag, null]); this.ids.set(tag, id); }
    return id;
  }

  /** The tag `country` (a `countryAt` id) had at `hour`: the tag it was renamed to by
   * then, else the tag it started with. After a country's last run (a former member,
   * a released subject) this still follows its renames, so it is never shown under the
   * tag another country took later. */
  tagOf(country: string, hour: number): string {
    const history = this.tags.get(country);
    if (history) return entryAt(history, hour).tag;
    const [tag] = JSON.parse(country) as [string, number | null];
    return tag;
  }
}

/** The last entry of `list` (in hour order) from `hour` or before, else the first. */
function entryAt<E extends { hour: number }>(list: readonly E[], hour: number): E {
  let low = 0;
  let high = list.length - 1;
  while (low < high) {
    const middle = (low + high + 1) >> 1;
    if (list[middle].hour <= hour) low = middle; else high = middle - 1;
  }
  return list[low];
}

/** Runs kept by tag (in any order), split wherever that tag names another country,
 * then grouped by country. A tag's native run can span a swap of its holders.
 * Each group is in hour order, groups by first run. */
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

/** Group membership by country, split at recorded tag changes, so a renamed
 * member keeps its history and a later holder of its old tag gets only its own. */
export function countryMembership<R extends { tag: string; firstHour: number; lastHour: number }>(runs: readonly R[], lineage: Lineage): Map<string, R[]> {
  return new Map(joinTagChanges(runs, lineage).map(group => [lineage.countryAt(group[0].tag, group[0].firstHour), group]));
}
