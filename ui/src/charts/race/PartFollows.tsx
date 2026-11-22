import type { FollowQuery, GatedHour } from '../../dashboard/regionTime';
import type { Gate } from '../../dashboard/regionGate';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useEffect, useMemo, useState } from 'react';
import type { RefObject } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import { loadProduct } from '../../dashboard/products';
import type { FollowEntry, FollowStore } from '../../dashboard/follow/store';
import { followsHead, useFollow } from '../../dashboard/follow/useFollow.ts';
import type { SeriesStateReply } from '../../generated/dashboard';
import { keyedAnswer, keyedStates, measureAnswer, measureParts, measureStates } from './measureParts.ts';
import type { MeasureParts, PartValues } from './measureParts';
import { productRequest } from '../../dashboard/request.ts';
import type { ParamsFor } from '../../dashboard/regionTime';

export class PartsHead {
  private entries = new Map<string, FollowEntry | null>();
  private ticks = 0;
  private wakes = new Set<() => void>();
  private made: { ticks: number; tags: readonly string[]; values: Map<string, PartValues> } | null = null;
  constructor(readonly parts: MeasureParts) {}

  version = () => this.ticks;

  subscribe = (wake: () => void) => { this.wakes.add(wake); return () => { this.wakes.delete(wake); }; };

  hold(id: string, entry: FollowEntry | null) {
    if (this.entries.get(id) === entry) return;
    this.entries.set(id, entry);
    this.land();
  }

  land() {
    this.ticks++;
    for (const wake of [...this.wakes]) wake();
  }

  followed(): boolean {
    const ids = this.parts.kind === 'measures' ? this.parts.items.map(item => item.id) : ['keyed'];
    return ids.every(id => this.entries.get(id) != null);
  }

  values(tags: readonly string[]): Map<string, PartValues> {
    if (this.made && this.made.ticks === this.ticks && this.made.tags === tags) return this.made.values;
    const parts = this.parts;
    const values = parts.kind === 'measures'
      ? measureParts(parts.items, new Map(parts.items.map(item => [item.id, measureAnswer(this.entries.get(item.id)?.answer, tags, parts.decimals)])), tags)
      : keyedAnswer(this.entries.get('keyed')?.answer, parts.partBits, parts.decimals);
    this.made = { ticks: this.ticks, tags, values };
    return values;
  }
}

function PartFollow({ store, query, root, id, head }: { store: FollowStore | null; query: FollowQuery | null;
  root: RefObject<Element | null>; id: string; head: PartsHead }) {
  const followed = useFollow(store, query, root, () => head.land());
  const entry = followsHead(followed) ? followed.entry : null;
  useEffect(() => { head.hold(id, entry); }, [head, id, entry]);
  useEffect(() => () => head.hold(id, null), [head, id]);
  return null;
}

export function PartFollows({ store, gate, branchId, tags, on, root, head }: { store: FollowStore | null; gate: Pick<Gate, 'follow'>; branchId: string | undefined;
  tags: readonly string[]; on: boolean; root: RefObject<Element | null>; head: PartsHead }) {
  const parts = head.parts;
  const reads = useMemo(() => tags.map(tag => ({ country: countryFromKey(tag) })), [tags]);
  const query = (family: unknown) => on && branchId && reads.length ? gate.follow({ product: 'series-state', params: { branchId, family, reads } }) : null;
  if (parts.kind === 'keyed') return <PartFollow store={store} query={query(parts.family)} root={root} id="keyed" head={head} />;
  return <>{parts.items.map(item => <PartFollow key={item.id} store={store} query={query(item.measure)} root={root} id={item.id} head={head} />)}</>;
}

function validateStates(value: unknown): SeriesStateReply {
  const reply = value as SeriesStateReply;
  if (!reply || typeof reply !== 'object' || !Array.isArray(reply.states) || typeof reply.decimals !== 'number') throw new Error('The parts state reply is malformed.');
  return reply;
}

export function usePartStates(client: DashboardClient, source: DashboardSource | undefined, branchId: string | undefined, parts: MeasureParts | undefined,
  tags: readonly string[], hour: GatedHour | null, enabled: boolean) {
  const key = enabled && source && branchId && parts && tags.length && hour !== null ? JSON.stringify([source.id, source.revision, branchId, parts.kind === 'keyed' ? parts.family
    : parts.items.map(item => item.measure), tags, hour]) : null;
  const [state, setState] = useState<{ key: string; values?: Map<string, PartValues>; error?: unknown }>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!key || !parts || hour === null) return;
    const controller = new AbortController();
    const ask = (family: unknown) => loadProduct(client, productRequest({ sourceId: source!.id, revision: source!.revision, product: 'series-state',
      params: { branchId, family: family as ParamsFor<'series-state'>['family'], reads: tags.map(tag => ({ country: countryFromKey(tag), hour })) } }), controller.signal, validateStates, source);
    const made = parts.kind === 'keyed'
      ? ask(parts.family).then(reply => keyedStates(reply, hour))
      : Promise.all(parts.items.map(item => ask(item.measure).then(reply => [item.id, measureStates(reply, hour)] as const)))
        .then(byPart => measureParts(parts.items, new Map(byPart), tags));
    made.then(values => { if (!controller.signal.aborted) setState({ key, values }); },
      error => { if (!controller.signal.aborted) setState({ key, error }); });
    return () => controller.abort();
  }, [key, client, attempt]);
  const held = state?.key === key ? state : undefined;
  return { values: held?.values, error: held?.error, retry: () => setAttempt(count => count + 1) };
}
