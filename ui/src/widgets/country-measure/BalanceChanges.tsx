import type { Gate } from '../../dashboard/regionGate';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { type Failure, asFailure } from '../../dashboard/queryError.ts';
import { useEffect, useMemo, useState } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { SeriesEventPage } from '../../generated/dashboard';
import WindowedRows from '../../charts/WindowedRows';
import { dayLabel } from '../../charts/draw';
import { Select } from '../../components/Select';
import WidgetState from '../../components/WidgetState';
import { validateMeasureDrops } from './measureSeriesData';
import { loadTransientProduct } from '../../dashboard/products';
import { netSpend } from './measureData';
import { productRequest } from '../../dashboard/request.ts';

export const DROP_LIST_PAGE = 32n;
export function dropListRange(first: bigint, total: bigint, end: bigint | null): { last: bigint; count: number } {
  const limit = end === null || end > total ? total : end;
  const last = first + DROP_LIST_PAGE < limit ? first + DROP_LIST_PAGE : limit;
  return { last, count: Number(last > first ? last - first : 0n) };
}

export default function BalanceChanges({ client, gate, source, branchId, measure, tags, initialTag, firstOrdinal = '0', endOrdinal,
  digits, unit, visibleThrough, label, onClose, onPick }: { client: DashboardClient; gate: Pick<Gate, 'within'>; source: DashboardSource; branchId: string; measure: string;
  tags: readonly string[]; initialTag: string; firstOrdinal?: string; endOrdinal?: string; digits: number; unit: string;
  visibleThrough: number; label: (tag: string) => string; onClose: () => void; onPick: (tag: string, hour: number) => void }) {
  const [tag, setTag] = useState(initialTag);
  const rangeFirst = tag === initialTag ? BigInt(firstOrdinal) : 0n;
  const end = tag === initialTag && endOrdinal !== undefined ? BigInt(endOrdinal) : null;
  const [position, setPosition] = useState({ tag: initialTag, first: rangeFirst });
  const first = position.tag === tag ? position.first : rangeFirst;
  const owner = JSON.stringify([source.id, source.revision, branchId, measure, tag, first.toString(), end?.toString(), Math.floor(visibleThrough)]);
  const [reply, setReply] = useState<{ owner: string; data?: SeriesEventPage; error?: Failure }>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    const count = end === null ? Number(DROP_LIST_PAGE) : Number(end - first < DROP_LIST_PAGE ? end - first : DROP_LIST_PAGE);
    const to = gate.within(visibleThrough);
    if (to === null) return () => controller.abort();
    void loadTransientProduct<unknown>(client, productRequest({ sourceId: source.id, revision: source.revision, product: 'events-page',
      params: { branchId, family: { kind: 'country-measure', measure }, countries: ([tag]).map(countryFromKey), cursor: first.toString(), pageSize: count, to } }), controller.signal)
      .then(value => { if (!controller.signal.aborted) setReply({ owner, data: validateMeasureDrops(value) }); })
      .catch(error => { if (!controller.signal.aborted) setReply(previous => previous?.owner === owner ? { ...previous, error: asFailure(error) } : { owner, error: asFailure(error) }); });
    return () => controller.abort();
  }, [client, owner, attempt]);
  const data = reply?.owner === owner ? reply.data : undefined;
  const rows = useMemo(() => data?.rows.map((event, index) => ({ key: (first + BigInt(index)).toString(), item: event.row })) ?? [], [data]);
  const total = data ? BigInt(data.total) : first;
  const { last } = dropListRange(first, total, end);
  const limit = end === null || end > total ? total : end;
  useEffect(() => {
    if (data && first >= limit && first > rangeFirst) setPosition({ tag, first: rangeFirst });
  }, [data, first, limit, rangeFirst, tag]);
  return <section className="country-measure-changes" aria-label="Balance changes">
    <div className="chart-line-key"><strong>Balance changes</strong>
      {tags.length > 1 && <Select label="Country for balance changes" value={tag} options={tags.map(tag => ({ value: tag, label: label(tag) }))} onChange={setTag} />}
      <button type="button" className="button small" onClick={onClose}>Close</button></div>
    {reply?.owner === owner && reply.error && data && <WidgetState kind="error" failure={reply.error} onRetry={() => setAttempt(before => before + 1)} />}
    {reply?.owner === owner && reply.error && !data ? <WidgetState kind="error" failure={reply.error} onRetry={() => setAttempt(before => before + 1)} />
      : !data ? <WidgetState kind="loading" label="Loading balance changes…" />
        : <WindowedRows label="Recorded balance changes" items={rows} rowHeight={28} empty={<p>No recorded balance changes.</p>}>
          {row => <button type="button" className="country-measure-change" onClick={() => onPick(tag, row.item.hour)}>
            <span>{dayLabel(row.item.hour)}</span><span>{netSpend(row.item, 5, digits)} {unit}</span>
          </button>}
        </WindowedRows>}
    <div className="chart-line-key"><button type="button" className="button small" disabled={first <= rangeFirst}
      onClick={() => setPosition({ tag, first: first - DROP_LIST_PAGE > rangeFirst ? first - DROP_LIST_PAGE : rangeFirst })}>Previous</button>
      <span aria-live="polite">{data ? `${rows.length ? (first - rangeFirst + 1n).toLocaleString() : '0'}–${rows.length ? (last - rangeFirst).toLocaleString() : '0'} of ${(limit > rangeFirst ? limit - rangeFirst : 0n).toLocaleString()}` : 'Loading…'}</span>
      <button type="button" className="button small" disabled={!data || last >= limit} onClick={() => setPosition({ tag, first: data?.cursor ? BigInt(data.cursor) : last })}>Next</button></div>
  </section>;
}
