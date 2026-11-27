import { isFailedFact } from './countryFacts.ts';
import type { FlagValue } from './countryFlags';
import { nameReadFrom, nameReadMetadata, type NameFact, type NameRead } from './nameReads.ts';
import { failureText, type Failure } from './queryError.ts';
import { compareTags } from './tagOrder.ts';

type NameValue = NameFact | null | undefined | Failure;
export type CountryLabelValue = Readonly<{ name: string; flag?: string; known?: boolean; nameFailed?: boolean; nameDetail?: string;
  nameFailure?: Failure; flagPending?: boolean; flagDetail?: string; flagReason?: string }>;
export type CountryLabel = (tag: string) => CountryLabelValue;

export function flagReadMetadata(read: FlagValue | undefined): Readonly<{ flag?: string; flagPending: boolean; flagDetail?: string; flagReason?: string }> {
  return { flag: read && !isFailedFact(read) ? read.url : undefined, flagPending: read === undefined,
    ...(isFailedFact(read) ? { flagDetail: read.message } : {}),
    flagReason: isFailedFact(read) ? failureText(read, 'The flag request failed.') : read?.reason ?? (read === null ? 'No readable flag was returned for this country at this hour.' : undefined) };
}

const countryLabel = (name: NameValue, flag: FlagValue | undefined): CountryLabelValue => ({
  ...nameReadMetadata(nameReadFrom(name, true)), ...flagReadMetadata(flag),
});
const pending = countryLabel(undefined, undefined);
const pendingLabel: CountryLabel = () => pending;
interface LabelNode { readonly next: Map<string, WeakMap<CountryLabelValue, LabelNode>>; label?: CountryLabel }

export class CountryLabelIndex {
  private readonly values = new Map<NameValue, Map<FlagValue | undefined, CountryLabelValue>>();
  private readonly absentReasons = new Map<string, { readonly reason: string }>();
  private readonly root: LabelNode = { next: new Map() };
  private readonly tables = new WeakMap<ReadonlyMap<string, NameRead>, WeakMap<ReadonlyMap<string, FlagValue>, (hour: number) => CountryLabel>>();
  private readonly bounds: readonly [number, number] | null;

  constructor(bounds: readonly [number, number] | null = [-Infinity, Infinity]) {
    this.bounds = bounds;
  }

  private internReason(reason: string): { readonly reason: string } {
    let held = this.absentReasons.get(reason);
    if (!held) this.absentReasons.set(reason, held = { reason });
    return held;
  }

  at(names: ReadonlyMap<string, NameRead>, flags: ReadonlyMap<string, FlagValue>): (hour: number) => CountryLabel {
    let byFlags = this.tables.get(names);
    const held = byFlags?.get(flags);
    if (held) return held;
    const hours = new Map<number, Set<string>>();
    for (const keys of [names.keys(), flags.keys()]) for (const key of keys) {
      const [hour, tag] = JSON.parse(key) as [number, string];
      let tags = hours.get(hour);
      if (!tags) hours.set(hour, tags = new Set());
      tags.add(tag);
    }
    const labels = new Map<number, CountryLabel>();
    for (const [hour, tags] of hours) {
      const byTag = new Map<string, CountryLabelValue>();
      let node = this.root;
      for (const tag of [...tags].sort(compareTags)) {
        const key = JSON.stringify([hour, tag]), read = names.get(key), flagRead = flags.get(key);
        const name: NameValue = read?.kind === 'value' ? read.value
          : read?.kind === 'failed' ? read.failure
          : read?.kind === 'loading' ? undefined
          : read?.kind === 'absent' && read.reason !== undefined ? this.internReason(read.reason)
          : null;
        const reading = this.bounds !== null && names.has(key) && hour >= this.bounds[0] && hour <= this.bounds[1];
        const flag = flagRead === undefined && !reading ? null : flagRead;
        let byFlag = this.values.get(name);
        if (!byFlag) this.values.set(name, byFlag = new Map());
        let label = byFlag.get(flag);
        if (!label) { label = countryLabel(name, flag); byFlag.set(flag, label); }
        byTag.set(tag, label);
        let next = node.next.get(tag);
        if (!next) node.next.set(tag, next = new WeakMap());
        let child = next.get(label);
        if (!child) { child = { next: new Map() }; next.set(label, child); }
        node = child;
      }
      const table: ReadonlyMap<string, CountryLabelValue> = byTag;
      node.label ??= tag => table.get(tag) ?? pending;
      labels.set(hour, node.label);
    }
    const table: ReadonlyMap<number, CountryLabel> = labels;
    const labelAt = (hour: number) => table.get(hour) ?? pendingLabel;
    if (!byFlags) this.tables.set(names, byFlags = new WeakMap());
    byFlags.set(flags, labelAt);
    return labelAt;
  }
}
