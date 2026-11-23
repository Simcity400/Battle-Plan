import { useCallback, useMemo,  useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import HeatMatrix from '../../charts/heat/HeatMatrix';
import type { MatrixAxisItem } from '../../charts/heat/HeatMatrix';
import { signedColor } from '../../charts/heat/matrix';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, pinnedBreakdown } from '../../charts/drill/drillState';
import { useDrill } from '../../charts/drill/useDrill';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { mapLive } from '../../charts/live';
import { countryNameDetails } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch } from '../../dashboard/groups';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { shownHour, useIntelLabels } from '../../dashboard/IntelCountry';
import { resolveOptions, setToken } from '../../dashboard/options';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { mix } from '../../shell/appearance/color';
import { OPTIONS, PAIR } from './definition';
import { cellState, countriesAt, coverageStructure, matrixCells, measureField, pairOptions, pairsAt, pairTooltip, validateIntelCoverage } from './intelCoverageData';
import type { Basis, ChosenPair, Measure } from './intelCoverageData';
import PairHistory from './PairHistory';
import './intelCoverage.css';
import { useChangedState } from '../../dashboard/changedState.ts';
import { useRegion, useRegionHour } from '../../dashboard/widgetTime.ts';

const PARAMS = {} as const;
const NO_TAGS: readonly string[] = [];

export default function IntelCoverageWidget(props: WidgetProps) {
  const { widget, runtime, active, onDiagnostics } = props;
  const options = resolveOptions(OPTIONS, widget.options);
  const tokens = useChartTokens();
  const anchor = useRef<HTMLDivElement>(null);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const saved = options.show === 'pair';
  const readHour = useRef<number | null>(null);
  const gate = useGate(runtime, widget);
  const head = useHeadProduct(runtime, gate, 'intel-network-coverage', track && !saved ? PARAMS : null,
    { active, validate: validateIntelCoverage, structure: coverageStructure, paint: reply => { readHour.current = reply.readHour; } });
  useShownEachFrame(runtime.clock, head.shownAt, head.live);
  const data = head.data;
  if (data && head.read !== 'followed') readHour.current = data.readHour;
  const hour = head.at ?? shownHour(runtime, widget, head) ?? track?.branch.lastHour ?? 0;
  const measure = (options.measure as Measure) ?? 'covered';
  const basis = (options.basis as Basis) ?? 'controlled';
  const field = measureField(measure, basis);
  const countries = widget.countries;
  const top = countries.kind === 'group' && countries.group.type === 'top' ? countries.group.count : null;
  const filtered = countries.kind === 'tags' || countries.kind === 'selected' || countries.kind === 'group' && top === null;
  const groups = useCountryGroups(runtime, track, active && filtered);
  const chosen = useMemo(() => countries.kind === 'tags' || countries.kind === 'selected' ? selectedCountryKeys(countries, track) : null, [countries, track]);
  const selection = useMemo(() => filtered ? countryMatch(countries, chosen ?? [], groups.data, track?.branch.lastHour ?? hour) : null,
    [filtered, countries, chosen, groups.data, track?.branch.lastHour, hour]);
  const shown = top ?? (Number(options.mostCountries) || 12);
  const touching = top !== null ? top === 1 : chosen?.length === 1;
  const at = useMemo(() => data ? pairsAt(data) : null, [data]);
  const ordered = useMemo(() => data && (!selection || selection.ready) ? countriesAt(data, shown, selection ? tag => selection.matches(tag, hour) : undefined, touching) : null,
    [data, shown, selection, hour, touching]);
  const axisTags = ordered ?? NO_TAGS;
  const [visible, setVisible] = useChangedState<readonly string[]>(NO_TAGS);
  const onVisible = useCallback((rows: readonly string[], columns: readonly string[]) => {
    const next = [...new Set([...rows, ...columns])];
    setVisible(previous => previous.length === next.length && previous.every((tag, index) => tag === next[index]) ? previous : next);
  }, []);
  const label = useIntelLabels(runtime, widget, visible, hour, active, head.read === 'followed');
  const axis = useMemo<MatrixAxisItem[]>(() => axisTags.map(tag => {
    const shownLabel = label(tag);
    return { id: tag, label: shownLabel.known === false ? tag : shownLabel.name, countryFlag: { ...(shownLabel.nameFailure ? { nameFailure: shownLabel.nameFailure } : {}),
      ...(shownLabel.flagDetail !== undefined ? { flagDetail: shownLabel.flagDetail } : {}), flag: shownLabel.flag, flagPending: shownLabel.flagPending, flagReason: shownLabel.flagReason } };
  }), [axisTags, label]);
  const grey = mix(tokens.surface, tokens.text, 0.1);
  const colour = useCallback((value: number, highest: number) => signedColor(value, 0, highest, grey, grey, tokens.accent), [grey, tokens.accent]);
  const live = head.live;
  const cells = useMemo(() => at ? mapLive(live, values => matrixCells(at, values, field, colour)) : undefined, [at, live, field, colour]);
  const cell = useCallback((row: number, column: number) => cells?.read().get(`${axisTags[row]}\u0000${axisTags[column]}`) ?? null, [cells, axisTags]);
  const tooltip = useCallback((row: number, column: number) => row === column || !at ? null : {
    ...pairTooltip(axis[row].label, axis[column].label, cellState(at, live.read(), axisTags[row], axisTags[column]),
      readHour.current === null ? null : dayLabel(readHour.current)),
    ...countryNameDetails(axis[row].countryFlag, axis[column].countryFlag),
  }, [at, live, axis, axisTags]);
  const drillControl = useDrill(runtime, useRegion(widget));
  const savedDrill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const scope = useMemo(() => ({}), [runtime.client, track?.source.id, track?.source.revision, track?.branch.id]);
  const [choice, setPair] = useState<{ scope: object; pair: ChosenPair } | null>(null);
  const pair = choice?.scope === scope ? choice.pair : null;
  const drill = pair ? savedDrill : CLOSED;
  const drillRegion = drillControl.region;
  const drillAt = useRegionHour(runtime, drillRegion, active && drill.breakdown !== null);
  const pairLabels = useIntelLabels(runtime, widget, pair ? [pair.owner, pair.target] : NO_TAGS, pair?.hour ?? null, active);
  const headRoot = head.root;
  const root = useCallback((element: HTMLDivElement | null) => { anchor.current = element; headRoot(element); }, [headRoot]);
  useDiagnostics(onDiagnostics, data?.warnings ?? null);
  if (!track) return <WidgetState kind="empty" icon="eye" message="Load a campaign to see intel networks." />;
  if (saved) return head.outside ? <WidgetState kind="outside" read={head} /> : <PairHistory {...props} pair={null} gate={gate} hour={gate.hour} />;
  const main = head.read === 'off' ? <WidgetState kind="outside" />
    : head.error && !data ? <WidgetState kind="error" failure={head.error} onRetry={head.retry} />
    : filtered && groups.error && !groups.data ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />
    : selection?.reason ? <WidgetState kind="unavailable" message={selection.reason} />
    : chosen?.length === 0 ? <WidgetState kind="unavailable" message="Choose a country in the widget settings." />
    : !data || selection && !selection.ready ? <WidgetState kind="loading" label="Loading intel networks…" />
    : data.missing || !at || !ordered ? <WidgetState kind="unavailable" message="Intel networks were not recorded at this time." detail={data.missing ?? undefined} />
    : touching && axisTags.length === 1 ? <WidgetState kind="empty" message="No intel networks of or in this country at this date." />
    : axisTags.length < 2 ? <WidgetState kind="empty" message="Fewer than two countries are shown at this date." />
    : !data.networks.length ? <WidgetState kind="empty" message="No intel networks at this date." />
    : <HeatMatrix label="Intel network coverage" rows={axis} columns={axis} cell={cell} live={cells} revision={`${field}:${axisTags.join(',')}`} active={active}
      onVisible={onVisible} tooltip={tooltip} onSelect={(row, column) => {
        if (row === column) return;
        const targetId = data.networks.find(network => network.owner.tag === axisTags[row] && network.target.tag === axisTags[column])?.target.id
          ?? at.ids.get(axisTags[column]);
        if (targetId === undefined) return;
        setPair({ scope, pair: { owner: axisTags[row], target: axisTags[column], targetId, hour } });
        dispatch({ type: 'open', breakdown: PAIR.id, hour, follow: drillControl.followsAt(hour) });
      }} menu={() => readHour.current === null ? [] : [jumpItem(runtime, track.id, readHour.current)]} />;
  return <div ref={root} className="intel-coverage">
    <DrillHost drill={drillControl} runtime={runtime} subject={drill.breakdown !== null} anchor={anchor} mode={options.drill === 'popout' ? 'popout' : 'inline'}
      title={pair ? `${pairLabels(pair.owner).name} → ${pairLabels(pair.target).name}` : PAIR.label}
      day={drillAt === null ? '' : dayLabel(drillAt)} canFollow
      onClose={() => dispatch({ type: 'close' })} main={main}
      pinRefused={drillControl.pinRefused} onPin={pair && drillControl.time() ? () => {
        const time = drillControl.time();
        if (!time) return;
        const pinned = pinnedBreakdown(widget, PAIR, track.id, time);
        const saved = pairOptions(pair);
        pinned.options = { ...pinned.options, owner: setToken(saved.owner), target: setToken(saved.target) };
        runtime.addWidgetBelow(widget.id, pinned);
      } : undefined}>
      {pair && <PairHistory {...props} pair={pair} gate={drillControl.gate} hour={drillControl.hour} />}
    </DrillHost>
  </div>;
}
