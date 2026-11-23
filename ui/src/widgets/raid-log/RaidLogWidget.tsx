import { useCallback, useEffect, useMemo } from 'react';
import ArtFallback from '../../components/ArtFallback';
import { ChipSet } from '../../components/ChipSet';
import { CountryFlagUrl } from '../../components/CountryFlag';
import WidgetState from '../../components/WidgetState';
import LiveText, { liveTexts } from '../../charts/LiveText';
import LogList from '../../charts/log/LogList';
import type { LogRow } from '../../charts/log/logData';
import { logCountryRequests } from '../../charts/log/logData';
import { logSettings } from '../../charts/log/logOptions';
import { jumpItem } from '../../charts/jump';
import { countryNameDetails } from '../../charts/tooltip.ts';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken, storableToken, toggleContentSet, UNSTORABLE } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { validatePlaceNamesReply } from '../../dashboard/placeNamesData.ts';
import { eventsThrough, raidChangeHours, raidIcon, raidPreparationText, raidPreparing, raidStatus, raidTypeText, useRaidLog } from '../../dashboard/raids';
import type { RaidAt } from '../../dashboard/raids';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownEachFrame, useShownHour } from '../../dashboard/widgetPerf';
import { useHeadHour } from '../army-size/headHour';
import type { PlaceNamesReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { raidDescription, raidLogRows, raidLogTypes, raidTooltip, targetNameRequest, targetNames, targetText } from './raidLogData';
import '../../charts/log/log.css';
import '../../dashboard/intelligence.css';
import { productRequest } from '../../dashboard/request.ts';
import { useCampaignGate } from '../../dashboard/regionGate';
import ReadView from '../../dashboard/ReadView';
import { readBlocked } from '../../dashboard/gatedRead.ts';

const NO_ROWS: readonly LogRow<RaidAt>[] = [];
const COLUMNS = 'var(--space-6) minmax(0, 1fr) auto';

export default function RaidLogWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const log = logSettings(options);
  const { track, read, data, error, retry } = useRaidLog(widget, runtime, active);
  const filtered = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && filtered);
  const following = active && widget.time.mode === 'follow' && !!track;
  const changes = useMemo(() => raidChangeHours(data?.episodes ?? []), [data]);
  const trackHour = (clockHour: number) => track ? resolveWindow(widget, track, runtime.tracks, clockHour)?.lastHour ?? null : null;
  const clockHour = useHeadHour(runtime.clock, following, at => { const hour = trackHour(at); return hour === null ? '' : String(eventsThrough(changes, hour)); });
  const window = track && resolveWindow(widget, track, runtime.tracks, following ? clockHour : runtime.clock.getHour());
  const hour = window ? Math.floor(window.lastHour) : null;
  const inBranch = track !== undefined && hour !== null && hour >= track.branch.firstHour && hour <= track.branch.lastHour;
  const from = widget.time.mode === 'range' ? window?.firstHour ?? null : null;
  const fallbackKey = JSON.stringify(selectedCountryKeys(widget.countries, track));
  const fallback = useMemo<string[]>(() => JSON.parse(fallbackKey), [fallbackKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const types = typeof options.types === 'string' ? options.types : '';
  const rows = useMemo(() => data && match.ready && inBranch && hour !== null ? raidLogRows(data, hour, contentSet(types).map(contentKey),
    (tag, read) => tag === null ? widget.countries.kind === 'all' : match.matches(tag, read), from) : NO_ROWS,
  [data, match, types, from, widget.countries.kind, inBranch, hour]);
  const liveHour = useCallback(() => following ? trackHour(runtime.clock.getHour()) ?? hour ?? 0 : hour ?? 0,
    [following, runtime.clock, hour, track, widget]);
  useShownEachFrame(runtime.clock, () => data && hour !== null ? Math.floor(liveHour()) : null);
  const report = useShownHour();
  useEffect(() => { report(data && hour !== null ? Math.floor(liveHour()) : null); }, [report, data, hour, liveHour]);
  const preparing = useMemo(() => data ? rows.filter(row => raidPreparing(data, row.item)) : NO_ROWS, [data, rows]);
  const preparation = useMemo(() => data && preparing.length ? liveTexts(runtime.clock, () => Math.floor(liveHour()),
    at => new Map(preparing.map(row => [row.key, raidPreparationText(data, row.item, at) ?? '']))) : null,
  [data, preparing, runtime.clock, liveHour]);
  const wanted = useMemo(() => logCountryRequests(rows, row => ({ hour: row.hour, tags: [row.item.episode.raider, row.item.episode.victim] })), [rows]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const labels = useCallback((row: LogRow<RaidAt>): { raider: ReturnType<CountryLabel>; victim: ReturnType<CountryLabel> } => {
    const at = labelAt(row.hour);
    return { raider: row.item.episode.raider === null ? { name: 'Raider not known' } : at(row.item.episode.raider),
      victim: row.item.episode.victim === null ? { name: 'Controller not known' } : at(row.item.episode.victim) };
  }, [labelAt]);
  const lastHour = track?.branch.lastHour;
  const gate = useCampaignGate(runtime, widget, active);
  const namesAsked = useMemo(() => data && lastHour !== undefined ? targetNameRequest(data.episodes, lastHour) : null, [data, lastHour]);
  const placeNames = useProduct<PlaceNamesReply>(runtime.client, track && namesAsked && (namesAsked.places.length || namesAsked.states.length) ? productRequest({
    sourceId: track.source.id, revision: track.source.revision, product: 'place-names',
    params: { branchId: track.branch.id, land: true, places: namesAsked.places.flatMap(place => { const hour = gate.recorded(place.hour); return hour === null ? [] : [{ ...place, hour }]; }),
      states: namesAsked.states.flatMap(state => { const hour = gate.recorded(state.hour); return hour === null ? [] : [{ ...state, hour }]; }) } }) : null, active, validatePlaceNamesReply, track?.source);
  const names = useMemo(() => placeNames.data ? targetNames(placeNames.data) : null, [placeNames.data]);
  const chips = useMemo(() => (data ? raidLogTypes(data) : []).map(type => {
    const value = setToken(type.token);
    return { value, label: type.label, icon: type.icon, unavailable: storableToken(value) ? undefined : UNSTORABLE };
  }), [data]);
  useDiagnostics(onDiagnostics, data ? [...data.warnings, ...placeNames.data?.warnings ?? [], ...placeNames.error ? [`Raid targets are not named: ${placeNames.error.message}`] : []] : null);

  if (!track) return <WidgetState kind="empty" icon="target" message="Load a campaign to see its raids." />;
  if (read && readBlocked(read)) return <ReadView read={read} loading="">{() => null}</ReadView>;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  const groupFailure = filtered && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (!data || !match.ready) return <>{groupFailure}<WidgetState kind="loading" label="Loading raids…" /></>;
  if (data.available === false) return <>{groupFailure}<WidgetState kind="unavailable" message="No raid types available." detail={data.unavailable ?? undefined} /></>;
  if (match.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={match.reason} /></>;
  if (!inBranch) return <>{groupFailure}<WidgetState kind="outside" /></>;

  const chosen = contentSet(types);
  const renderRow = (row: LogRow<RaidAt>) => {
    const icon = raidIcon(data, row.item.episode);
    const { raider, victim } = labels(row);
    return <>
      <span className={`log-art${icon ? '' : ' is-missing'}`} aria-hidden="true" title={icon ? undefined : row.item.episode.iconMissing ?? undefined}>{icon ? <img src={icon} alt="" /> : <ArtFallback icon="target" />}</span>
      <span className="log-text">
        {row.item.episode.raider !== null && <CountryFlagUrl className="log-flag" url={raider.flag} detail={raider.flagDetail} reason={raider.flagReason} pending={raider.flagPending} />}<b>{raider.name}</b> targets{' '}
        {row.item.episode.victim !== null && <CountryFlagUrl className="log-flag" url={victim.flag} detail={victim.flagDetail} reason={victim.flagReason} pending={victim.flagPending} />}<b title={row.item.episode.victimMissing ?? undefined}>{victim.name}</b> at {targetText(row.item.episode, names)}
        {' · '}{raidStatus(data, row.item)}
        {preparation && raidPreparing(data, row.item) ? <>{' · '}<LiveText live={preparation} at={row.key} /></> : null}
      </span>
      <span className="log-tag">{raidTypeText(data, row.item.episode)}</span>
    </>;
  };
  const missing = data.missing.length;
  return <div className="log-widget">{groupFailure}
    {missing > 0 && <p className="intel-note">Raids are not recorded for {missing} stretch{missing === 1 ? '' : 'es'} of this campaign.</p>}
    {log.chips && chips.length > 0 && <div className="log-chips"><ChipSet label="Raid types" max={Number.MAX_SAFE_INTEGER} selected={chosen.length ? chosen : ['']}
      chips={[{ value: '', label: 'All types' }, ...chips]} onToggle={value => {
        if (value === '') setOption?.('types', '');
        else { const next = toggleContentSet(types, value, Number.MAX_SAFE_INTEGER); if (next !== null) setOption?.('types', next); }
      }} /></div>}
    <LogList rows={rows} label="Raids" density={log.density} months={log.months} animate={log.animate && widget.time.mode === 'follow'}
      playhead={widget.time.mode === 'follow' ? hour : null} columns={COLUMNS} renderRow={renderRow}
      describe={row => { const named = labels(row); return raidDescription(data, row.item, named.raider.name, named.victim.name, targetText(row.item.episode, names), Math.floor(liveHour())); }}
      tooltip={row => { const named = labels(row); return [{ ...raidTooltip(data, row.item, named.raider.name, named.victim.name, targetText(row.item.episode, names), Math.floor(liveHour())), ...countryNameDetails(named.raider, named.victim) }]; }}
      menu={row => [jumpItem(runtime, track.id, row.hour)]} empty="No recorded raids match these settings." />
  </div>;
}
