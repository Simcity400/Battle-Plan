import { useCallback, useEffect, useMemo, useState } from 'react';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { ChipSet } from '../../components/ChipSet';
import WidgetState from '../../components/WidgetState';
import LogList from '../../charts/log/LogList';
import type { LogRow } from '../../charts/log/logData';
import { logSettings } from '../../charts/log/logOptions';
import { jumpItem } from '../../charts/jump';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken, storableToken, toggleContentSet, UNSTORABLE } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { ShipsSunkRow } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { shipBattleLabel, shipBattleRequests, shipClassChoices, shipClassName, shipCountryLabels, shipCountryRequests, shipCoverage, shipCauseLabel, shipDescription, shipLogOrder, shipLogRows, shipName, shipPlace, shipTooltip } from './shipsSunkData';
import { useShipsSunk, useShipsSunkBattleTitles } from './useShipsSunk';

const NO_WARNINGS: string[] = [];
const NO_ROWS: readonly LogRow<ShipsSunkRow>[] = [];
const COLUMNS = 'var(--space-6) minmax(0, 1fr) auto';

export default function ShipsSunkWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const log = logSettings(options);
  const { track, data, error, retry } = useShipsSunk(widget, runtime, active);
  const filtered = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && filtered);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const from = widget.time.mode === 'range' ? window?.firstHour ?? null : null;
  const fallbackKey = JSON.stringify(selectedTags(widget.countries, track));
  const fallback = useMemo<string[]>(() => JSON.parse(fallbackKey), [fallbackKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const classes = typeof options.classes === 'string' ? options.classes : '';
  const role = options.countryRole === 'victim' || options.countryRole === 'sinker' ? options.countryRole : 'both';
  const campaignKey = track?.source.metadata.campaignKey ?? '';
  const ordered = useMemo(() => data ? shipLogOrder(data) : NO_ROWS, [data]);
  const rows = useMemo(() => data && hour != null && match.ready ? shipLogRows(data, hour, contentSet(classes).map(contentKey), role,
    (tag, namingHour) => tag === null ? widget.countries.kind === 'all' : match.matches(tag, namingHour), from, ordered, campaignKey) : NO_ROWS,
  [data, hour, match, classes, role, from, widget.countries.kind, ordered, campaignKey]);
  const [visible, setVisible] = useState<readonly LogRow<ShipsSunkRow>[]>(NO_ROWS);
  const titleRows = useMemo(() => shipBattleRequests(visible, data), [visible, data]);
  const titles = useShipsSunkBattleTitles(runtime, track, titleRows, active);
  const titleByRow = useMemo(() => new Map(titles.data?.rows.map(row => [String(row.index), row])), [titles.data]);
  const wanted = useMemo(() => shipCountryRequests(visible, data), [visible, data]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const labels = useCallback((row: ShipsSunkRow) => shipCountryLabels(row, labelAt), [labelAt]);
  const chips = useMemo(() => shipClassChoices(data, campaignKey).map(choice => {
    const value = setToken(choice.value);
    return { ...choice, value, unavailable: storableToken(value) ? undefined : UNSTORABLE };
  }), [data, campaignKey]);
  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(groups.data?.warnings ?? []), ...(titles.data?.warnings ?? [])];
    if (titles.error) warnings.push(`Ships sunk battle titles unavailable for ${titleRows.length} drawn rows: ${titles.error}`);
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groups.data, titles.data, titles.error, titleRows.length, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon="history" message="Load a campaign to see its ships sunk log." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (filtered && groups.error) return <WidgetState kind="error" message={groups.error} onRetry={groups.retry} />;
  if (!data || !match.ready) return <WidgetState kind="loading" label="Loading ships sunk…" />;
  if (!data.available) return <WidgetState kind="unavailable" message="Not in this recording" />;
  if (match.reason) return <WidgetState kind="unavailable" message={match.reason} />;
  if (hour == null || hour < track.branch.firstHour || hour > track.branch.lastHour)
    return <WidgetState kind="unavailable" message="This time is outside the recorded branch." />;

  const chosen = contentSet(classes);
  const renderRow = (logRow: LogRow<ShipsSunkRow>) => {
    const row = logRow.item;
    const type = data.classes[row.classIndex];
    const { victim, attacker } = labels(row);
    const battle = shipBattleLabel(row, titleByRow.get(logRow.key));
    const attackerFlag = row.attacker === null ? null : <CountryFlagUrl className="log-flag"
      url={attacker.flag} reason={attacker.flagReason} pending={attacker.flagPending} />;
    return <>
      <span className={`log-art${type.icon ? '' : ' is-missing'}`} aria-hidden="true">{type.icon ? <img src={type.icon} alt="" /> : '?'}</span>
      <span className="log-text">
        <b>{shipName(row)}</b>, {shipClassName(data, row)}{' · '}
        {row.victim !== null && <CountryFlagUrl className="log-flag" url={victim.flag} reason={victim.flagReason} pending={victim.flagPending} />}<b>{victim.name}</b>{', '}
        {row.cause === 'cause' ? <>lost to {row.attributionName ?? 'Not recorded'}{row.attacker !== null && <>; attributed to {attackerFlag}<b>{attacker.name}</b></>}</>
          : row.assistedKill || row.cause === 'assisted' ? <>sunk with assistance from {attackerFlag}<b>{attacker.name}</b></>
            : <>sunk by {attackerFlag}<b>{attacker.name}</b></>}
        {' in '}{shipPlace(data, row)}{battle && <>{', '}{battle}</>}
      </span>
      <span className="log-tag">{shipCauseLabel(row)}</span>
    </>;
  };
  return <div className="log-widget">
    {log.chips && chips.length > 1 && <div className="log-chips"><ChipSet label="Ship classes" max={Number.MAX_SAFE_INTEGER} selected={chosen.length ? chosen : ['']}
      chips={[{ value: '', label: 'All classes' }, ...chips]} onToggle={value => {
        if (value === '') setOption?.('classes', '');
        else { const next = toggleContentSet(classes, value, Number.MAX_SAFE_INTEGER); if (next !== null) setOption?.('classes', next); }
      }} /></div>}
    <LogList rows={rows} label="Ships sunk" density={log.density} months={log.months} animate={log.animate && widget.time.mode === 'follow'}
      columns={COLUMNS} renderRow={renderRow} onVisible={setVisible}
      describe={row => { const named = labels(row.item); return shipDescription(data, row.item, named.victim.name, named.attacker.name, titleByRow.get(row.key)); }}
      tooltip={row => { const named = labels(row.item); return [shipTooltip(data, row.item, named.victim.name, named.attacker.name, titleByRow.get(row.key), titles.error)]; }}
      menu={row => [jumpItem(runtime, track.id, row.hour)]} empty={`No ships sunk recorded${chosen.length || from !== null || widget.countries.kind !== 'all' ? ' for these settings' : ''}. ${shipCoverage(data)}`} />
  </div>;
}
