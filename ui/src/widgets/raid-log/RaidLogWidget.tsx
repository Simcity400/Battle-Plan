import { preparedPrefix } from '../../dashboard/preparedPrefix';
import { useParityRows } from '../../dashboard/parityRows';
import ArtFallback from '../../components/ArtFallback';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { ChipSet } from '../../components/ChipSet';
import WidgetState from '../../components/WidgetState';
import LogList, { PreparedLogCoverage } from '../../charts/log/LogList';
import type { LogRow } from '../../charts/log/logData';
import { logCountryRequests } from '../../charts/log/logData';
import { logSettings } from '../../charts/log/logOptions';
import { jumpItem } from '../../charts/jump';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken, storableToken, toggleContentSet, UNSTORABLE } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { raidIcon, raidPlace, raidType, raidTypeName } from '../../dashboard/raids';
import type { RaidAt } from '../../dashboard/raids';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useRaids } from '../../dashboard/useRaids';
import { OPTIONS } from './definition';
import { raidDescription, raidLogRows, raidLogTypes, raidPreparation, raidStatus, raidTooltip } from './raidLogData';

const NO_WARNINGS: string[] = [];
const NO_ROWS: readonly LogRow<RaidAt>[] = [];
const COLUMNS = 'var(--space-6) minmax(0, 1fr) auto';

export default function RaidLogWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const log = logSettings(options);
  const { track, data, error, retry } = useRaids(widget, runtime, active, { view: 'log', tags: [], top: null });
  const filtered = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && filtered);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const inBranch = track !== undefined && hour != null && hour >= track.branch.firstHour && hour <= track.branch.lastHour;
  const from = widget.time.mode === 'range' ? window?.firstHour ?? null : null;
  const fallbackKey = JSON.stringify(selectedTags(widget.countries, track));
  const fallback = useMemo<string[]>(() => JSON.parse(fallbackKey), [fallbackKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const types = typeof options.types === 'string' ? options.types : '';
  const rows = useMemo(() => data && inBranch && hour != null && match.ready ? raidLogRows(data, hour, contentSet(types).map(contentKey),
    (tag, read) => tag === null ? widget.countries.kind === 'all' : match.matches(tag, read), from) : NO_ROWS,
  [data, inBranch, hour, match, types, from, widget.countries.kind]);
  const [visible, setVisible] = useState<readonly LogRow<RaidAt>[]>(NO_ROWS);
  const wanted = useMemo(() => logCountryRequests(visible, row => ({ hour: row.hour, tags: [row.item.latest.raider, row.item.latest.victim] })), [visible]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const labels = useCallback((row: LogRow<RaidAt>): { raider: ReturnType<CountryLabel>; victim: ReturnType<CountryLabel> } => {
    const at = labelAt(row.hour);
    return { raider: row.item.latest.raider === null ? { name: 'Raider not known' } : at(row.item.latest.raider),
      victim: row.item.latest.victim === null ? { name: 'Controller not known' } : at(row.item.latest.victim) };
  }, [labelAt]);
  const chips = useMemo(() => (data ? raidLogTypes(data) : []).map(type => {
    const value = setToken(type.token);
    return { value, label: type.label, icon: type.icon, unavailable: storableToken(value) ? undefined : UNSTORABLE };
  }), [data]);
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);

  const root = useRef<HTMLDivElement>(null);
  useParityRows(root, () => ({ kind: 'log', label: 'Raid log rows', complete: !!data && match.ready,
    total: rows.length, hour, from, rows: rows.map(row => ({ id: row.id, hour: row.hour, item: row.item })) }));
  if (!track) return <WidgetState kind="empty" icon="target" message="Load a campaign to see its raids." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (filtered && groups.error) return <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} />;
  if (!data || !match.ready) return <WidgetState kind="loading" label="Loading raids…" />;
  if (data.available !== true && preparedPrefix(data)?.pending.length) return <PreparedLogCoverage coverage={preparedPrefix(data)?.coverage} exact={(data as { exact?: number }).exact} />;
  if (data.available !== true) return <WidgetState kind="unavailable" message={data.available === false ? 'No raid types available.' : 'Raids unavailable.'} />;
  if (match.reason) return <WidgetState kind="unavailable" message={match.reason} />;
  if (!inBranch) return <WidgetState kind="unavailable" message="This time is outside the recording." />;

  const chosen = contentSet(types);
  const renderRow = (row: LogRow<RaidAt>) => {
    const type = raidType(data.types, row.item.latest);
    const icon = raidIcon(data, row.item.latest);
    const preparation = raidPreparation(data, row.item);
    const { raider, victim } = labels(row);
    return <>
      <span className={`log-art${icon ? '' : ' is-missing'}`} aria-hidden="true">{icon ? <img src={icon} alt="" /> : <ArtFallback icon="target" />}</span>
      <span className="log-text">
        {row.item.latest.raider !== null && <CountryFlagUrl className="log-flag" url={raider.flag} reason={raider.flagReason} pending={raider.flagPending} />}<b>{raider.name}</b> targets{' '}
        {row.item.latest.victim !== null && <CountryFlagUrl className="log-flag" url={victim.flag} reason={victim.flagReason} pending={victim.flagPending} />}<b>{victim.name}</b> at {raidPlace(row.item.latest)}
        {' · '}{raidStatus(data, row.item)}
        {preparation && <>{' · '}{preparation}</>}
      </span>
      <span className="log-tag">{raidTypeName(type)}</span>
    </>;
  };
  return <div ref={root} className="log-widget">
    <PreparedLogCoverage coverage={preparedPrefix(data)?.coverage} exact={(data as { exact?: number }).exact} />
    {log.chips && chips.length > 0 && <div className="log-chips"><ChipSet label="Raid types" max={Number.MAX_SAFE_INTEGER} selected={chosen.length ? chosen : ['']}
      chips={[{ value: '', label: 'All types' }, ...chips]} onToggle={value => {
        if (value === '') setOption?.('types', '');
        else { const next = toggleContentSet(types, value, Number.MAX_SAFE_INTEGER); if (next !== null) setOption?.('types', next); }
      }} /></div>}
    <LogList rows={rows} label="Raids" density={log.density} months={log.months} animate={log.animate && widget.time.mode === 'follow'}
      columns={COLUMNS} renderRow={renderRow} onVisible={setVisible}
      describe={row => { const named = labels(row); return raidDescription(data, row.item, named.raider.name, named.victim.name); }}
      tooltip={row => { const named = labels(row); return [raidTooltip(data, row.item, named.raider.name, named.victim.name)]; }}
      menu={row => [jumpItem(runtime, track.id, row.hour)]} empty="No recorded raids match these settings." />
  </div>;
}
