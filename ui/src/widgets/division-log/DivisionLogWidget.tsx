import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import { useCallback, useMemo, useRef } from 'react';
import ArtFallback from '../../components/ArtFallback';
import { ChipSet } from '../../components/ChipSet';
import ContentSetPicker from '../../components/ContentSetPicker';
import { CountryFlagUrl } from '../../components/CountryFlag';
import WidgetState from '../../components/WidgetState';
import LiveText, { liveTexts } from '../../charts/LiveText';
import LogList from '../../charts/log/LogList';
import { AHEAD_ROWS, useRevealClock } from '../../charts/reveal';
import LogGaps from '../../charts/log/LogGaps';
import { logCountryRequests } from '../../charts/log/logData';
import type { LogRow } from '../../charts/log/logData';
import { logSettings } from '../../charts/log/logOptions';
import { countsAhead, validateLogCounts } from '../../charts/log/logSource';
import type { LogCounts } from '../../charts/log/logSource';
import { jumpItem } from '../../charts/jump';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatchFilter } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { contentSet, resolveOptions, toggleContentSet } from '../../dashboard/options';
import { placeKey, validatePlaceNamesReply } from '../../dashboard/placeNamesData';
import type { DivisionLogManifestReply, PlaceName, PlaceNamesReply } from '../../generated/dashboard';
import { useContentRevision } from '../../dashboard/contentRevision';
import { loadProduct, useProduct, useProducts } from '../../dashboard/products';
import { useKeyedPainter, useKeyedReads } from '../../dashboard/keyedReads';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { countBinHours, useLogPages } from '../../dashboard/useLogPages';
import type { LogCut, LogSpec } from '../../dashboard/useLogPages';
import { OPTIONS } from './definition';
import { divisionEntry, divisionFacets, divisionName, divisionNamedRows, divisionPlace, divisionStatus, divisionTooltip, gapWarnings, kindTotals, KINDS,
  selectedTemplates, shownKinds, templateToken, divisionPlaces, divisionPlaceText, PLACE_LOADING, validateDivisionManifest } from './divisionLogData.ts';
import type { DivisionKind, DivisionLogEntry } from './divisionLogData';
import './divisionLog.css';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { trackHourOf } from '../../dashboard/settledHour';
import { useDistinctState } from '../../dashboard/useDistinctState';
import { productRequest } from '../../dashboard/request.ts';

const NO_ROWS: readonly LogRow<DivisionLogEntry>[] = [];
const sameKeys = (held: readonly LogRow<DivisionLogEntry>[], next: readonly LogRow<DivisionLogEntry>[]) => held.length === next.length
  && held.every((row, index) => row.key === next[index].key);
const VANISHED = 3;

export default function DivisionLogWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const gate = useGate(runtime, widget, SPEC, active);
useShownEachFrame(runtime.clock, () => track ? trackHourOf(widget, track, runtime.tracks, runtime.clock.getHour()) : null);
  const options = resolveOptions(OPTIONS, widget.options);
  const log = logSettings(options);
  const follow = widget.time.mode === 'follow';
  const window = track && !follow ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour()) : null;
  const first = widget.time.mode === 'range' && window ? Math.floor(window.firstHour) : track?.branch.firstHour ?? 0;
  const last = track?.branch.lastHour ?? 0;
  const cut: LogCut = follow ? { follow: true, offset: track?.axisOffset ?? 0 }
    : { follow: false, hour: widget.time.mode === 'campaign' || !window ? null : Math.floor(window.lastHour) };
  const manifest = useProduct<DivisionLogManifestReply>(runtime.client, track ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'division-log-manifest', params: { branchId: track.branch.id } }) : null, active, validateDivisionManifest);
  const campaign = track?.source.metadata.campaignKey ?? '';
  const branch = track?.branch.id ?? '';
  const templatesValue = typeof options.templates === 'string' ? options.templates : '';
  const templates = useMemo(() => selectedTemplates(templatesValue, campaign, branch), [templatesValue, campaign, branch]);
  const chosen = useMemo(() => contentSet(templatesValue), [templatesValue]);
  const templateChoices = useMemo(() => (manifest.data?.templates ?? []).map(template => ({
    value: templateToken(campaign, branch, template.id), label: template.name ?? `Unnamed template (${template.id})`,
  })), [manifest.data, campaign, branch]);
  const events = String(options.events);
  const groups = useCountryGroups(runtime, track, active && (widget.countries.kind === 'group' || widget.countries.kind === 'factions'));
  const tags = useMemo<string[] | null>(() => {
    if (widget.countries.kind === 'all') return null;
    if (widget.countries.kind !== 'group' && widget.countries.kind !== 'factions') return selectedCountryKeys(widget.countries, track);
    if (!groups.data) return null;
    const filter = countryMatchFilter(widget.countries, [], groups.data, last, undefined, track?.branch.firstHour);
    return [...new Set(filter.membership?.map(run => run.tag) ?? [])].sort();
  }, [widget.countries, track, groups.data, last]);
  const waiting = tags === null && widget.countries.kind !== 'all';
  const kinds = useMemo(() => shownKinds(events), [events]);
  const losses = manifest.data?.lossesAvailable === true;
  const facets = useMemo(() => divisionFacets(losses ? kinds : kinds.filter(kind => kind === 'created'), templates, tags), [kinds, losses, templates, tags]);
  const logSpan = gate.span(first, last);
  const spec = useMemo<LogSpec | null>(() => track && logSpan && manifest.data?.available && !waiting && facets[0].values.length
    ? { source: track.source, branchId: track.branch.id, log: 'division-events', product: 'division-log', facets, ...logSpan } : null,
  [track, manifest.data?.available, waiting, facets, logSpan?.firstHour, logSpan?.lastHour]);
  const list = useLogPages<DivisionLogEntry>(runtime.client, runtime.clock, spec, cut, active, divisionEntry);
  const counted = useMemo(() => [...(options.totals === true ? facets[0].values : []), VANISHED], [options.totals, facets]);
  const countRequests = useMemo(() => spec ? counted.map(kind => (productRequest({ sourceId: spec.source.id, revision: spec.source.revision, product: 'log-counts',
    params: { branchId: spec.branchId, log: spec.log, facets: [{ name: 'kind', values: [kind], countries: [] }, ...facets.slice(1)],
      from: spec.firstHour, to: spec.lastHour, binHours: countBinHours(first, last) } }))) : [], [spec, counted, facets, first, last]);
  const validateCounts = useCallback((value: unknown) => validateLogCounts(value, first, last), [first, last]);
  const kindCounts = useProducts<LogCounts>(runtime.client, countRequests, active, validateCounts, track?.source);
  const until = cut.follow ? Math.floor(runtime.clock.getHour()) + cut.offset : cut.hour;
  const offset = cut.follow ? cut.offset : 0;
  const trackHour = useCallback((clockHour: number) => Math.floor(clockHour) + offset, [offset]);
  const reveal = useRevealClock(runtime.clock, cut.follow, trackHour);
  const totals = useMemo(() => {
    if (!list.counts || !kindCounts.data) return null;
    const byKind = new Map<DivisionKind, LogCounts>();
    counted.forEach((kind, index) => { if (kind !== VANISHED) byKind.set((Object.keys(KINDS) as DivisionKind[])[kind], kindCounts.data![index]); });
    return kindTotals(list.counts, byKind, list.pages, list.shown, list.at);
  }, [list.counts, list.pages, list.shown, list.at, kindCounts.data, counted]);
  const liveTotals = useMemo(() => totals ? countsAhead(totals, list.ahead, row => row.item.kind) : null, [totals, list.ahead]);
  const totalTexts = useMemo(() => liveTexts(runtime.clock, () => follow ? trackHour(runtime.clock.getHour()) : 0, hour => {
    const at = liveTotals?.(hour);
    return new Map((Object.keys(KINDS) as DivisionKind[]).map(kind => [kind, at?.get(kind)?.toLocaleString() ?? 'Loading…']));
  }), [runtime.clock, follow, trackHour, liveTotals]);
  const vanished = useMemo(() => {
    const counts = kindCounts.data?.[counted.length - 1];
    if (!counts) return 0;
    const bin = until === null ? counts.counts.length : Math.floor((until - counts.firstHour) / counts.binHours);
    return counts.counts.slice(0, Math.max(0, bin)).reduce((sum, count) => sum + count, 0);
  }, [kindCounts.data, counted.length, until]);

  const newest = list.ahead.length + 2 * AHEAD_ROWS;
  const [visible, setVisible] = useDistinctState<readonly LogRow<DivisionLogEntry>[]>(NO_ROWS, sameKeys);
  const named = useMemo(() => divisionNamedRows(follow, list.pages, newest, visible), [follow, list.pages, newest, visible]);
  const namedKeys = useMemo(() => follow ? new Set(named.map(row => row.key)) : null, [follow, named]);
  const onVisible = useCallback((rows: readonly LogRow<DivisionLogEntry>[]) => {
    if (namedKeys && rows.every(row => namedKeys.has(row.key))) return;
    setVisible(rows);
  }, [namedKeys, setVisible]);
  const wanted = useMemo(() => [...logCountryRequests(named, row => ({ hour: row.item.readHour, tags: [row.item.tag, row.item.killerTag] }))], [named]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active, Math.max(MAX_NAME_HOURS, new Set(wanted.map(country => country.hour)).size));
  const content = useContentRevision(runtime.client);
  const placesOwner = track ? JSON.stringify([track.source.id, track.source.revision, branch, content]) : null;
  const placesAt = follow ? list.at : null;
  const places = useMemo(() => divisionPlaces(named, placesAt), [named, placesAt]);
  const readPlaces = useCallback((asked: readonly { provinceId: number; hour: number }[], signal: AbortSignal) => loadProduct<PlaceNamesReply>(runtime.client,
    productRequest({ sourceId: track!.source.id, revision: track!.source.revision, product: 'place-names', params: { branchId: branch,
      places: asked.flatMap(place => { const hour = gate.recorded(place.hour); return hour === null ? [] : [{ ...place, hour }]; }), states: [] } }),
    signal, validatePlaceNamesReply, track?.source).then(reply => ({ values: reply.places.map(place => [placeKey(place.provinceId, place.hour), place] as const), warnings: reply.warnings })),
  [runtime.client, track, branch]);
  const placeReads = useKeyedReads<{ provinceId: number; hour: number }, PlaceName>(placesOwner, places, active, place => placeKey(place.provinceId, place.hour), readPlaces);
  const root = useRef<HTMLDivElement>(null);
  useKeyedPainter(placeReads.reads as never, root, 'place', key => {
    const reads = placeReads.reads, text = divisionPlaceText(reads.values.get(key), { pending: reads.pendingKey(key), failed: reads.failures.has(key) });
    return { text, loading: text === PLACE_LOADING };
  });
  const knownPlaces = placeReads.reads.values;
  const labels = useCallback((row: DivisionLogEntry) => {
    const ask = row.province === null || row.placeHour === null ? null : { provinceId: row.province, hour: row.placeHour };
    return {
      country: row.tag === null ? { name: 'Country not known' } : labelAt(row.readHour)(row.tag),
      killer: row.killerTag === null ? { name: 'Killer not known' } : labelAt(row.readHour)(row.killerTag),
      place: divisionPlace(row, knownPlaces, { pending: ask !== null && placeReads.reads.pending(ask), failed: ask !== null && placeReads.reads.failure(ask) !== undefined }),
    };
  }, [labelAt, knownPlaces, placeReads.reads, placeReads.version]);
  const warnings = useMemo(() => [...new Set([...(manifest.data?.warnings ?? []),
    ...(manifest.data ? gapWarnings(manifest.data, first, until ?? last) : []),
    ...list.pages.flatMap(page => page.error ? [`Division log: ${page.error.message}`] : []),
    ...(vanished ? [`Division left the census without a removal witness; no loss inferred (${vanished})`] : []),
    ...placeReads.reads.warnings, ...[...placeReads.reads.failures.values()].map(error => `Place names: ${error.message}`)])],
  [manifest.data, first, until, last, list.pages, vanished, placeReads.reads, placeReads.version]);
  useDiagnostics(onDiagnostics, warnings);

  if (!track) return <WidgetState kind="empty" icon="shield" message="Load a campaign to see its divisions." />;
  if (manifest.error) return <WidgetState kind="error" failure={manifest.error} onRetry={manifest.retry} />;
  if (widget.countries.kind !== 'all' && groups.error && !groups.data) return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
  if (!manifest.data || waiting) return <WidgetState kind="loading" label="Loading division log…" />;
  if (!manifest.data.available || events === 'lost' && !losses) return <WidgetState kind="unavailable" message="Not in this recording" />;
  if (list.error) return <WidgetState kind="error" failure={list.error} onRetry={list.retry} />;
  const total = (kind: DivisionKind) => <LiveText live={totalTexts} at={kind} />;
  const renderRow = ({ item: row }: LogRow<DivisionLogEntry>) => {
    const { country, killer, place } = labels(row);
    return <><span className="log-art" aria-hidden={row.icon ? true : undefined}>{row.icon ? <img src={row.icon} alt="" /> : <ArtFallback icon="helmet" name={row.templateName ?? divisionName(row)} reason={row.iconMissing} />}</span>
      <span className="log-text">{row.tag !== null && <CountryFlagUrl className="log-flag" url={country.flag} detail={country.flagDetail} reason={country.flagReason} pending={country.flagPending} />}<b>{country.name}</b>: <b>{divisionName(row)}</b>
        {' · '}{row.templateName ?? 'Template not recorded'}{' · '}{row.placeAtEvent ? 'at ' : 'last seen at '}{row.province === null || row.placeHour === null ? place
          : <span className={place === PLACE_LOADING ? 'is-loading' : undefined} data-keyed="place" data-keyed-key={placeKey(row.province, row.placeHour)}>{place}</span>}
        {row.killerTag !== null && <>{' · by '}<CountryFlagUrl className="log-flag" url={killer.flag} detail={killer.flagDetail} reason={killer.flagReason} pending={killer.flagPending} />{killer.name}</>}
      </span><span className="log-tag">{divisionStatus(row)}</span></>;
  };
  return <div ref={root} className="log-widget division-log">
    {options.totals === true && <div className="division-log-summary" aria-live="polite">
      {kindCounts.error ? <WidgetState kind="error" what="division totals" failure={kindCounts.error} onRetry={kindCounts.retry} /> : <>
        {events !== 'lost' && <span><b>{total('created')}</b> created</span>}
        {events !== 'created' && <><span><b>{losses ? total('lost') : 'Not recorded'}</b> lost / removed</span>
          <span><b>{losses ? total('transfer') : 'Not recorded'}</b> transferred / merged</span></>}
      </>}
    </div>}
    {!losses && events !== 'created' && <p className="division-log-note">Losses and transfers: Not in this recording.</p>}
    {log.chips && <div className="log-chips"><ChipSet label="Division events" chips={[{ value: 'both', label: 'Both' }, { value: 'created', label: 'Created' }, { value: 'lost', label: 'Lost / transferred' }]}
      selected={[events]} max={1} full={false} onToggle={value => setOption?.('events', value)} />
      {chosen.length > 0 && <ContentSetPicker label="Division template types" automatic="All templates" choices={templateChoices} selected={chosen}
        onReset={() => setOption?.('templates', '')} onToggle={value => {
          const next = toggleContentSet(templatesValue, value, Number.MAX_SAFE_INTEGER); if (next !== null) setOption?.('templates', next);
        }} />}</div>}
    <LogGaps missing={list.missing} until={until} subject="Division events are not recorded whole" />
    <LogList pages={list.pages} onWantedPages={list.onWantedPages} label="Division log" density={log.density} months={log.months}
      animate={log.animate && follow} playhead={cut.follow ? until : null} reveal={reveal}
      columns="var(--space-6) minmax(0, 1fr) auto" renderRow={renderRow} onVisible={onVisible}
      describe={({ item: row }) => { const named = labels(row); return `${named.country.name}: ${divisionName(row)}, ${divisionStatus(row)}, ${row.templateName ?? 'template not recorded'}, ${named.place}, ${named.killer.name}.`; }}
      tooltip={({ item: row }) => { const named = labels(row); return [divisionTooltip(row, named.country, named.killer, named.place)]; }}
      menu={({ item: row }) => [jumpItem(runtime, track.id, row.readHour)]}
      empty={list.counts ? 'No recorded division events match these settings.' : <WidgetState kind="loading" label="Loading division log…" />} />
  </div>;
}
