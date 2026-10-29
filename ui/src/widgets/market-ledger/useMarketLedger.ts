import { useCallback, useEffect, useMemo, useState } from 'react';
import type { LogPage } from '../../charts/log/logPages';
import { useContentRevision } from '../../dashboard/contentRevision';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { preparedPrefix } from '../../dashboard/preparedPrefix';
import { useProductWindows } from '../../dashboard/productWindows';
import { useFollowedProduct } from '../../dashboard/products';
import type { MarketLedgerReply } from '../../generated/dashboard';
import { extendCursors, ledgerCursors, ledgerPages, MARKET_LEDGER, PAGE_SIZE, validateLedger } from './ledgerData';
import type { LedgerCountries, LedgerItem } from './ledgerData';

const NO_WARNINGS: readonly string[] = [];

/** Newest-first pages of the prepared ledger from `from` (the branch's start when null)
 * through the hour shown; only wanted pages
 * load. Every ledger pages by the cursor each page returns (`next`): a page is known
 * once the page before it has loaded, so the log grows by one waiting page as it is
 * scrolled; every country's total only sizes that waiting page. Later pages and their
 * cursors belong to the first page they continue: when preparation advances and the
 * first page changes, they are asked again rather than kept beside it. `countries`
 * null asks nothing. */
export function useMarketLedger(runtime: WidgetRuntime, track: BoundTrack | undefined, active: boolean, from: number | null, hour: number | null,
  countries: LedgerCountries | null, kinds: readonly number[]) {
  const content = useContentRevision(runtime.client);
  const request = (before: string | null, lastHour: number | null) => ({ sourceId: track!.source.id, revision: track!.source.revision, product: MARKET_LEDGER,
    params: { branchId: track!.branch.id, tags: countries!.tags, ...countries!.members.length ? { members: countries!.members } : {},
      kinds: [...kinds], ...from === null ? {} : { firstHour: from }, lastHour, pageSize: PAGE_SIZE, ...before === null ? {} : { before } } });
  const first = useFollowedProduct<MarketLedgerReply>(runtime.client, track && countries && hour !== null && kinds.length ? request(null, hour) : null,
    ['lastHour'], active, validateLedger, track?.source);
  // Later pages continue the hour the first page answers; rows still stop at `hour`.
  const read = first.data && first.moment ? (JSON.parse(first.moment) as [number | null])[0] : hour;
  const snapshot = first.data ? JSON.stringify([first.data.lastHour, first.data.total, first.data.next, first.data.rows.length,
    first.data.rows[0]?.id ?? null, preparedPrefix(first.data)]) : null;
  const identity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content, countries, kinds, from, read, snapshot]) : null;
  const [wanted, setWanted] = useState<{ identity: string | null; pages: readonly number[] }>({ identity, pages: [] });
  const wantedPages = wanted.identity === identity ? wanted.pages : NO_PAGES;
  // Each page's cursor, as far as the loaded pages reach: page 0 has none.
  const [cursors, setCursors] = useState<{ identity: string | null; next: readonly string[] }>({ identity, next: [] });
  const known = cursors.identity === identity ? cursors.next : NO_CURSORS;
  const chained = useMemo(() => first.data ? ledgerCursors(first.data, known) : [], [first.data, known]);
  const indices = wantedPages.filter(index => index > 0 && chained[index] !== undefined);
  const chunks = useProductWindows<MarketLedgerReply>(runtime.client, identity, index => request(chained[index] ?? null, read),
    indices, active && !!first.data, validateLedger, track?.source);
  // A loaded page's `next` opens the page after it.
  useEffect(() => {
    if (!first.data) return;
    const next = extendCursors(first.data, known, index => chunks.data.get(index));
    if (next !== known) setCursors({ identity, next });
  }, [first.data, known, chunks.data, identity]);
  const pages = useMemo<readonly LogPage<LedgerItem>[]>(() => !first.data || !first.data.rows.length ? []
    : ledgerPages(first.data, chained.length, index => chunks.data.get(index), chunks.errors, hour, from ?? track?.branch.firstHour ?? 0),
  [first.data, chunks.data, chunks.errors, chained.length, hour, from, track?.branch.firstHour]);
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
