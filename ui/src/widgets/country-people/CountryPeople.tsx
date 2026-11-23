import { answersKey, countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useMemo, useRef } from 'react';
import { VirtualList } from '../../components/VirtualList';
import WidgetState from '../../components/WidgetState';
import WindowedRows from '../../charts/WindowedRows';
import { dayLabel } from '../../charts/draw';
import { usNumber } from '../../charts/numberFormat';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { SPEC, type RegionAction } from '../../dashboard/regionTime';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { factTexts, peopleStructure, peopleValue, validatePeople } from './peopleData';
import './people.css';
import type { PeopleReply } from '../../generated/dashboard';

const staticRow = () => false;

export default function CountryPeople({ runtime, widget, track, tag, time = SPEC, active, view, onWarnings }: {
  runtime: WidgetRuntime; widget: WidgetSpec; track: BoundTrack | undefined; tag: string | null | undefined; time?: RegionAction; active: boolean;
  view: 'manpower' | 'politics'; onWarnings?: (warnings: string[]) => void;
}) {
  const exileScroll = useRef<HTMLDivElement>(null);
  const facts = useRef<HTMLDListElement | null>(null);
  const params = useMemo(() => track && tag ? { country: countryFromKey(tag) } : null, [track, tag]);
  const paint = useCallback((reply: PeopleReply) => {
    const list = facts.current;
    if (!list) return;
    const texts = factTexts(reply[view]);
    for (const cell of list.querySelectorAll<HTMLElement>('dd[data-fact]')) {
      const text = texts.get(cell.dataset.fact ?? '');
      if (text !== undefined && cell.textContent !== text) cell.textContent = text;
    }
  }, [view]);
  const structure = useCallback((reply: PeopleReply) => peopleStructure(reply, view), [view]);
  const head = useHeadProduct(runtime, useGate(runtime, widget, time), 'country-people', params, { active, validate: validatePeople, structure, paint });
  const data = head.data;
  const current = answersKey(data?.asked, tag) && data?.branchId === track?.branch.id;
  useEffect(() => { onWarnings?.(current ? data?.warnings ?? [] : []); return () => onWarnings?.([]); }, [current, data, onWarnings]);
  if (!tag || !track) return null;
  if (head.error && !data) return <WidgetState kind="error" scale="section" what="manpower and politics" failure={head.error} onRetry={head.retry} />;
  if (!data || !current) return <div ref={head.root}><WidgetState kind="loading" scale="section" label="Loading country details…" /></div>;
  const number = (value: number | null) => value === null ? 'Not recorded' : usNumber.format(value);
  return <section ref={head.root} className="country-people" aria-label={view === 'manpower' ? 'Recruitment and manpower pools' : 'Elections and governments in exile'}>
    <dl ref={facts} className="country-people-facts">{data[view].map(fact => <div key={fact.label}><dt>{fact.label}</dt><dd data-fact={fact.label}>{peopleValue(fact)}</dd></div>)}</dl>
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
