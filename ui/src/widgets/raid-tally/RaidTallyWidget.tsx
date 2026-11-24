import { useCallback, useEffect, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import CountryRaceView from '../../charts/race/CountryRaceView';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatchFilter, noMembersAt, projectResolvedCountries, resolveCountries } from '../../dashboard/groups';
import NoMembersState from '../../dashboard/NoMembersState';
import { countryIdentityIndex } from '../../dashboard/countryIdentity';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { raidTally, unattributedAt, useRaidLog } from '../../dashboard/raids';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownEachFrame, useShownHour } from '../../dashboard/widgetPerf';
import { OPTIONS } from './definition';
import { tallyIdentities, tallyLabel, tallyLines, tallyLineTooltip, tallyModel, tallySegmentAt, tallyValue } from './raidTallyData';
import type { TallyMetric } from './raidTallyData';
import { useRegion, useRegionCoverage } from '../../dashboard/widgetTime.ts';
import ReadView from '../../dashboard/ReadView';
import { readBlocked } from '../../dashboard/gatedRead.ts';

const EMPTY_RANKED: never[] = [];

export default function RaidTallyWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const metric = options.metric as TallyMetric;
  const { track, read, data, error, retry } = useRaidLog(widget, runtime, active);
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groups = useCountryGroups(runtime, track, active && grouped);
  const fallback = defaultCountryKey(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, track?.branch.lastHour ?? 0, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const span = useMemo(() => track ? { firstHour: track.branch.firstHour, lastHour: track.branch.lastHour } : null, [track?.branch.firstHour, track?.branch.lastHour]);
  const tally = useMemo(() => data && span ? raidTally(data.episodes, span, data.missing, resolved.tags,
    resolved.top == null ? null : { count: resolved.top, metric }) : null,
  [data, span, resolved.tags, resolved.top, metric]);
  const series = useMemo(() => tallyModel(tally?.series ?? []), [tally]);
  const identities = useMemo(() => grouped && groups.data ? countryIdentityIndex(groups.data.countryIdentities) : tally ? tallyIdentities(tally.series) : null,
    [grouped, groups.data, tally]);
  const counted = useMemo(() => projectResolvedCountries(resolved, countryMatchFilter(widget.countries, fallback ? [fallback] : [],
    groups.data, track?.branch.lastHour ?? 0, identities, track?.branch.firstHour ?? 0)),
  [resolved, widget.countries, fallback, groups.data, track?.branch.lastHour, track?.branch.firstHour, identities]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const extent: [number, number] = track ? widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour] : [0, 1];
  const playhead = (widget.time.mode === 'follow' || widget.time.mode === 'pin') && window ? window.lastHour : null;
  const moment = Math.floor(playhead ?? extent[1]);
  const coverage = useRegionCoverage(runtime, useRegion(widget));
  useShownEachFrame(runtime.clock, () => data ? moment : null);
  const report = useShownHour();
  useEffect(() => { report(data ? moment : null); }, [report, data, moment]);
  const shownSeries = useMemo(() => {
    const tags = new Set(counted.tags);
    return tally?.series.filter(item => counted.top != null || tags.has(item.tag)) ?? [];
  }, [tally, counted]);
  const lines = useMemo(() => tallyLines(shownSeries, metric, { firstHour: extent[0], lastHour: extent[1] }, counted.membership),
    [shownSeries, metric, extent[0], extent[1], counted.membership]);
  const segmentAt = useCallback((tag: string, hour: number) => tallySegmentAt(series, tag, hour), [series]);
  const liveHead = useMemo(() => widget.time.mode === 'follow' && track
    ? { read: () => runtime.clock.getSnapshot().position + track.axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
  [widget.time.mode, runtime.clock, track]);
  const unattributed = tally ? unattributedAt(tally, moment) : 0;
  useDiagnostics(onDiagnostics, data || groups.data ? [...data?.warnings ?? [], ...groups.data?.warnings ?? [],
    ...unattributed ? [`${unattributed} raids have no named raider and are not counted.`] : []] : null);

  if (!track) return <WidgetState kind="empty" icon="target" message="Load a campaign to see its raid tally." />;
  if (read && readBlocked(read)) return <ReadView read={read} loading="">{() => null}</ReadView>;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="outside" read={coverage} />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  const groupFailure = grouped && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (!data || !tally || (grouped && !groups.data)) return <>{groupFailure}<WidgetState kind="loading" label="Loading the raid tally…" /></>;
  if (data.available === false) return <>{groupFailure}<WidgetState kind="unavailable" message="No raid types available." detail={data.unavailable ?? undefined} /></>;
  if (resolved.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={resolved.reason} /></>;
  const none = coverage.outside ? null : noMembersAt(widget.countries, resolved, groups.data, moment);
  if (none) return <>{groupFailure}<NoMembersState reason={none.reason} icon="target" /></>;
  if (resolved.top == null && !tally.series.some(item => item.runs.length)) return <>{groupFailure}<WidgetState kind="unavailable" message="No raid counts were recorded for these countries." /></>;
  return <>{groupFailure}<CountryRaceView runtime={runtime} track={track} active={active} options={options} view="lines" extent={extent} playhead={playhead} moment={moment}
    slots={resolved.top ?? Math.max(1, counted.tags.length)} ranked={EMPTY_RANKED} lines={lines} segmentAt={segmentAt} label={tallyLabel(metric)}
    liveHead={liveHead} presentation={{ exact: false, held: false }}
    beforeChart={metric === 'both' ? <p className="chart-line-key"><i aria-hidden="true" />Solid: launched · <i className="is-dashed" aria-hidden="true" />dashed: succeeded</p> : null}
    empty="No raid counts recorded at this time."
    formatValue={tallyValue}
    tooltip={() => null}
    lineTooltip={(hour, tags, labelAt, country) => [tallyLineTooltip(series, hour, tags, labelAt, country, metric)]} /></>;
}
