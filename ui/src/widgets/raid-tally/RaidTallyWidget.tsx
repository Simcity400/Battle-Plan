import { useCallback, useEffect, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import CountryRaceView from '../../charts/race/CountryRaceView';
import { runAt } from '../../charts/series';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { rankedCandidates, resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useRaids } from '../../dashboard/useRaids';
import { readAt } from '../../dashboard/reads';
import { OPTIONS } from './definition';
import { tallyEntries, tallyLabel, tallyLines, tallyLineTooltip, tallyModel, tallyTooltip, tallyValue } from './raidTallyData';
import type { TallyMetric } from './raidTallyData';

const NO_WARNINGS: string[] = [];
export default function RaidTallyWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const metric = options.metric as TallyMetric;
  const view = options.view === 'lines' ? 'lines' : 'race';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groups = useCountryGroups(runtime, track, active && grouped);
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, track?.branch.lastHour ?? 0, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const params = { view: 'tally' as const, tags: resolved.tags, top: resolved.top === undefined ? null : { count: resolved.top, metric } };
  const { data, error, retry } = useRaids(widget, runtime, active && (!grouped || !!groups.data), params);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const extent: [number, number] = track ? widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour] : [0, 1];
  const playhead = (widget.time.mode === 'follow' || widget.time.mode === 'pin') && window ? window.lastHour : null;
  const moment = Math.floor(playhead ?? extent[1]);
  const series = useMemo(() => tallyModel(data?.series ?? []), [data]);
  const ranked = useMemo(() => tallyEntries(series, rankedCandidates(resolved, moment, [...series.keys()]), moment, metric, resolved.top, resolved.top != null),
    [series, resolved, moment, metric]);
  const lines = useMemo(() => tallyLines(data?.series ?? [], metric, { firstHour: extent[0], lastHour: extent[1] }, resolved.membership),
    [data, metric, extent[0], extent[1], resolved.membership]);
  const segmentAt = useCallback((tag: string, hour: number) => runAt(series.get(tag)?.segments ?? [], Math.floor(hour))?.firstHour ?? null, [series]);
  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(groups.data?.warnings ?? [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groups.data, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon="target" message="Load a campaign to see its raid tally." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (grouped && groups.error) return <WidgetState kind="error" message={groups.error} onRetry={groups.retry} />;
  if (!data || (grouped && !groups.data)) return <WidgetState kind="loading" label="Loading the raid tally…" />;
  if (data.available !== true) return <WidgetState kind="unavailable" message={data.unavailable ?? (data.available === false ? 'Not in this recording' : 'Raid availability not known')} />;
  if (resolved.reason) return <WidgetState kind="unavailable" message={resolved.reason} />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="This time is outside the recorded branch." />;
  if (resolved.top == null && !data.series.some(item => item.runs.length)) return <WidgetState kind="unavailable" message="No raid counts were recorded for these countries." />;
  return <CountryRaceView runtime={runtime} track={track} active={active} options={options} view={view} extent={extent} playhead={playhead} moment={moment}
    slots={resolved.top ?? Math.max(1, resolved.tags.length)} ranked={ranked} lines={lines} segmentAt={segmentAt} label={tallyLabel(metric)}
    presentation={{ exact: false, held: false }}
    beforeChart={metric === 'both' ? view === 'lines'
      ? <p className="chart-line-key"><i aria-hidden="true" />Solid: launched · <i className="is-dashed" aria-hidden="true" />dashed: succeeded</p>
      : <p className="chart-line-key">Outer: launched · inner: succeeded. Successes can exceed proven launches.</p> : null}
    empty={resolved.top != null && readAt(data.readHours, moment) === moment ? `No raids ${metric === 'both' ? 'observed' : metric} yet.` : 'No raid counts recorded at this time.'}
    formatValue={value => `${tallyValue(value)} observed`}
    tooltip={(tag, hour, name) => [tallyTooltip(data, tag, hour, name)]}
    lineTooltip={(hour, tags, labelAt, country) => [tallyLineTooltip(series, hour, tags, labelAt, country, metric)]} />;
}
