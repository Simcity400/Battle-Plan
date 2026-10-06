import { useCallback, useEffect, useMemo, useState } from 'react';
import { ChipSet } from '../../components/ChipSet';
import WidgetState from '../../components/WidgetState';
import LogList from '../../charts/log/LogList';
import type { LogRow } from '../../charts/log/logData';
import { logSettings } from '../../charts/log/logOptions';
import ProgressMeter from '../../charts/meter/ProgressMeter';
import { jumpItem } from '../../charts/jump';
import { runAt } from '../../charts/series';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { selectedTags } from '../../dashboard/countrySelection';
import { formatDateTime } from '../../dashboard/calendar';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, toggleContentSet } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { OPTIONS } from './definition';
import { operationChoices, operationJumpHours, operationName, operationRows, operationState, operationTooltip, operationWording, operationProgress, operationOutcome } from './espionageData';
import type { OperationAt } from './espionageData';
import { useEspionageLog } from './useEspionageLog';
import './espionageLog.css';

const NO_WARNINGS: string[] = [];
const NO_ROWS: readonly LogRow<OperationAt>[] = [];
const COLUMNS = 'var(--space-6) minmax(0, 1fr) auto';
const METER_COLUMNS = 'var(--space-6) minmax(0, 2fr) minmax(0, 1fr) auto';
const drawsMeter = (at: OperationAt, live: boolean) => live && (at.state === 'preparing' || at.state === 'running');

export default function EspionageLogWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const log = logSettings(options);
  const { track, data, error, retry } = useEspionageLog(widget, runtime, active);
  const grouped = widget.countries.kind === 'group';
  const groups = useCountryGroups(runtime, track, active && grouped);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const from = widget.time.mode === 'range' ? window?.firstHour ?? null : null;
  const selected = selectedTags(widget.countries, track);
  const selectedKey = JSON.stringify(selected);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const fallback = useMemo(() => selected, [selectedKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const types = typeof options.types === 'string' ? options.types : '';
  const role = options.countryRole === 'target' || options.countryRole === 'either' ? options.countryRole : 'owner';
  const live = widget.time.mode === 'follow';
  const runningOrder = options.runningOrder === 'preparation' ? 'preparation' : 'start';
  const includeUnknownCountries = widget.countries.kind === 'all';
  const rows = useMemo(() => data && hour != null && match.ready
    ? operationRows(data, hour, { types: contentSet(types).map(contentKey), role, country: match.matches, from, live, runningOrder, includeUnknownCountries }) : NO_ROWS,
  [data, hour, match, types, role, from, live, runningOrder, includeUnknownCountries]);
  const [visible, setVisible] = useState<readonly LogRow<OperationAt>[]>(NO_ROWS);
  // Names and flags belong to each displayed row's date, including an ending's upper bound.
  const wanted = useMemo(() => {
    const seen = new Map<string, { hour: number; tag: string }>();
    for (const row of visible) for (const tag of [row.item.ownerTag, row.item.targetTag]) {
      if (tag !== null) seen.set(JSON.stringify([row.hour, tag]), { hour: row.hour, tag });
    }
    return [...seen.values()];
  }, [visible]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const labels = useCallback((row: LogRow<OperationAt>, tag: string | null) => tag === null ? { name: 'Unknown country' } : labelAt(row.hour)(tag), [labelAt]);
  const chips = useMemo(() => operationChoices(data), [data]);
  const byType = useMemo(() => new Map(data?.types.map(type => [type.key, type])), [data]);
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon="target" message="Load a campaign to see its espionage operations." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (grouped && groups.error) return <WidgetState kind="error" message={`Country groups are unavailable: ${groups.error}`} onRetry={groups.retry} />;
  if (!data || (grouped && !groups.data)) return <WidgetState kind="loading" label="Loading espionage operations…" />;
  if (match.reason) return <WidgetState kind="unavailable" message={match.reason} />;
  if (data.unavailable && !data.completionsAvailable.length) return <WidgetState kind="unavailable" message={data.unavailable} />;
  if (hour == null) return <WidgetState kind="unavailable" message="The chosen moment is outside this campaign branch." />;
  if (!data.available.length && !data.completionsAvailable.length) return <WidgetState kind="unavailable" message="Espionage operations are not in this recording." />;
  if (!runAt(data.available, hour) && !runAt(data.completionsAvailable, hour) && !rows.length) return <WidgetState kind="unavailable" message="Operations were not recorded at this hour. Gaps stay gaps." />;

  const chosen = contentSet(types);
  const toggle = (value: string) => { const next = toggleContentSet(types, value, 16); if (next !== null) setOption?.('types', next); };
  const renderRow = (row: LogRow<OperationAt>) => {
    const at = row.item;
    const type = at.typeKey === null ? undefined : byType.get(at.typeKey);
    const owner = labels(row, at.ownerTag);
    const target = labels(row, at.targetTag);
    const meter = drawsMeter(at, live);
    const wording = operationWording(at);
    const outcome = operationOutcome(at);
    return <>
      <span className={`log-art${type?.icon ? '' : ' is-missing'}`} aria-hidden="true">{type?.icon ? <img src={type.icon} alt="" /> : '?'}</span>
      <span className="log-text">
        {owner.flag && <img className="log-flag" src={owner.flag} alt="" />}<b>{owner.name}</b>
        <span className="espionage-muted">{wording.action}</span>{operationName(type)}<span className="espionage-muted"> against </span>
        {target.flag && <img className="log-flag" src={target.flag} alt="" />}<b>{target.name}</b><span className="espionage-muted">{wording.afterTarget}</span>
        {at.startHour !== null && <span className="espionage-muted"> · started {formatDateTime(at.startHour)}</span>}
        {!meter && <span className="espionage-muted"> · {operationState(at)}{outcome && <> · {outcome}</>}</span>}
      </span>
      {meter && <span className="espionage-progress">
        <span>{operationState(at)}{outcome && <> · {outcome}</>}</span>
        <ProgressMeter value={operationProgress(at)} state="live" label={at.state === 'running' ? 'Recorded execution progress: elapsed / duration' : 'Recorded overall civilian preparation'} />
      </span>}
      <span className="log-tag espionage-type"><i />Espionage</span>
    </>;
  };
  return <div className="espionage-log">
    {log.chips && chips.length > 1 && <div className="log-chips">
      <ChipSet label="Operation types" max={16} selected={chosen.length ? chosen : ['']}
        chips={[{ value: '', label: 'All types' }, ...chips]}
        onToggle={value => { if (value === '') { if (chosen.length) setOption?.('types', ''); } else toggle(value); }} />
    </div>}
    <LogList key={`${track.source.id}:${track.branch.id}`} rows={rows} label="Espionage operations" density={log.density} months={log.months}
      animate={log.animate && live} columns={row => drawsMeter(row.item, live) ? METER_COLUMNS : COLUMNS} renderRow={renderRow}
      describe={row => `${labels(row, row.item.ownerTag).name}${operationWording(row.item).action}${operationName(row.item.typeKey === null ? undefined : byType.get(row.item.typeKey))} against ${labels(row, row.item.targetTag).name}${operationWording(row.item).afterTarget}. ${operationState(row.item)}.`}
      tooltip={row => operationTooltip(data, row, tag => labels(row, tag).name)}
      menu={row => operationJumpHours(row.item, hour, row.hour).map(at => jumpItem(runtime, track.id, at))}
      empty="No espionage operations match these countries and settings by this moment." onVisible={setVisible} />
  </div>;
}
