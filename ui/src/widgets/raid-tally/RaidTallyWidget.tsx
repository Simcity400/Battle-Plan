import { pendingAt, preparedPrefix } from '../../dashboard/preparedPrefix';
import { useParityRows } from '../../dashboard/parityRows';
import { useCallback, useEffect, useMemo, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import CountryRaceView from '../../charts/race/CountryRaceView';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatchFilter, MEMBERSHIP_PREPARING, noMembersAt, projectResolvedCountries, rankedCandidates, resolveCountries } from '../../dashboard/groups';
import NoMembersState from '../../dashboard/NoMembersState';
import { countryIdentityIndex } from '../../dashboard/countryIdentity';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useRaids } from '../../dashboard/useRaids';
import { readAt } from '../../dashboard/reads';
import { OPTIONS } from './definition';
import { tallyEntries, tallyIdentities, tallyLabel, tallyLines, tallyLineTooltip, tallyModel, tallySegmentAt, tallyTooltip, tallyValue } from './raidTallyData';
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
  const identities = useMemo(() => grouped && groups.data ? countryIdentityIndex(groups.data.countryIdentities)
    : data ? tallyIdentities(data.series) : null, [grouped, groups.data, data]);
  const counted = useMemo(() => projectResolvedCountries(resolved, countryMatchFilter(widget.countries, fallback ? [fallback] : [],
    groups.data, track?.branch.lastHour ?? 0, identities, track?.branch.firstHour ?? 0)),
    [resolved, widget.countries, fallback, groups.data, track?.branch.lastHour, track?.branch.firstHour, identities]);
  const ranked = useMemo(() => tallyEntries(series, rankedCandidates(counted, moment, [...series.keys()]), moment, metric, resolved.top, resolved.top != null),
    [series, counted, moment, metric, resolved.top]);
  const shownSeries = useMemo(() => {
    const tags = new Set(counted.tags);
    return data?.series.filter(item => counted.top != null || tags.has(item.tag)) ?? [];
  }, [data, counted]);
  const lines = useMemo(() => tallyLines(shownSeries, metric, { firstHour: extent[0], lastHour: extent[1] }, counted.membership),
    [shownSeries, metric, extent[0], extent[1], counted.membership]);
  const segmentAt = useCallback((tag: string, hour: number) => tallySegmentAt(series, tag, hour), [series]);
  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(groups.data?.warnings ?? [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groups.data, onDiagnostics]);

  const root = useRef<HTMLDivElement>(null);
  useParityRows(root, () => ({ kind: 'chart', label: 'Raid tally values', complete: !!data,
    total: shownSeries.length, moment, metric, rows: shownSeries }));
  if (!track) return <WidgetState kind="empty" icon="target" message="Load a campaign to see its raid tally." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (grouped && groups.error) return <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} />;
  if (!data || (grouped && !groups.data)) return <WidgetState kind="loading" label="Loading the raid tally…" />;
  if (pendingAt(preparedPrefix(data), moment)) return <WidgetState kind="loading" label="This range is still being prepared." />;
  if (data.available !== true) return <WidgetState kind="unavailable" message={data.available === false ? 'No raid types available.' : 'Raid totals unavailable.'} />;
  if (resolved.reason) return <WidgetState kind="unavailable" message={resolved.reason} />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  // A group with no member found yet has no count to show: the moment says why once it is read.
  const none = noMembersAt(widget.countries, resolved, groups.data, moment, track.branch.lastHour);
  if (none) return <NoMembersState reason={none.reason} icon="target" />;
  // A group's members at hours still being prepared are not known yet: no count is not proof of none.
  if (resolved.top == null && !data.series.some(item => item.runs.length)) return resolved.known(track.branch.firstHour, track.branch.lastHour)
    ? <WidgetState kind="unavailable" message="No raid counts were recorded for these countries." /> : <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  return <div ref={root} style={{ display: 'contents' }}><CountryRaceView runtime={runtime} track={track} active={active} options={options} view={view} extent={extent} playhead={playhead} moment={moment}
    slots={resolved.top ?? Math.max(1, counted.tags.length)} ranked={ranked} lines={lines} segmentAt={segmentAt} label={tallyLabel(metric)}
    presentation={{ exact: false, held: false }}
    beforeChart={metric === 'both' ? view === 'lines'
      ? <p className="chart-line-key"><i aria-hidden="true" />Solid: launched · <i className="is-dashed" aria-hidden="true" />dashed: succeeded</p>
      : <p className="chart-line-key">Outer: launched · inner: succeeded.</p> : null}
    empty={resolved.top != null && readAt(data.readHours, moment) === moment ? `No raids ${metric === 'both' ? 'launched or succeeded' : metric} yet.`
      : counted.known(moment) ? 'No raid counts recorded at this time.' : MEMBERSHIP_PREPARING}
    formatValue={tallyValue}
    tooltip={(tag, hour, name) => [tallyTooltip(data, tag, hour, name)]}
    lineTooltip={(hour, tags, labelAt, country) => [tallyLineTooltip(series, hour, tags, labelAt, country, metric)]} /></div>;
}
