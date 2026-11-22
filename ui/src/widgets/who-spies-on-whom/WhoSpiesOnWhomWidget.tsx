import { useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import ChordChart from '../../charts/chord/ChordChart';
import type { ChordFrame } from '../../charts/chord/ChordChart';
import type { ChordData } from '../../charts/chord/layout';
import { motionOf } from '../../charts/easing.ts';
import { jumpItem } from '../../charts/jump';
import { LiveCell } from '../../charts/live.ts';
import { useCountryColours } from '../../charts/race/countryColours';
import { useChartTokens } from '../../charts/useChartTokens';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { worldFilter } from '../../dashboard/groups';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { shownHour, useIntelLabels } from '../../dashboard/IntelCountry';
import { usePushedLooksHour } from '../../dashboard/pushedLooks.ts';
import { tagsOf, validatePlacements } from '../../dashboard/intelligence';
import { resolveOptions } from '../../dashboard/options';
import { asFailure } from '../../dashboard/queryError.ts';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { OPTIONS } from './definition';
import { placementChord, placementRows, placementStructure, placementTooltip } from './placementsData';
import type { PlacementChord, PlacementLook } from './placementsData';
import './placements.css';
import type { Country, OperativePlacementReply } from '../../generated/dashboard';

const NO_TAGS: readonly string[] = [];

export default function WhoSpiesOnWhomWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const tokens = useChartTokens();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const params = useMemo(() => track ? {} : null, [track]);
  const live = useMemo(() => new LiveCell<ChordFrame>({ data: null }), []);
  const paint = useRef<(reply: OperativePlacementReply) => void>(() => {});
  const head = useHeadProduct(runtime, useGate(runtime, widget), 'operative-placements', params,
    { active, validate: validatePlacements, structure: placementStructure, paint: reply => paint.current(reply) });
  const hour = shownHour(runtime, widget, head);
  const groups = useCountryGroups(runtime, track, active);
  const fallback = defaultCountryKey(track);
  const selection = widget.countries;
  const top = selection.kind === 'group' && selection.group.type === 'top' ? selection.group.count : null;
  const filter = useMemo(() => top !== null || selection.kind === 'all' ? null
    : worldFilter(selection, fallback ? [fallback] : [], groups.data, hour ?? 0, track?.branch.lastHour ?? 0),
  [top, selection, fallback, groups.data, hour, track?.branch.lastHour]);
  const most = top ?? Number(options.mostCountries);
  const tags = useMemo(() => head.data ? tagsOf(head.data.placements.flatMap(row => [row.owner, row.target])) : NO_TAGS, [head.data]);
  const pushed = head.read === 'followed';
  const label = useIntelLabels(runtime, widget, tags, hour, active, pushed);
  const named = usePushedLooksHour(runtime, widget, track, tags, active && pushed, hour ?? 0);
  const looks = useCountryLooks(runtime.client, track, tags, named.hour, active, undefined, undefined, named.exact);
  const colour = useCountryColours(tags, looks.color, tokens, looks);
  const exact = useMemo(() => new WeakMap<ChordData, PlacementChord>(), []);
  const segment = `${track?.source.id}:${track?.source.revision}:${track?.branch.id}`;
  paint.current = reply => {
    if (reply.missing || (filter && (filter.reason || !filter.tags))) { live.set({ data: null, segment }); return; }
    const look = (country: Country): PlacementLook => country.tag ? label(country.tag)
      : { name: `Country ${country.id}`, flagReason: 'The recording names no tag for this country at this hour' };
    const chord = placementChord(placementRows(reply.placements, filter?.tags ?? null), most, look,
      country => country.tag ? colour(country.tag) : tokens.muted, tokens.muted);
    exact.set(chord.data, chord);
    live.set({ data: chord.data, segment, playing: runtime.clock.getSnapshot().playing });
  };
  useLayoutEffect(() => { if (head.data) paint.current(head.data); }, [head.data, label, colour, most, filter, segment]);
  useShownEachFrame(runtime.clock, () => head.data ? head.hour ?? runtime.clock.getHour() : null);
  const [chartError, setChartError] = useState<string | null>(null);
  const menu = useContextMenu('Operative placement actions');
  useDiagnostics(onDiagnostics, head.data?.warnings ?? null);
  const body = () => {
    if (!track) return <WidgetState kind="empty" icon="target" message="Load a campaign to see its operative placements." />;
    if (head.read === 'off') return <WidgetState kind="outside" />;
    if (head.error) return <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
    if (groups.error && !groups.data && selection.kind === 'group') return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
    if (!head.data) return <WidgetState kind="loading" label="Loading operative placements…" />;
    if (head.data.missing) return <WidgetState kind="unavailable" message="Operatives were not recorded at this time." detail={head.data.missing} />;
    if (filter?.reason) return <WidgetState kind="empty" message={filter.reason} />;
    const shown = placementRows(head.data.placements, filter?.tags ?? null);
    return <>
      <ChordChart label="Who spies on whom" caption="Ribbon colour = the country spying" frame={{ data: null }} live={live} onError={setChartError}
        hoursPerSecond={motionOf(runtime.clock.getSnapshot())} active={active}
        tooltip={(hit, data) => { const chord = exact.get(data); return chord ? placementTooltip(hit, chord) : null; }}
        onContextMenu={(hit, data, x, y) => {
          const chord = exact.get(data);
          const read = head.data?.readHour;
          if (chord && read != null && (hit.kind === 'flow' ? chord.flows.has(hit.id) : chord.nodes.has(hit.id))) menu.open(x, y, [jumpItem(runtime, track.id, read)]);
        }} />
      {menu.menu}
      {chartError && <WidgetState kind="error" failure={asFailure(chartError)} />}
      {!shown.length && <WidgetState kind="empty" message="No operatives placed yet." />}
    </>;
  };
  return <div ref={head.root} className={`who-spies-on-whom${chartError ? ' has-chart-error' : ''}`}>{body()}</div>;
}
