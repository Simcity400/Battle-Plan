import { useCallback, useEffect, useMemo, useState } from 'react';
import type { LogPage } from '../../charts/log/logPages';
import { useContentRevision } from '../../dashboard/contentRevision';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { preparedPrefix } from '../../dashboard/preparedPrefix';
import { useProductWindows } from '../../dashboard/productWindows';
import { useProduct } from '../../dashboard/products';
import type { MarketLedgerReply } from '../../generated/dashboard';
import { extendCursors, ledgerCursors, ledgerRows, MARKET_LEDGER, PAGE_SIZE, validateLedger } from './ledgerData';
import type { LedgerCountries, LedgerItem } from './ledgerData';

const NO_WARNINGS: readonly string[] = [];

/** Newest-first pages of the prepared ledger through the hour shown; only wanted pages
 * load. Every ledger pages by the cursor each page returns (`next`): a page is known
 * once the page before it has loaded, so the log grows by one waiting page as it is
 * scrolled; every country's total only sizes that waiting page. Later pages and their
 * cursors belong to the first page they continue: when preparation advances and the
 * first page changes, they are asked again rather than kept beside it. `countries`
 * null asks nothing. */
export function useMarketLedger(runtime: WidgetRuntime, track: BoundTrack | undefined, active: boolean, hour: number | null,
  countries: LedgerCountries | null, kinds: readonly number[]) {
  const content = useContentRevision(runtime.client);
  const request = (before: string | null) => ({ sourceId: track!.source.id, revision: track!.source.revision, product: MARKET_LEDGER,
    params: { branchId: track!.branch.id, tags: countries!.tags, ...countries!.members.length ? { members: countries!.members } : {},
      kinds: [...kinds], lastHour: hour, pageSize: PAGE_SIZE, ...before === null ? {} : { before } } });
  const first = useProduct<MarketLedgerReply>(runtime.client, track && countries && hour !== null && kinds.length ? request(null) : null,
    active, validateLedger, track?.source);
  const total = first.data?.total == null ? null : Number(first.data.total);
  const snapshot = first.data ? JSON.stringify([first.data.lastHour, first.data.total, first.data.next, first.data.rows.length,
    first.data.rows[0]?.id ?? null, preparedPrefix(first.data)]) : null;
  const identity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content, countries, kinds, hour, snapshot]) : null;
  const [wanted, setWanted] = useState<{ identity: string | null; pages: readonly number[] }>({ identity, pages: [] });
  const wantedPages = wanted.identity === identity ? wanted.pages : NO_PAGES;
  // Each page's cursor, as far as the loaded pages reach: page 0 has none.
  const [cursors, setCursors] = useState<{ identity: string | null; next: readonly string[] }>({ identity, next: [] });
  const known = cursors.identity === identity ? cursors.next : NO_CURSORS;
  const chained = useMemo(() => first.data ? ledgerCursors(first.data, known) : [], [first.data, known]);
  const indices = wantedPages.filter(index => index > 0 && chained[index] !== undefined);
  const chunks = useProductWindows<MarketLedgerReply>(runtime.client, identity, index => request(chained[index] ?? null),
    indices, active && !!first.data, validateLedger, track?.source);
  // A loaded page's `next` opens the page after it.
  useEffect(() => {
    if (!first.data) return;
    const next = extendCursors(first.data, known, index => chunks.data.get(index));
    if (next !== known) setCursors({ identity, next });
  }, [first.data, known, chunks.data, identity]);
  const pages = useMemo<readonly LogPage<LedgerItem>[]>(() => {
    if (!first.data || !first.data.rows.length) return [];
    const branchStart = track?.branch.firstHour ?? 0;
    return Array.from({ length: chained.length }, (_, index) => {
      const reply = index === 0 ? first.data : chunks.data.get(index);
      const rows = reply ? ledgerRows(reply, index) : undefined;
      const count = rows?.length ?? (total === null ? PAGE_SIZE : Math.max(1, Math.min(PAGE_SIZE, total - index * PAGE_SIZE)));
      const start = rows?.length ? rows.at(-1)!.hour : branchStart;
      const end = rows?.length ? rows[0].hour : hour ?? branchStart;
      return { key: String(index), start, end, count, rows, error: chunks.errors.get(index) };
    });
  }, [first.data, chunks.data, chunks.errors, total, chained.length, hour, track?.branch.firstHour]);
  const onWantedPages = useCallback((keys: readonly string[]) => {
    const next = keys.map(Number).filter(Number.isInteger);
    setWanted(previous => previous.identity === identity && previous.pages.join(',') === next.join(',') ? previous : { identity, pages: next });
  }, [identity]);
  const retry = useCallback(() => { first.retry(); chunks.retry(); }, [first.retry, chunks.retry]); // eslint-disable-line react-hooks/exhaustive-deps
  return { data: first.data, error: first.error, retry, pages, onWantedPages, identity, errors: chunks.errors,
    prefix: first.data ? preparedPrefix(first.data) : null, warnings: first.data?.warnings ?? NO_WARNINGS };
}

const NO_PAGES: readonly number[] = [];
const NO_CURSORS: readonly string[] = [];
