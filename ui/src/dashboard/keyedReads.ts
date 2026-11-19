import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { RefObject } from 'react';
import { inFrameNow } from './frameUpdates.ts';
import { asFailure } from './queryError.ts';
import type { Failure } from './queryError';

/** What a read of some asks answered: each value by its ask's key, and the reply's warnings. */
export interface KeyedAnswer<V> { values: Iterable<readonly [string, V]>; warnings: readonly string[] }

/** The asks a want leads with, read alone before the rest: a service's read costs about one frame a distinct hour, so
 * the rows a view shows first (given first) are named in tens of milliseconds, not after a chunk of a window's. */
export const LEAD_ASKS = 24;

/** Values read by key for a widget (place names by province and hour, a battle's details by battle and hour): each key
 * is asked once, the first time it is wanted, in chunks, the first `lead` of them in a chunk of their own; a read in
 * flight is never cancelled because more keys are wanted, so a list opened, sought or paged into a window has its own
 * rows named once that read lands, whatever is asked after it. A key a landed reply did not answer is settled as
 * unanswered, never left loading. */
export class KeyedReads<A, V> {
  readonly values = new Map<string, V>();
  readonly failures = new Map<string, Failure>();
  readonly warnings = new Set<string>();
  private readonly unanswered = new Set<string>();
  private readonly asked = new Set<string>();
  private readonly listeners = new Set<() => void>();
  private readonly keyOf: (ask: A) => string;
  private readonly chunk: number;
  private readonly lead: number;
  constructor(keyOf: (ask: A) => string, chunk: number, lead = chunk) { this.keyOf = keyOf; this.chunk = chunk; this.lead = Math.min(lead, chunk); }

  /** The asks of `wanted` neither answered, failed nor in flight, each once, in order: the first `lead` in a chunk, the
   * rest in chunks of at most `chunk`; each is in flight from now. */
  take(wanted: readonly A[]): A[][] {
    const fresh = new Map<string, A>();
    for (const ask of wanted) {
      const key = this.keyOf(ask);
      if (!this.asked.has(key) && !this.values.has(key) && !this.failures.has(key) && !this.unanswered.has(key)) fresh.set(key, ask);
    }
    for (const key of fresh.keys()) this.asked.add(key);
    const asks = [...fresh.values()], chunks: A[][] = [];
    if (asks.length) chunks.push(asks.slice(0, this.lead));
    for (let at = this.lead; at < asks.length; at += this.chunk) chunks.push(asks.slice(at, at + this.chunk));
    return chunks;
  }

  /** Tells `listener` each time a read lands or fails, as it does (a painter writing what landed into the rows). */
  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  }

  /** Tells every listener that reads landed or failed. */
  told(): void { for (const listener of [...this.listeners]) listener(); }

  /** Whether the key `key` waits for a read; see `pending`. */
  pendingKey(key: string): boolean { return !this.values.has(key) && !this.failures.has(key) && !this.unanswered.has(key); }

  landed(chunk: readonly A[], answer: KeyedAnswer<V>): void {
    for (const [key, value] of answer.values) this.values.set(key, value);
    for (const warning of answer.warnings) this.warnings.add(warning);
    for (const ask of chunk) {
      const key = this.keyOf(ask);
      this.asked.delete(key);
      if (!this.values.has(key)) this.unanswered.add(key);
    }
  }

  failed(chunk: readonly A[], failure: Failure): void {
    for (const ask of chunk) { const key = this.keyOf(ask); this.asked.delete(key); this.failures.set(key, failure); }
  }

  /** Whether `ask` waits for a read (wanted and in flight, or not asked yet): neither answered, failed nor unanswered. */
  pending(ask: A): boolean { return this.pendingKey(this.keyOf(ask)); }

  failure(ask: A): Failure | undefined { return this.failures.get(this.keyOf(ask)); }
}

/** Reads `wanted` by key while `active` (`KeyedReads`), under `owner` (a recording, branch and content revision: a new
 * one starts afresh and stops the reads of the last). `wanted` comes in the order the view shows it: the first `lead`
 * new asks are read alone and the rest once that read settles, so the service answers the rows in view first. `read`
 * asks one chunk. A landing is told to the reads' listeners at once (`useKeyedPainter` writes it into the rows), and
 * the next frame renders their new version. */
export function useKeyedReads<A, V>(owner: string | null, wanted: readonly A[], active: boolean, keyOf: (ask: A) => string,
  read: (chunk: readonly A[], signal: AbortSignal) => Promise<KeyedAnswer<V>>, chunk = 256, lead = LEAD_ASKS): { reads: KeyedReads<A, V>; version: number } {
  const [version, setVersion] = useState(0);
  const held = useRef<{ owner: string | null; reads: KeyedReads<A, V>; controller: AbortController } | null>(null);
  const fresh = () => ({ owner, reads: new KeyedReads<A, V>(keyOf, chunk, lead), controller: new AbortController() });
  if (!held.current || held.current.owner !== owner) {
    held.current?.controller.abort();
    held.current = fresh();
  }
  const reader = useRef(read);
  reader.current = read;
  useEffect(() => {
    if (!active || owner === null) return;
    // Unmounted and mounted again (a remount keeps the ref): the reads stopped then start afresh.
    if (held.current!.controller.signal.aborted) held.current = fresh();
    const { reads, controller } = held.current!;
    // A landing renders in its frame, while playing too: the rows in view wait on it.
    const told = () => {
      reads.told();
      inFrameNow(() => setVersion(value => value + 1));
    };
    const ask = (asks: readonly A[]) => reader.current(asks, controller.signal).then(answer => { if (!controller.signal.aborted) { reads.landed(asks, answer); told(); } },
      error => { if (!controller.signal.aborted) { reads.failed(asks, asFailure(error)); told(); } });
    const [first, ...rest] = reads.take(wanted);
    if (first) void ask(first).then(() => { if (!controller.signal.aborted) for (const asks of rest) void ask(asks); });
  }, [active, owner, wanted]); // eslint-disable-line react-hooks/exhaustive-deps -- `fresh` is of this owner
  useEffect(() => () => held.current?.controller.abort(), []);
  return { reads: held.current!.reads, version };
}

/** What a keyed element shows: its text and whether it still waits for its read. */
export interface KeyedText { text: string; loading: boolean }

/** Writes what `reads` hold into every element under `root` that names one of its keys (`data-keyed` = `kind`,
 * `data-keyed-key` = the key, its text its only child): its text, its title where it has one, and `is-loading` while its
 * read is out. Painted as each read lands and after each commit (a row mounted by a block renders what its memo held),
 * so a name, a detail or a place shows in the frame its read lands in and never waits on a render: the value is the
 * DOM's, the row's structure React's. */
export function useKeyedPainter(reads: KeyedReads<unknown, unknown>, root: RefObject<HTMLElement | null>, kind: string, text: (key: string) => KeyedText) {
  const latest = useRef(text);
  latest.current = text;
  const paint = useRef(() => {});
  paint.current = () => {
    const host = root.current;
    if (!host) return;
    for (const element of host.querySelectorAll<HTMLElement>(`[data-keyed="${kind}"]`)) {
      const shown = latest.current(element.dataset.keyedKey!);
      if (element.textContent !== shown.text) element.textContent = shown.text;
      if (element.hasAttribute('title') && element.title !== shown.text) element.title = shown.text;
      if (element.classList.contains('is-loading') !== shown.loading) element.classList.toggle('is-loading', shown.loading);
    }
  };
  useLayoutEffect(() => paint.current());
  useEffect(() => reads.subscribe(() => paint.current()), [reads]);
}
