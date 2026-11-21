import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { RefObject } from 'react';
import { inFrameNow } from './frameUpdates.ts';
import { asFailure } from './queryError.ts';
import type { Failure } from './queryError';

export interface KeyedAnswer<V> { values: Iterable<readonly [string, V]>; warnings: readonly string[] }

export const LEAD_ASKS = 24;

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

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  }

  told(): void { for (const listener of [...this.listeners]) listener(); }

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

  pending(ask: A): boolean { return this.pendingKey(this.keyOf(ask)); }

  failure(ask: A): Failure | undefined { return this.failures.get(this.keyOf(ask)); }
}

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
    if (held.current!.controller.signal.aborted) held.current = fresh();
    const { reads, controller } = held.current!;
    const told = () => {
      reads.told();
      inFrameNow(() => setVersion(value => value + 1));
    };
    const ask = (asks: readonly A[]) => reader.current(asks, controller.signal).then(answer => { if (!controller.signal.aborted) { reads.landed(asks, answer); told(); } },
      error => { if (!controller.signal.aborted) { reads.failed(asks, asFailure(error)); told(); } });
    const [first, ...rest] = reads.take(wanted);
    if (first) void ask(first).then(() => { if (!controller.signal.aborted) for (const asks of rest) void ask(asks); });
  }, [active, owner, wanted]);
  useEffect(() => () => held.current?.controller.abort(), []);
  return { reads: held.current!.reads, version };
}

export interface KeyedText { text: string; loading: boolean }

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
