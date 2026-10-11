import { useCallback, useEffect, useMemo, useState } from 'react';
import ArtFallback from '../../components/ArtFallback';
import { ChipSet } from '../../components/ChipSet';
import ContentSetPicker from '../../components/ContentSetPicker';
import { CountryFlagUrl } from '../../components/CountryFlag';
import WidgetState from '../../components/WidgetState';
import LogList from '../../charts/log/LogList';
import { logCountryRequests } from '../../charts/log/logData';
import type { LogRow } from '../../charts/log/logData';
import { logSettings } from '../../charts/log/logOptions';
import { useTableHour } from '../../charts/table/useTableHour';
import { jumpItem } from '../../charts/jump';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch, countryMatchFilter } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { contentSet, resolveOptions, toggleContentSet } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { useProductWindows } from '../../dashboard/productWindows';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { DivisionLogManifestReply, DivisionLogReply, DivisionLogTotalsReply, PlaceNamesReply } from '../../generated/dashboard';
import { battleWindowIndex, battleWindows } from '../land-battles/landBattlesData';
import { placeKey, validatePlaceNamesReply } from '../../dashboard/placeNamesData';
import { OPTIONS } from './definition';
import { divisionName, divisionRows, divisionStatus, divisionTooltip, selectedTemplates, templateToken,
  validateDivisionLog, validateDivisionManifest, validateDivisionTotals } from './divisionLogData';
import type { DivisionLogItem } from './divisionLogData';
import './divisionLog.css';

const NO_ROWS: readonly LogRow<DivisionLogItem>[] = [];
const NO_WARNINGS: string[] = [];

export default function DivisionLogWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const live = widget.time.mode === 'follow';
  const playback = useTableHour(runtime.clock, active && live);
  const contentRevision = useContentRevision(runtime.client);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const log = logSettings(options);
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const hour = window ? Math.floor(window.lastHour) : null;
  const first = widget.time.mode === 'range' && window ? Math.floor(window.firstHour) : track?.branch.firstHour ?? 0;
  const groups = useCountryGroups(runtime, track, active && widget.countries.kind !== 'all');
  const fallbackKey = JSON.stringify(selectedTags(widget.countries, track));
  const fallback = useMemo<string[]>(() => JSON.parse(fallbackKey), [fallbackKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const manifest = useProduct<DivisionLogManifestReply>(runtime.client, track ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'division-log-manifest', params: { branchId: track.branch.id } } : null, active, validateDivisionManifest);
  const identity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, contentRevision]) : null;
  const playheadWindow = battleWindowIndex(manifest.data?.windows ?? [], hour ?? first);
  const [browse, setBrowse] = useState<{ identity: string | null; hour: number | null }>({ identity: null, hour: null });
  const shownHour = browse.identity === identity && browse.hour !== null ? Math.min(browse.hour, hour ?? browse.hour) : hour;
  const browseIndex = battleWindowIndex(manifest.data?.windows ?? [], shownHour ?? first);
  const [visibleWindows, setVisibleWindows] = useState<{ identity: string | null; at: number; windows: number[] }>({ identity: null, at: -1, windows: [] });
  const visibleIndices = visibleWindows.identity === identity && visibleWindows.at === playheadWindow ? visibleWindows.windows : [];
  const wantedWindows = battleWindows(manifest.data, hour ?? first, live, [...visibleIndices, ...(browse.identity === identity && browse.hour !== null ? [browseIndex] : [])], first);
  const history = useProductWindows<DivisionLogReply>(runtime.client, identity, index => ({ sourceId: track!.source.id, revision: track!.source.revision,
    product: 'division-log', params: { branchId: track!.branch.id, window: index } }), wantedWindows,
  active && manifest.data?.available === true, validateDivisionLog);
  const templatesValue = typeof options.templates === 'string' ? options.templates : '';
  const campaign = track?.source.metadata.campaignKey ?? '';
  const branch = track?.branch.id ?? '';
  const templates = useMemo(() => selectedTemplates(templatesValue, campaign, branch), [templatesValue, campaign, branch]);
  const chosen = useMemo(() => contentSet(templatesValue), [templatesValue]);
  const templateChoices = useMemo(() => (manifest.data?.templates ?? []).map(template => ({
    value: templateToken(campaign, branch, template.id), label: template.name ?? `Unnamed template (${template.id})`,
  })), [manifest.data, campaign, branch]);
  const events = String(options.events);
  const rows = useMemo(() => shownHour === null || !match.ready ? NO_ROWS : divisionRows(history.data, shownHour, first, events,
    templates, match.matches, widget.countries.kind === 'all'), [history.data, shownHour, first, events, templates, match, widget.countries.kind]);
  const countries = useMemo(() => countryMatchFilter(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0,
    undefined, track?.branch.firstHour), [widget.countries, fallback, groups.data, track?.branch.lastHour, track?.branch.firstHour]);
  const totals = useProduct<DivisionLogTotalsReply>(runtime.client, track && hour !== null ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'division-log-totals', params: { branchId: branch, firstHour: first, lastHour: hour, countries, templates } } : null,
  active && options.totals === true && match.ready && !match.reason && manifest.data?.available === true, validateDivisionTotals);
  const [visible, setVisible] = useState<readonly LogRow<DivisionLogItem>[]>(NO_ROWS);
  const currentRows = useMemo(() => new Map(rows.map(row => [row.key, row])), [rows]);
  const drawn = useMemo(() => visible.flatMap(row => { const current = currentRows.get(row.key); return current ? [current] : []; }), [visible, currentRows]);
  const wanted = useMemo(() => [...logCountryRequests(drawn, row => ({ hour: row.item.countryHour, tags: [row.item.tag] })),
    ...logCountryRequests(drawn, row => ({ hour: row.item.killerHour ?? row.item.readHour, tags: [row.item.killerTag] }))], [drawn]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active, Math.max(MAX_NAME_HOURS, new Set(wanted.map(country => country.hour)).size));
  const places = useMemo(() => [...new Map(drawn.flatMap(row => row.item.province === null || row.item.placeHour === null ? []
    : [[placeKey(row.item.province, row.item.placeHour), { provinceId: row.item.province, hour: row.item.placeHour }] as const])).values()], [drawn]);
  const placeData = useProduct<PlaceNamesReply>(runtime.client, track && places.length ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'place-names', params: { branchId: branch, places, states: [] } } : null, active, validatePlaceNamesReply);
  const namedPlaces = useMemo(() => new Map(placeData.data?.places.map(place => [placeKey(place.provinceId, place.hour), place])), [placeData.data]);
  const labels = useCallback((row: DivisionLogItem) => {
    const killer = row.killerTag === null ? { name: 'Killer not known' } : labelAt(row.killerHour ?? row.readHour)(row.killerTag);
    return {
      country: row.tag === null ? { name: 'Country not known' } : labelAt(row.countryHour)(row.tag),
      killer: row.killerHour === null ? { ...killer, name: 'Killer not known' } : killer,
      place: row.province === null || row.placeHour === null ? 'Location not recorded'
        : namedPlaces.get(placeKey(row.province, row.placeHour))?.name ?? 'Unnamed province',
    };
  }, [labelAt, namedPlaces]);
  const onVisible = useCallback((drawn: readonly LogRow<DivisionLogItem>[]) => {
    setVisible(previous => previous.length === drawn.length && previous.every((row, index) => row === drawn[index]) ? previous : drawn);
    const windows = [...new Set(drawn.map(row => row.item.windowIndex))].sort((a, b) => a - b);
    setVisibleWindows(previous => previous.identity === identity && previous.at === playheadWindow && JSON.stringify(previous.windows) === JSON.stringify(windows)
      ? previous : { identity, at: playheadWindow, windows });
  }, [identity, playheadWindow]);
  const warnings = useMemo(() => [...new Set([...(manifest.data?.contentWarnings ?? NO_WARNINGS),
    ...[...history.data.values()].flatMap(reply => [...reply.contentWarnings, ...reply.warnings.filter(w => w.hour >= first && w.hour <= (hour ?? first)).map(w => `${w.reason} (${w.count})`)]),
    ...[...history.errors].filter(([index]) => index <= playheadWindow).map(([, reason]) => `Division log window: ${reason}`),
    ...(options.totals === true ? totals.data?.warnings ?? NO_WARNINGS : NO_WARNINGS), ...(placeData.data?.warnings ?? NO_WARNINGS),
    ...(placeData.error ? [`Place names: ${placeData.error}`] : [])])], [manifest.data, history.data, history.errors, playheadWindow, first, hour, options.totals, totals.data, placeData.data, placeData.error]);
  useEffect(() => { onDiagnostics(warnings); return () => onDiagnostics(NO_WARNINGS); }, [warnings, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon="shield" message="Load a campaign to see its divisions." />;
  if (manifest.error) return <WidgetState kind="error" detail={manifest.error} onRetry={manifest.retry} />;
  if (widget.countries.kind !== 'all' && groups.error) return <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} />;
  if (!window) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (match.reason) return <WidgetState kind="unavailable" message={match.reason} />;
  if (!manifest.data || !match.ready) return <WidgetState kind="loading" label="Loading division log…" />;
  if (!manifest.data.available || events === 'lost' && !manifest.data.lossesAvailable) return <WidgetState kind="unavailable" message="Not in this recording" />;
  const error = history.errors.get(browseIndex);
  if (error) return <WidgetState kind="error" detail={error} onRetry={history.retry} />;
  if (browseIndex >= 0 && !history.data.has(browseIndex)) return <WidgetState kind="loading" label="Loading division log…" />;
  const preceding = wantedWindows.filter(index => index <= browseIndex);
  const oldest = preceding.length ? Math.min(...preceding) : 0;
  const earlier = oldest > 0 && manifest.data.windows[oldest - 1].lastHour >= first;
  const renderRow = ({ item: row }: LogRow<DivisionLogItem>) => {
    const { country, killer, place } = labels(row);
    return <><span className="log-art" aria-hidden="true">{row.icon ? <img src={row.icon} alt="" /> : <ArtFallback icon="helmet" />}</span>
      <span className="log-text">{row.tag !== null && <CountryFlagUrl className="log-flag" url={country.flag} reason={country.flagReason} pending={country.flagPending} />}<b>{country.name}</b>: <b>{divisionName(row)}</b>
        {' · '}{row.templateName ?? 'Template not recorded'}{' · '}{row.placeAtEvent ? 'at ' : 'last seen at '}{place}
        {row.killerTag !== null && <>{' · by '}<CountryFlagUrl className="log-flag" url={killer.flag} reason={killer.flagReason} pending={killer.flagPending} />{killer.name}</>}
      </span><span className="log-tag">{divisionStatus(row)}</span></>;
  };
  return <div className="log-widget division-log">
    {options.totals === true && <div className="division-log-summary" aria-live="polite">
      {totals.error ? <WidgetState kind="error" what="division totals" detail={totals.error} onRetry={totals.retry} /> : !totals.data ? 'Loading totals…' : <>
        {events !== 'lost' && <span><b>{totals.data.created.toLocaleString()}</b> created</span>}
        {events !== 'created' && <><span><b>{manifest.data.lossesAvailable ? totals.data.lost.toLocaleString() : 'Not recorded'}</b> lost / removed</span>
          <span><b>{manifest.data.lossesAvailable ? totals.data.transferred.toLocaleString() : 'Not recorded'}</b> transferred / merged</span></>}
      </>}
    </div>}
    {!manifest.data.lossesAvailable && events !== 'created' && <p className="division-log-note">Losses and transfers: Not in this recording.</p>}
    {log.chips && <div className="log-chips"><ChipSet label="Division events" chips={[{ value: 'both', label: 'Both' }, { value: 'created', label: 'Created' }, { value: 'lost', label: 'Lost / transferred' }]}
      selected={[events]} max={1} full={false} onToggle={value => setOption?.('events', value)} />
      {chosen.length > 0 && <ContentSetPicker label="Division template types" automatic="All templates" choices={templateChoices} selected={chosen}
        onReset={() => setOption?.('templates', '')} onToggle={value => {
          const next = toggleContentSet(templatesValue, value, Number.MAX_SAFE_INTEGER); if (next !== null) setOption?.('templates', next);
        }} />}</div>}
    {(earlier || shownHour !== hour) && <div className="division-log-navigation">
      {earlier && <button type="button" onClick={() => { setBrowse({ identity, hour: manifest.data!.windows[oldest - 1].lastHour }); setVisibleWindows({ identity, at: playheadWindow, windows: [] }); }}>Earlier divisions</button>}
      {shownHour !== hour && <button type="button" onClick={() => { setBrowse({ identity, hour: null }); setVisibleWindows({ identity, at: playheadWindow, windows: [] }); }}>Latest divisions</button>}
    </div>}
    <LogList rows={rows} label="Division log" density={log.density} months={log.months} animate={log.animate && live && shownHour === hour}
      columns="var(--space-6) minmax(0, 1fr) auto" renderRow={renderRow} anchorRows onVisible={onVisible}
      describe={({ item: row }) => { const named = labels(row); return `${named.country.name}: ${divisionName(row)}, ${divisionStatus(row)}, ${row.templateName ?? 'template not recorded'}, ${named.place}, ${named.killer.name}.`; }}
      tooltip={({ item: row }) => { const named = labels(row); return [divisionTooltip(row, named.country, named.killer, named.place)]; }}
      menu={({ item: row }) => [jumpItem(runtime, track.id, row.readHour)]} empty="No recorded division events match these settings." />
  </div>;
}
