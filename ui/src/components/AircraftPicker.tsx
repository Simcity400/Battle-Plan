import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';
import type { AircraftEntry, AircraftScope, FlyAircraftStatus } from '../generated/dashboard';
import type { DashboardClient, DashboardSource } from '../dashboard/client';
import { aircraftRowInteractive, referenceText, refusalLine, useAircraftCatalogue } from '../dashboard/aircraftCatalogue';
import type { AircraftPages } from '../dashboard/aircraftCatalogue';
import { Icon } from './Icons';
import { Segmented } from './Segmented';
import { VirtualList } from './VirtualList';
import './aircraftPicker.css';

/** References shown on a row before the rest go to its tooltip. */
const SHOWN_REFERENCES = 2;

/** What the panel says while no aircraft is chosen: there is no default. */
export const NO_AIRCRAFT = 'No aircraft is chosen: Fly mode is a free camera until you choose one.';

/** What the chosen aircraft's status line says: that none is chosen, or what the map last answered for the one chosen; nothing while the map has not answered for it yet. */
export function statusText(chosen: string | null, status: FlyAircraftStatus | null): { text: string; alert: boolean } | null {
  if (!chosen) return { text: NO_AIRCRAFT, alert: false };
  if (!status || status.aircraft !== chosen) return null;
  switch (status.state.kind) {
    case 'loading': return { text: `Loading ${chosen}...`, alert: false };
    case 'ready': return { text: `Flying ${chosen}`, alert: false };
    case 'refused': return { text: `${chosen} cannot be flown: ${status.state.reason}`, alert: true };
  }
}

function kindBadge(entry: AircraftEntry): string | null {
  switch (entry.kind) {
    case 'mesh': return 'Mesh (no animation)';
    case 'unregistered': return 'Not registered';
    case 'entity': return null;
  }
}

/** One catalogue row: an entry once its page arrives, a placeholder asking for its page until then. */
function Row({ index, pages, chosen, onChoose, request, retry }: {
  index: number; pages: AircraftPages; chosen: string | null; onChoose: (name: string) => void;
  request: (first: number, last: number) => void; retry: (index: number) => void;
}) {
  const entry = pages.entry(index);
  const error = pages.error(Math.floor(index / pages.size));
  useEffect(() => { if (!entry && !error) request(index, index); }, [entry, error, index, request]);
  if (!entry) {
    if (error) return <div className="aircraft-row is-failed" role="alert">
      <span>Rows from {(Math.floor(index / pages.size) * pages.size + 1).toLocaleString()} did not load: {error}</span>
      <button type="button" className="map-menu-action" onClick={() => retry(index)}>Retry</button>
    </div>;
    return <div className="aircraft-row is-loading" aria-hidden="true"><span className="skeleton" /></div>;
  }
  const refused = entry.refusal;
  const references = entry.references.map(referenceText);
  const badge = kindBadge(entry);
  return <button type="button" className="aircraft-row" disabled={!!refused} aria-pressed={entry.name === chosen}
    data-tooltip={refused ? `Cannot be flown: ${refused.reason}` : references.length > SHOWN_REFERENCES ? references.join('\n') : undefined}
    onClick={() => onChoose(entry.name)}>
    <span className="aircraft-row-title">{entry.titles.length ? entry.titles.join(' / ') : <code>{entry.name}</code>}{badge && <small className="aircraft-badge">{badge}</small>}</span>
    {entry.titles.length > 0 && <code className="aircraft-row-name">{entry.name}</code>}
    {refused ? <small className="aircraft-row-refusal">{refused.reason}</small>
      : references.length > 0 && <small className="aircraft-row-references">{references.slice(0, SHOWN_REFERENCES).join('; ')}
        {references.length > SHOWN_REFERENCES && ` +${(references.length - SHOWN_REFERENCES).toLocaleString()} more`}</small>}
  </button>;
}

/** The catalogue a picker shows: search, scope, the chosen aircraft's status, every matching entry a page at a time, the counted refusals and the content's notes. */
export function AircraftCatalogueView({ pages, version, scope, onScope, query, onQuery, chosen, status, onChoose, request, retry }: {
  pages: AircraftPages | null; version: number; scope: AircraftScope; onScope: (scope: AircraftScope) => void; query: string; onQuery: (query: string) => void;
  chosen: string | null; status: FlyAircraftStatus | null; onChoose: (aircraft: string | null) => void;
  request: (first: number, last: number) => void; retry: (index: number) => void;
}) {
  const scroll = useRef<HTMLDivElement>(null);
  const matching = pages?.matching ?? 0;
  const rows = useMemo(() => Array.from({ length: matching }, (_, index) => index), [matching]);
  // Refused rows keep their places but are no keyboard destinations; a new predicate each time
  // pages arrive, as eligibility changes with them.
  const interactive = useCallback((index: number) => !!pages && aircraftRowInteractive(pages, index), [pages, version]); // eslint-disable-line react-hooks/exhaustive-deps
  const line = statusText(chosen, status);
  const refusals = refusalLine(pages?.refusals ?? []);
  const failed = pages?.firstError();
  const scopes = [
    { value: 'aircraft' as const, label: pages?.aircraft == null ? 'Aircraft' : `Aircraft (${pages.aircraft.toLocaleString()})`, tooltip: 'Models the content makes an aircraft: graphics pools, model names, saved designs and their clones' },
    { value: 'all' as const, label: pages?.all == null ? 'All models' : `All models (${pages.all.toLocaleString()})`, tooltip: 'Every entity and mesh the content registers, for planes nothing references' },
  ];
  return <>
    <div className="aircraft-picker-controls">
      <input type="search" autoFocus placeholder="Search names, equipment and countries" aria-label="Search aircraft"
        value={query} onChange={event => onQuery(event.currentTarget.value)} />
      <Segmented label="Listed models" value={scope} onChange={onScope} options={scopes} />
      <button type="button" className="map-menu-action" aria-pressed={chosen === null} onClick={() => onChoose(null)}>Choose none</button>
    </div>
    {line && <p className={line.alert ? 'aircraft-picker-status is-refused' : 'aircraft-picker-status'} role={line.alert ? 'alert' : 'status'}>{line.text}</p>}
    <div className="aircraft-picker-list" ref={scroll}>
      {pages && pages.matching === null && !failed && <p className="aircraft-picker-note" role="status">Reading the content's aircraft...</p>}
      {pages && pages.matching === null && failed && <p className="aircraft-picker-note" role="alert">{failed}
        <button type="button" className="map-menu-action" onClick={() => retry(0)}>Retry</button></p>}
      {pages?.matching === 0 && <p className="aircraft-picker-note">{query.trim() ? 'No model matches the search.' : 'The content has no models here.'}</p>}
      {pages && rows.length > 0 && <VirtualList identity={pages} items={rows} itemKey={index => String(index)} estimate={52} label="Aircraft models" scrollRef={scroll} isInteractive={interactive}>
        {index => <Row index={index} pages={pages} chosen={chosen} onChoose={onChoose} request={request} retry={retry} />}
      </VirtualList>}
    </div>
    {refusals && <p className="aircraft-picker-note">{refusals}</p>}
    {pages && pages.warnings.length > 0 && <details className="aircraft-picker-warnings"><summary>Content notes ({pages.warnings.length.toLocaleString()})</summary>
      <ul>{pages.warnings.map(warning => <li key={warning}>{warning}</li>)}</ul></details>}
  </>;
}

/** The docked aircraft list. It is a page region, not a popover, so the map-overlay guard (which
 * hides the native map behind any open popover) leaves the map shown; the shell gives it its own
 * column beside the map, which narrows to make room, so the chosen plane is seen while trying
 * others. Escape or its close button closes it. */
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

/** Fly mode's aircraft: any aircraft model or entity of the recording's content stack, or none. There is no default. The list docks in `host`, beside the map; choosing an aircraft shows it on the map, which is its preview, and keeps the list open to try another. With `prompt`, the list opens by itself while nothing is chosen, as Fly mode starts. */
export default function AircraftPicker({ client, source, chosen, status, onChoose, host, prompt = false }: {
  client: DashboardClient; source: DashboardSource | null; chosen: string | null; status: FlyAircraftStatus | null;
  onChoose: (aircraft: string | null) => void; host: HTMLElement | null; prompt?: boolean;
}) {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const [scope, setScope] = useState<AircraftScope>('aircraft');
  const [query, setQuery] = useState('');
  const shown = open && !!host && !!source;
  const { pages, version, request, retry } = useAircraftCatalogue(client, source, scope, query, shown);
  // Opened once for each campaign Fly mode starts on with nothing chosen; closing it keeps it closed.
  useEffect(() => { if (prompt && chosen === null && source) setOpen(true); }, [prompt, source?.id]); // eslint-disable-line react-hooks/exhaustive-deps
  const close = useCallback(() => { setOpen(false); trigger.current?.focus({ preventScroll: true }); }, []);
  const line = statusText(chosen, status);
  if (!source) return <button type="button" className="icon-button aircraft-picker-trigger" disabled data-tooltip="Open a campaign to choose an aircraft"><Icon name="plane" /><span>No aircraft chosen</span></button>;
  return <>
    <button ref={trigger} type="button" className={chosen ? 'icon-button aircraft-picker-trigger' : 'icon-button aircraft-picker-trigger is-unchosen'}
      aria-expanded={shown} aria-controls={shown ? id : undefined} disabled={!host}
      data-tooltip={line?.text ?? 'Choose the aircraft to fly'} onClick={() => setOpen(value => !value)}><Icon name="plane" /><span>{chosen ?? 'No aircraft chosen'}</span></button>
    {shown && createPortal(<AircraftPanel id={id} onClose={close}>
      <AircraftCatalogueView pages={pages} version={version} scope={scope} onScope={setScope} query={query} onQuery={setQuery}
        chosen={chosen} status={status} onChoose={onChoose} request={request} retry={retry} />
    </AircraftPanel>, host)}
  </>;
}
