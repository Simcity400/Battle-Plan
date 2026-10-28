import { useCallback, useMemo, useReducer, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { Segmented } from '../../components/Segmented';
import HeatMatrix from '../../charts/heat/HeatMatrix';
import type { MatrixAxisItem, MatrixCell } from '../../charts/heat/HeatMatrix';
import { signedColor } from '../../charts/heat/matrix';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, pinnedBreakdown, reduceDrill } from '../../charts/drill/drillState';
import { PreparedKey } from '../../charts/PreparedKey';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { useChartTokens } from '../../charts/useChartTokens';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { WidgetProps } from '../../dashboard/definitions';
import { MEMBERSHIP_PREPARING, worldFilter } from '../../dashboard/groups';
import { intelShown } from '../../dashboard/widgetRequests';
import { resolveWindow } from '../../dashboard/model';
import { usePlaybackHour } from '../../dashboard/playback';
import { pendingReplyAt, preparedPrefix } from '../../dashboard/preparedPrefix';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { mix } from '../../shell/appearance/color';
import { cellState, countriesAt, fieldNumber, measureField, pairTooltip, pairsAt, visibleCountries } from './intelCoverageData';
import type { Basis, Measure } from './intelCoverageData';
import { MEASURES, PAIR } from './definition';
import { setToken } from '../../dashboard/options';
import PairHistory from './PairHistory';
import { useIntelCoverage } from './useIntelCoverage';
import './intelCoverage.css';

/** Who has an intel network in whom at the hour shown: owners as rows, targets as columns,
 * each cell the chosen recorded measure of the owner's network in the target. */
export default function IntelCoverageWidget(props: WidgetProps) {
  const { widget, runtime, active, onDiagnostics, setOption } = props;
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const tokens = useChartTokens();
  const anchor = useRef<HTMLDivElement>(null);
  const { track, options, data, error, retry, grouped, groups, roots, chosen, touching } = useIntelCoverage(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const measure = (options.measure as Measure) ?? 'covered';
  const basis = (options.basis as Basis) ?? 'controlled';
  const field = measureField(measure, basis);
  const at = useMemo(() => data ? pairsAt(data, hour) : null, [data, hour]);
  // A group's members at the hour shown, by their tags then.
  const members = useMemo(() => grouped && track ? worldFilter(widget.countries, [], groups.data, hour, track.branch.lastHour) : null,
    [grouped, widget.countries, groups.data, hour, track]);
  const tags = members ? [...members.tags ?? []] : chosen;
  const shown = intelShown(widget);
  const ranked = useMemo(() => data ? countriesAt(data, hour, shown, tags, touching) : null,
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [data, hour, shown, JSON.stringify(tags), touching]);
  const countries = useMemo(() => ranked ?? [], [ranked]);
  const readHour = at?.readHour ?? hour;
  // Names and flags load for the axes the matrix draws, not every shown country.
  const [visible, setVisible] = useState<ReadonlySet<string>>(() => new Set());
  const onVisible = useCallback((rows: readonly string[], columns: readonly string[]) => {
    const next = new Set([...rows, ...columns]);
    setVisible(previous => previous.size === next.size && [...next].every(id => previous.has(id)) ? previous : next);
  }, []);
  const wanted = useMemo(() => visibleCountries(countries, visible).map(country => ({ hour: readHour, tag: country.tag })), [countries, visible, readHour]);
  const { labelAt, revision: labels } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const axis = useMemo<MatrixAxisItem[]>(() => countries.map(country => {
    const label = labelAt(readHour)(country.tag);
    return { id: country.id, label: label.name, countryFlag: { flag: label.flag, flagPending: label.flagPending, flagReason: label.flagReason } };
  }), [countries, labelAt, readHour]);
  const grey = mix(tokens.surface, tokens.text, 0.1);
  const highest = useMemo(() => {
    let most = 0;
    for (const [, fields] of at?.pairs ?? []) { const value = fields.get(field); if (value !== undefined) most = Math.max(most, fieldNumber(field, value)); }
    return most;
  }, [at, field]);
  const cell = useCallback((row: number, column: number): MatrixCell => {
    if (row === column || !at) return null;
    const state = cellState(at, countries[row].id, countries[column].id);
    if (state.kind === 'unknown') return { unknown: true };
    const value = state.kind === 'present' ? state.fields.get(field) : undefined;
    return value === undefined ? null : { color: signedColor(fieldNumber(field, value), 0, highest, grey, grey, tokens.accent) };
  }, [at, countries, field, highest, grey, tokens.accent]);
  const readDate = at?.readHour === undefined ? null : dayLabel(at.readHour);
  const tooltip = useCallback((row: number, column: number) => row === column ? null
    : at ? pairTooltip(axis[row].label, axis[column].label, cellState(at, countries[row].id, countries[column].id), readDate) : null, [axis, at, countries, readDate]);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const [pair, setPair] = useState<{ owner: string; target: string } | null>(null);
  const drillAt = drillHour(drill, hour);
  useDiagnostics(onDiagnostics, data?.warnings ?? null);
  const saved = options.show === 'pair';
  if (!track) return <WidgetState kind="empty" icon="eye" message="Load a campaign to see intel networks." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (saved) return <PairHistory {...props} pair={null} hour={hour} />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (grouped && groups.error && !groups.data) return <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} />;
  if (grouped && !groups.data) return <WidgetState kind="loading" label="Loading intel networks…" />;
  const reason = roots?.reason ?? members?.reason ?? (chosen?.length === 0 ? 'Choose a country in the widget settings.' : null);
  if (reason) return <WidgetState kind="unavailable" message={reason} />;
  // A group's members still being prepared are not known yet: never the most active instead.
  if (roots && !roots.ready || members && !members.known) return <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  if (!data) return <WidgetState kind="loading" label="Loading intel networks…" />;
  if (pendingReplyAt(data, hour)) return <WidgetState kind="loading" label="This range is still being prepared." />;
  if (!at || !ranked) return <WidgetState kind="unavailable" message="Intel networks were not recorded at this time." />;
  const prefix = preparedPrefix(data);
  // The chosen country alone: no network of or in it, proven only where the hour is fully read.
  const main = touching && countries.length === 1 ? at.known ? <WidgetState kind="empty" message="No intel networks of or in this country at this date." />
    : <WidgetState kind="unavailable" message="Intel networks were not fully recorded at this time." />
    : countries.length < 2 ? <WidgetState kind="empty" message="Fewer than two countries are shown at this date." />
    : !at.pairs.size && at.known ? <WidgetState kind="empty" message="No intel networks at this date." />
    : <HeatMatrix label="Intel network coverage" rows={axis} columns={axis} cell={cell} revision={`${hour}:${field}:${labels}:${highest}`} active={active} onVisible={onVisible}
      tooltip={tooltip} onSelect={(row, column) => {
        if (row === column) return;
        setPair({ owner: countries[row].id, target: countries[column].id });
        dispatch({ type: 'open', breakdown: 'pair', hour, playing: widget.time.mode === 'follow', playhead: hour });
      }} menu={() => at.readHour === undefined ? [] : [jumpItem(runtime, track.id, at.readHour)]} />;
  const names = pair ? [countries.findIndex(country => country.id === pair.owner), countries.findIndex(country => country.id === pair.target)] : [];
  return <div ref={anchor} className="intel-coverage">
    <div className="intel-coverage-caption">
      <PreparedKey pending={prefix?.pending} approximate={prefix?.approximate} />
      {setOption && <Segmented value={measure} options={MEASURES} label="Measure" onChange={value => setOption('measure', value)} />}
    </div>
    <DrillHost anchor={anchor} mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown !== null}
      title={pair ? `${axis[names[0]]?.label ?? 'Owner'} → ${axis[names[1]]?.label ?? 'Target'}` : 'Pair history'}
      day={drillAt === null ? '' : dayLabel(drillAt)} following={drill.follow} canFollow
      onClose={() => dispatch({ type: 'close' })} onFollow={() => dispatch({ type: 'follow' })} main={main}
      onPin={pair ? () => {
        const pinned = pinnedBreakdown(widget, PAIR, track.id);
        pinned.options = { ...pinned.options, owner: setToken(pair.owner), target: setToken(pair.target) };
        runtime.addWidgetBelow(widget.id, pinned);
      } : undefined}>
      {pair && <PairHistory {...props} pair={pair} hour={drillAt ?? hour} />}
    </DrillHost>
  </div>;
}
