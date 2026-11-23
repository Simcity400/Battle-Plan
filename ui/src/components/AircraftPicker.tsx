import { failureText } from '../dashboard/queryError.ts';
import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';
import type { AircraftOrigin, AircraftOriginCount, AircraftOriginFilter, FlyAircraftStatus } from '../generated/dashboard';
import type { DashboardClient, DashboardSource } from '../dashboard/client';
import { CATALOGUE_NOTE, aircraftRowInteractive, countryTag, originFilter, originLabel, refusalLine, refusalText, sameOrigin, useAircraftCatalogue } from '../dashboard/aircraftCatalogue';
import { aircraftTitle } from '../dashboard/aircraftChoice';
import type { AircraftChoice } from '../dashboard/aircraftChoice';
import type { AircraftPages } from '../dashboard/aircraftCatalogue';
import { Icon } from './Icons';
import { FailureLine } from './FailureLine';
import { VirtualList } from './VirtualList';
import './aircraftPicker.css';

export const NO_AIRCRAFT = 'No aircraft is chosen: Fly mode is a free camera until you choose one.';

export function statusText(chosen: AircraftChoice | null, status: FlyAircraftStatus | null): { text: string; alert: boolean; detail?: string } | null {
  if (!chosen) return { text: NO_AIRCRAFT, alert: false };
  if (!status || status.aircraft !== chosen.name) return null;
  switch (status.state.kind) {
    case 'loading': return { text: `Loading ${chosen.title}…`, alert: false };
    case 'ready': return { text: `Flying ${chosen.title}`, alert: false };
    case 'refused': return { text: `${chosen.title} can’t be flown.`, alert: true, detail: status.state.reason };
  }
}

const REFUSED = 'refused';
type Item = number | typeof REFUSED;

function originText(origin: AircraftOrigin | null): { text: string; tooltip?: string } {
  if (!origin) return { text: 'Source unknown', tooltip: 'No game or mod file registers this model' };
  if (origin.kind === 'game') return { text: originLabel(origin), tooltip: origin.dlc ? `From the ${origin.dlc} DLC` : undefined };
  return { text: originLabel(origin) };
}

function ChosenState({ chosen, status }: { chosen: AircraftChoice; status: FlyAircraftStatus | null }) {
  const state = status?.aircraft === chosen.name ? status.state.kind : 'loading';
  if (state === 'refused') return <span className="aircraft-row-state is-alert"><Icon name="alert" /></span>;
  return <span className="aircraft-row-state">{state === 'loading' ? <span className="spinner" role="status" aria-label="Loading" /> : <Icon name="check" />}</span>;
}

function Row({ index, pages, chosen, status, onChoose, request, retry }: {
  index: number; pages: AircraftPages; chosen: AircraftChoice | null; status: FlyAircraftStatus | null; onChoose: (aircraft: AircraftChoice | null) => void;
  request: (first: number, last: number) => void; retry: (index: number) => void;
}) {
  const entry = pages.entry(index);
  const error = pages.error(Math.floor(index / pages.size));
  useEffect(() => { if (!entry && !error) request(index, index); }, [entry, error, index, request]);
  if (!entry) {
    if (error) return <FailureLine className="aircraft-row is-failed" detail={error.message} message={failureText(error, 'Couldn’t load these rows.')} onRetry={() => retry(index)} />;
    return <div className="aircraft-row is-loading" aria-hidden="true"><span className="aircraft-tile skeleton" /><span className="skeleton" /></div>;
  }
  const refused = entry.refusal;
  const title = aircraftTitle(entry);
  const tag = countryTag(entry);
  const picked = !!chosen && entry.name === chosen.name;
  const from = originText(entry.origin);
  const others = entry.titles.slice(1).join(', ');
  return <button type="button" className="aircraft-row" disabled={!!refused} aria-pressed={picked} data-origin={entry.origin?.kind ?? 'unknown'}
    data-tooltip={picked ? 'Click again to go back to the free camera' : undefined} onClick={() => onChoose(picked ? null : { name: entry.name, title })}>
    <span className="aircraft-tile" aria-hidden="true">{tag ?? <Icon name="plane" />}</span>
    <span className="aircraft-row-text">
      <span className="aircraft-row-title"><span className="aircraft-row-name">{title}</span>{entry.kind === 'mesh' && <small className="aircraft-badge">No animation</small>}</span>
      {refused ? <small className="aircraft-row-refusal" data-detail={refused.reason}>{refusalText(refused.kind)}</small>
        : <small className="aircraft-row-detail"><span className="aircraft-row-source" data-tooltip={from.tooltip}>{from.text}</span>
          {others && <span className="aircraft-row-also" data-tooltip={`Also ${others}`}>Also {others}</span>}</small>}
    </span>
    {picked && <ChosenState chosen={chosen} status={status} />}
  </button>;
}

function OriginTabs({ found, origins, origin, onOrigin }: {
  found: number; origins: readonly AircraftOriginCount[]; origin: AircraftOriginFilter | null; onOrigin: (origin: AircraftOriginFilter | null) => void;
}) {
  return <div className="aircraft-origins" role="group" aria-label="Where the aircraft come from">
    <button type="button" className="aircraft-origin" aria-pressed={origin === null} onClick={() => onOrigin(null)}>
      All<span className="aircraft-origin-count">{found.toLocaleString()}</span></button>
    {origins.map(({ origin: known, count }) => {
      const filter = originFilter(known);
      return <button key={known.kind === 'game' ? 'game' : `mod:${known.id}`} type="button" className="aircraft-origin" data-origin={known.kind}
        aria-pressed={sameOrigin(origin, filter)} onClick={() => onOrigin(filter)}>
        {originLabel(known)}<span className="aircraft-origin-count">{count.toLocaleString()}</span></button>;
    })}
  </div>;
}

export function AircraftCatalogueView({ pages, version, origin, onOrigin, query, onQuery, chosen, status, onChoose, request, retry }: {
  pages: AircraftPages | null; version: number; origin: AircraftOriginFilter | null; onOrigin: (origin: AircraftOriginFilter | null) => void;
  query: string; onQuery: (query: string) => void;
  chosen: AircraftChoice | null; status: FlyAircraftStatus | null; onChoose: (aircraft: AircraftChoice | null) => void;
  request: (first: number, last: number) => void; retry: (index: number) => void;
}) {
  const scroll = useRef<HTMLDivElement>(null);
  const matching = pages?.matching ?? 0;
  const flyable = pages?.flyable() ?? 0;
  const items = useMemo(() => {
    const rows: Item[] = Array.from({ length: matching }, (_, index) => index);
    if (flyable < matching) rows.splice(flyable, 0, REFUSED);
    return rows;
  }, [matching, flyable]);
  const interactive = useCallback((item: Item) => item !== REFUSED && !!pages && aircraftRowInteractive(pages, item), [pages, version]);
  const tabs = useRef<{ found: number; origins: readonly AircraftOriginCount[] } | null>(null);
  if (pages?.found != null) tabs.current = { found: pages.found, origins: pages.origins };
  const listed = pages?.found != null ? pages.origins : null;
  useEffect(() => {
    if (origin && listed && !listed.some(known => sameOrigin(originFilter(known.origin), origin))) onOrigin(null);
  }, [origin, listed, onOrigin]);
  const line = statusText(chosen, status);
  const refusals = refusalLine(pages?.refusals ?? []);
  const failed = pages?.firstError();
  const empty = query.trim() ? 'No aircraft match the search.' : origin ? 'No aircraft come from here.' : 'There are no aircraft to list.';
  return <>
    {tabs.current && tabs.current.origins.length > 0 && <OriginTabs found={tabs.current.found} origins={tabs.current.origins} origin={origin} onOrigin={onOrigin} />}
    <div className="aircraft-picker-search">
      <Icon name="search" />
      <input type="search" autoFocus placeholder="Search aircraft, equipment or country" aria-label="Search aircraft"
        value={query} onChange={event => onQuery(event.currentTarget.value)} />
    </div>
    {line?.alert && <FailureLine className="aircraft-picker-failure" message={line.text} detail={line.detail} />}
    {!chosen && line && <p className="aircraft-picker-hint" role="status">{line.text}</p>}
    <div className="aircraft-picker-list" ref={scroll}>
      {pages && pages.matching === null && !failed && <p className="aircraft-picker-note" role="status">Reading aircraft…</p>}
      {pages && pages.matching === null && failed && <FailureLine className="aircraft-picker-failure" detail={failed.message} message={failureText(failed, 'Couldn’t load aircraft.')} onRetry={() => retry(0)} />}
      {pages?.matching === 0 && <p className="aircraft-picker-note">{empty}</p>}
      {pages && items.length > 0 && <VirtualList identity={pages} items={items} itemKey={String} estimate={56} label="Aircraft" scrollRef={scroll} isInteractive={interactive}>
        {item => item === REFUSED
          ? <p className="aircraft-group" data-tooltip={refusals?.tooltip} data-detail={refusals?.detail}>Can’t be flown · {(matching - flyable).toLocaleString()}</p>
          : <Row index={item} pages={pages} chosen={chosen} status={status} onChoose={onChoose} request={request} retry={retry} />}
      </VirtualList>}
    </div>
    {pages && pages.warnings.length > 0 && <p className="aircraft-picker-note" data-detail={pages.warnings.join('\n')}>{CATALOGUE_NOTE}</p>}
  </>;
}

export function AircraftPanel({ id, onClose, children }: { id?: string; onClose: () => void; children: ReactNode }) {
  return <section id={id} className="aircraft-picker" aria-label="Aircraft" onKeyDown={event => {
    if (event.key === 'Escape') { event.stopPropagation(); onClose(); }
  }}>
    <div className="ui-popover-heading"><strong>Aircraft</strong><div className="ui-popover-actions">
      <button type="button" className="icon-button ui-popover-action" aria-label="Close the aircraft list" data-tooltip="Close" onClick={onClose}><Icon name="close" /></button>
    </div></div>
    {children}
  </section>;
}

export default function AircraftPicker({ client, source, chosen, status, onChoose, host, prompt = false }: {
  client: DashboardClient; source: DashboardSource | null; chosen: AircraftChoice | null; status: FlyAircraftStatus | null;
  onChoose: (aircraft: AircraftChoice | null) => void; host: HTMLElement | null; prompt?: boolean;
}) {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const [origin, setOrigin] = useState<AircraftOriginFilter | null>(null);
  const [query, setQuery] = useState('');
  const shown = open && !!host && !!source;
  const { pages, version, request, retry } = useAircraftCatalogue(client, source, origin, query, shown);
  useEffect(() => { if (prompt && chosen === null && source) setOpen(true); }, [prompt, source?.id]);
  const close = useCallback(() => { setOpen(false); trigger.current?.focus({ preventScroll: true }); }, []);
  const line = statusText(chosen, status);
  const label = chosen?.title ?? 'No aircraft chosen';
  if (!source) return <button type="button" className="icon-button aircraft-picker-trigger" disabled aria-label="Aircraft: none chosen"
    data-tooltip="Open a campaign to choose an aircraft"><Icon name="plane" /><span>{label}</span></button>;
  return <>
    <button ref={trigger} type="button" className={chosen ? 'icon-button aircraft-picker-trigger' : 'icon-button aircraft-picker-trigger is-unchosen'}
      aria-expanded={shown} aria-controls={shown ? id : undefined} disabled={!host} aria-label={`Aircraft: ${chosen ? chosen.title : 'none chosen'}`}
      data-tooltip={line?.text ?? 'Choose the aircraft to fly'} data-detail={line?.detail} onClick={() => setOpen(value => !value)}><Icon name="plane" /><span>{label}</span></button>
    {shown && createPortal(<AircraftPanel id={id} onClose={close}>
      <AircraftCatalogueView pages={pages} version={version} origin={origin} onOrigin={setOrigin} query={query} onQuery={setQuery}
        chosen={chosen} status={status} onChoose={onChoose} request={request} retry={retry} />
    </AircraftPanel>, host)}
  </>;
}
