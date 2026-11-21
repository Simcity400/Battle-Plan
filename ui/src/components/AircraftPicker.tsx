import { failureText } from '../dashboard/queryError.ts';
import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';
import type { AircraftEntry, AircraftScope, FlyAircraftStatus } from '../generated/dashboard';
import type { DashboardClient, DashboardSource } from '../dashboard/client';
import { CATALOGUE_NOTE, aircraftRowInteractive, refusalLine, refusalText, useAircraftCatalogue } from '../dashboard/aircraftCatalogue';
import { aircraftTitle } from '../dashboard/aircraftChoice';
import type { AircraftChoice } from '../dashboard/aircraftChoice';
import type { AircraftPages } from '../dashboard/aircraftCatalogue';
import { Icon } from './Icons';
import { FailureLine } from './FailureLine';
import { Segmented } from './Segmented';
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

function kindBadge(entry: AircraftEntry): string | null {
  switch (entry.kind) {
    case 'mesh': return 'No animation';
    case 'unregistered': case 'entity': return null;
  }
}

function Row({ index, pages, chosen, onChoose, request, retry }: {
  index: number; pages: AircraftPages; chosen: AircraftChoice | null; onChoose: (aircraft: AircraftChoice) => void;
  request: (first: number, last: number) => void; retry: (index: number) => void;
}) {
  const entry = pages.entry(index);
  const error = pages.error(Math.floor(index / pages.size));
  useEffect(() => { if (!entry && !error) request(index, index); }, [entry, error, index, request]);
  if (!entry) {
    if (error) return <FailureLine className="aircraft-row is-failed" detail={error.message} message={failureText(error, 'Couldn’t load these rows.')} onRetry={() => retry(index)} />;
    return <div className="aircraft-row is-loading" aria-hidden="true"><span className="skeleton" /></div>;
  }
  const refused = entry.refusal;
  const badge = kindBadge(entry);
  const title = aircraftTitle(entry);
  return <button type="button" className="aircraft-row" disabled={!!refused} aria-pressed={entry.name === chosen?.name}
    onClick={() => onChoose({ name: entry.name, title })}>
    <span className="aircraft-row-title">{title}{badge && <small className="aircraft-badge">{badge}</small>}</span>
    {refused && <small className="aircraft-row-refusal" data-detail={refused.reason}>{refusalText(refused.kind)}</small>}
  </button>;
}

export function AircraftCatalogueView({ pages, version, scope, onScope, query, onQuery, chosen, status, onChoose, request, retry }: {
  pages: AircraftPages | null; version: number; scope: AircraftScope; onScope: (scope: AircraftScope) => void; query: string; onQuery: (query: string) => void;
  chosen: AircraftChoice | null; status: FlyAircraftStatus | null; onChoose: (aircraft: AircraftChoice | null) => void;
  request: (first: number, last: number) => void; retry: (index: number) => void;
}) {
  const scroll = useRef<HTMLDivElement>(null);
  const matching = pages?.matching ?? 0;
  const rows = useMemo(() => Array.from({ length: matching }, (_, index) => index), [matching]);
  const interactive = useCallback((index: number) => !!pages && aircraftRowInteractive(pages, index), [pages, version]);
  const line = statusText(chosen, status);
  const refusals = refusalLine(pages?.refusals ?? []);
  const failed = pages?.firstError();
  const scopes = [
    { value: 'aircraft' as const, label: pages?.aircraft == null ? 'Aircraft' : `Aircraft (${pages.aircraft.toLocaleString()})`, tooltip: 'Aircraft the game uses' },
    { value: 'all' as const, label: pages?.all == null ? 'All models' : `All models (${pages.all.toLocaleString()})`, tooltip: 'Every 3D model, including ones no aircraft uses' },
  ];
  return <>
    <div className="aircraft-picker-controls">
      <input type="search" autoFocus placeholder="Search names, equipment and countries" aria-label="Search aircraft"
        value={query} onChange={event => onQuery(event.currentTarget.value)} />
      <Segmented label="Listed models" value={scope} onChange={onScope} options={scopes} />
      <button type="button" className="map-menu-action" aria-pressed={chosen === null} onClick={() => onChoose(null)}>Choose none</button>
    </div>
    {line && (line.alert ? <FailureLine className="aircraft-picker-failure" message={line.text} detail={line.detail} />
      : <p className="aircraft-picker-status" role="status">{line.text}</p>)}
    <div className="aircraft-picker-list" ref={scroll}>
      {pages && pages.matching === null && !failed && <p className="aircraft-picker-note" role="status">Reading aircraft…</p>}
      {pages && pages.matching === null && failed && <FailureLine className="aircraft-picker-failure" detail={failed.message} message={failureText(failed, 'Couldn’t load aircraft.')} onRetry={() => retry(0)} />}
      {pages?.matching === 0 && <p className="aircraft-picker-note">{query.trim() ? 'No model matches the search.' : 'There are no models to list.'}</p>}
      {pages && rows.length > 0 && <VirtualList identity={pages} items={rows} itemKey={index => String(index)} estimate={52} label="Aircraft models" scrollRef={scroll} isInteractive={interactive}>
        {index => <Row index={index} pages={pages} chosen={chosen} onChoose={onChoose} request={request} retry={retry} />}
      </VirtualList>}
    </div>
    {refusals && <p className="aircraft-picker-note" tabIndex={0} data-tooltip={refusals.tooltip} data-detail={refusals.detail}>{refusals.text}</p>}
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
  const [scope, setScope] = useState<AircraftScope>('aircraft');
  const [query, setQuery] = useState('');
  const shown = open && !!host && !!source;
  const { pages, version, request, retry } = useAircraftCatalogue(client, source, scope, query, shown);
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
      <AircraftCatalogueView pages={pages} version={version} scope={scope} onScope={setScope} query={query} onQuery={setQuery}
        chosen={chosen} status={status} onChoose={onChoose} request={request} retry={retry} />
    </AircraftPanel>, host)}
  </>;
}
