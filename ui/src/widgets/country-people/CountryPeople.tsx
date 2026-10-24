import { useEffect, useRef } from 'react';
import { VirtualList } from '../../components/VirtualList';
import WidgetState from '../../components/WidgetState';
import WindowedRows from '../../charts/WindowedRows';
import { dayLabel } from '../../charts/draw';
import { usNumber } from '../../charts/numberFormat';
import type { BoundTrack } from '../../dashboard/model';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { useFollowedProduct } from '../../dashboard/products';
import type { PeopleReply } from '../../generated/dashboard';
import { peopleValue, validatePeople } from './peopleData';
import './people.css';

const staticRow = () => false;

/** Reached details share the country and date of the existing view. */
export default function CountryPeople({ runtime, track, tag, hour, active, view, onWarnings }: {
  runtime: WidgetRuntime; track: BoundTrack | undefined; tag: string | null | undefined; hour: number; active: boolean;
  view: 'manpower' | 'politics'; onWarnings?: (warnings: string[]) => void;
}) {
  const exileScroll = useRef<HTMLDivElement>(null);
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'country-people',
    params: { branchId: track.branch.id, tag, hour: Math.floor(hour) } } : null;
  // During playback the last read hour stays shown while the next loads.
  const result = useFollowedProduct<PeopleReply>(runtime.client, request, ['hour'], active, validatePeople);
  const current = result.data?.tag === tag && result.data?.branchId === track?.branch.id;
  useEffect(() => { onWarnings?.(current ? result.data?.warnings ?? [] : []); return () => onWarnings?.([]); }, [current, result.data, onWarnings]);
  if (!tag || !track) return null;
  if (result.error) return <WidgetState kind="error" what="manpower and politics" detail={result.error} onRetry={result.retry} />;
  if (!result.data || !current) return <WidgetState kind="loading" label="Loading country details…" />;
  const data = result.data;
  const number = (value: number | null) => value === null ? 'Not recorded' : usNumber.format(value);
  return <section className="country-people" aria-label={view === 'manpower' ? 'Recruitment and manpower pools' : 'Elections and governments in exile'}>
    <dl className="country-people-facts">{data[view].map(fact => <div key={fact.label}><dt>{fact.label}</dt><dd>{peopleValue(fact)}</dd></div>)}</dl>
    {view === 'manpower' && data.readHour !== null && data.readHour !== data.hour && <p className="country-people-note">Recruitment last read {dayLabel(data.readHour)}.</p>}
    {view === 'manpower' && <details><summary>State manpower pools ({data.states.length})</summary>
      <div className="country-people-state-heading"><span>State</span><span>Available</span><span>Locked</span><span>Total</span><span>Recruitment</span><span>Recruitable</span></div>
      <WindowedRows items={data.states.map(item => ({ key: String(item.state), item }))} rowHeight={30} label="State manpower pools" className="country-people-states">
        {({ item }) => <div className="country-people-state"><span>{item.state}</span><span>{number(item.available)}</span><span>{number(item.locked)}</span><span>{number(item.total)}</span><span>{peopleValue({ value: item.effective, unit: 'ratio' })}</span><span>{number(item.recruitable)}</span></div>}
      </WindowedRows>
    </details>}
    {data.exiles.length > 0 && <details open={view === 'politics'}><summary>Governments in exile ({data.exiles.length})</summary>
      <div ref={exileScroll} className="country-people-exiles"><VirtualList items={data.exiles} itemKey={exile => exile.tag} estimate={240} label="Governments in exile" scrollRef={exileScroll} isInteractive={staticRow}>
      {exile => <article className="country-people-exile"><strong>{exile.tag}</strong>
        <dl className="country-people-facts">
          <div><dt>Host</dt><dd>{exile.host ?? 'Not recorded'}</dd></div>
          <div><dt>Legitimacy</dt><dd>{exile.legitimacy === null ? 'Not recorded' : peopleValue({ value: exile.legitimacy, unit: 'ratio100' }).replace(/%$/, '')}</dd></div>
          <div><dt>Exile manpower</dt><dd>{number(exile.manpower)}</dd></div>
          <div><dt>Exile manpower cap</dt><dd>{number(exile.cap)}</dd></div>
          <div><dt>Deployed air crews</dt><dd>{number(exile.deployedAir)}</dd></div>
          <div><dt>Army leaders recruited</dt><dd>{number(exile.leaders)}</dd></div>
        </dl>
        {exile.readHour !== null && exile.readHour !== data.hour && <p className="country-people-note">Exile resources last read {dayLabel(exile.readHour)}.</p>}
      </article>}
      </VirtualList></div>
    </details>}
  </section>;
}
