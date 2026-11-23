import { useEffect, useMemo, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import { countryNameDetails, fitTooltip } from '../../charts/tooltip.ts';
import { hideTooltip, showTooltip } from '../../charts/tooltip.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { readingValues, type TableReading } from '../../draw/tables/surface.ts';
import { useContextMenu } from '../../components/ContextMenu';
import { jumpItem } from '../../charts/jump';
import { useViewportHeight } from '../../charts/useChartTokens';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch, countryMatchChangeHours, membershipUnrecordedAt } from '../../dashboard/groups';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useHeldTimeline } from '../../dashboard/countryNamesAt';
import { useContentRevision } from '../../dashboard/contentRevision';
import { lookStarts } from '../../dashboard/countryTimeline';
import { resolveOptions } from '../../dashboard/options';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useShownHour, useWhyCommit } from '../../dashboard/widgetPerf.ts';
import { OPTIONS } from './definition';
import { occupationTooltip, occupiedStructure, validateOccupiedStates, type CountryRole, type OccupiedDetail } from './occupiedStatesData.ts';
import { lookAt } from '../../draw/tables/looks.ts';
import type { OccupiedFilter } from './occupiedTable';
import OccupiedTableSurface from './OccupiedTableSurface.tsx';
import './occupiedStates.css';

const NO_WARNINGS: string[] = [];
const NO_TAGS: readonly string[] = [];
const PARAMS = {} as const;
const EVERY_COUNTRY: OccupiedFilter = { passing: null, unrecorded: [], bounds: [] };
const filters = new WeakMap<object, Map<string, OccupiedFilter>>();

function filterTimeline(tags: readonly string[], matches: (tag: string, hour: number) => boolean, unrecordedAt: (hour: number) => string | null,
  changes: readonly number[], first: number, last: number): OccupiedFilter {
  const bounds = [...new Set([first, ...changes.filter(at => at > first && at <= last), last + 1])].sort((a, b) => a - b);
  const passing: Record<string, [number, number][]> = {}, unrecorded: [number, number, string][] = [];
  for (let at = 0; at + 1 < bounds.length; at++) {
    const from = bounds[at]!, to = bounds[at + 1]! - 1;
    for (const tag of tags) if (matches(tag, from)) {
      const ranges = passing[tag] ??= [], previous = ranges.at(-1);
      if (previous && previous[1] === from - 1) previous[1] = to; else ranges.push([from, to]);
    }
    const reason = unrecordedAt(from), previous = unrecorded.at(-1);
    if (reason !== null) { if (previous && previous[1] === from - 1 && previous[2] === reason) previous[1] = to; else unrecorded.push([from, to, reason]); }
  }
  return { passing, unrecorded, bounds };
}

export default function OccupiedStatesWidget({ widget, runtime, active, onDiagnostics, setOption, headerSlot }: WidgetProps) {
  const viewportHeight = useViewportHeight();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const role: CountryRole = options.countryRole === 'occupied' ? 'occupied' : 'occupier';
  const head = useHeadProduct(runtime, widget, 'occupied-states', track ? PARAMS : null, { active, validate: validateOccupiedStates,
    structure: occupiedStructure });
  const data = head.data;
  const shown = useShownHour();
  const shownAt = useRef(head.shownAt);
  shownAt.current = head.shownAt;
  useEffect(() => { if (head.data) shown(shownAt.current()); });
  const hour = head.at ?? track?.branch.lastHour ?? 0;
  const groups = useCountryGroups(runtime, track, active);
  const tagsText = useMemo(() => (groups.data?.countryIdentities ?? []).map(history => history.tag).sort().join(','), [groups.data]);
  const tags = useMemo(() => tagsText ? tagsText.split(',') : NO_TAGS, [tagsText]);
  const named = tags;
  const selected = selectedCountryKeys(widget.countries, track);
  const selectedKey = JSON.stringify(selected);
  const fallback = useMemo(() => selected, [selectedKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? hour),
    [widget.countries, fallback, groups.data, track?.branch.lastHour, hour]);
  const first = track?.branch.firstHour, last = track?.branch.lastHour;
  const filter = useMemo<OccupiedFilter>(() => {
    if (widget.countries.kind === 'all' || !match.ready || !groups.data || first === undefined || last === undefined) return EVERY_COUNTRY;
    const key = JSON.stringify([widget.countries, fallback, first, last]);
    let held = filters.get(groups.data);
    if (!held) filters.set(groups.data, held = new Map());
    let made = held.get(key);
    if (!made) held.set(key, made = filterTimeline(tags, match.matches, at => membershipUnrecordedAt(widget.countries, groups.data, at),
      countryMatchChangeHours(match.matches) ?? [], first, last));
    return made;
  }, [widget.countries, match, groups.data, fallback, tags, first, last]);
  const timeline = useHeldTimeline(runtime.client, track, useContentRevision(runtime.client, active), active);
  const startsText = JSON.stringify(named.map(tag => [tag, lookStarts(timeline, tag, first ?? hour, hour)]));
  const starts = useMemo(() => JSON.parse(startsText) as [string, [number, boolean][]][], [startsText]);
  const wanted = useMemo(() => starts.flatMap(([tag, hours]) => hours.flatMap(([at, held]) => held ? [{ hour: at, tag }] : [])), [starts]);
  const { labelAt, exactAt } = useCountryLabelsAt(runtime.client, track, wanted, active, Infinity, true);
  const labelsText = JSON.stringify(starts.map(([tag, hours]) => [tag, hours.map(([at, held]) => {
    const look = held ? exactAt(at)(tag) : undefined;
    return [at, look?.known ? look.name : null, look?.flag ?? null, look && !look.known ? look.name : null];
  })]));
  const labels = useMemo(() => Object.fromEntries((JSON.parse(labelsText) as [string, [number, string | null, string | null, string | null][]][])
    .map(([tag, runs]) => [tag, runs.map(([from, name, flag, status]) => ({ from, name, flag: flag ?? undefined, status: status ?? undefined }))])), [labelsText]);
  const label = labelAt(hour);
  const looks = useCountryLooks(runtime.client, track, named, hour, active);
  const countryColours = useMemo(() => new Map(named.flatMap(tag => {
    const colour = looks.color(tag, hour);
    return colour ? [[tag, `rgb(${colour[0]}, ${colour[1]}, ${colour[2]})`] as const] : [];
  })), [named, looks, hour]);
  const context = useMemo(() => ({ filter, role, labels }), [filter, role, labels]);
  const exactRead = head.exactRead;
  const exactReply = exactRead && match.ready && !match.reason ? async (at: number) => validateOccupiedStates(await exactRead(at)) : undefined;
  const tooltip = (key: string, drawn: TableReading) => {
    const detail = drawn.detail as OccupiedDetail | undefined;
    if (!detail) return null;
    const { value, law } = detail, at = Math.floor(drawn.hour), values = readingValues(drawn);
    const lookOf = (tag: string) => {
      const runs = labels[tag], look = lookAt(runs, at), held = label(tag);
      return runs ? { ...held, name: look?.name ?? look?.status ?? 'Country name unavailable', flag: look?.flag } : held;
    };
    const numbers = { get: (at: string) => { const read = values.get(at); return typeof read === 'number' || typeof read === 'bigint' ? Number(read) : undefined; },
      missing: values.missing };
    const content = occupationTooltip({ key, state: value.state, value }, numbers, tag => lookOf(tag).name, detail.strengthMissing,
      token => contentTitle(law?.token === token ? law.name : undefined, token, 'Occupation law name unavailable'));
    return fitTooltip([{ ...content, subtitle: formatDateTime(at), ...countryNameDetails(value.occupied === null ? null : lookOf(value.occupied)) }], viewportHeight);
  };
  const tooltipOwner = `occupied-table:${widget.id}`;
  useEffect(() => () => hideTooltip(tooltipOwner), [tooltipOwner]);
  const warnings = data?.warnings ?? NO_WARNINGS;
  const contextMenu = useContextMenu('Occupied states actions');
  useWhyCommit('OccupiedStatesWidget', { data, at: head.at, read: head.read, filter, labels, groups: groups.data });
  useEffect(() => { onDiagnostics(warnings); return () => onDiagnostics(NO_WARNINGS); }, [warnings, onDiagnostics]);

  let body;
  if (!track) body = <WidgetState kind="empty" message="Load a campaign to see occupied states." />;
  else if (head.read === 'off') body = <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  else if (head.error && !data) body = <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
  else if (groups.error && !groups.data) body = <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
  else if (match.reason) body = <WidgetState kind="unavailable" message={match.reason} />;
  else if (!data || !match.ready || !groups.data) body = <WidgetState kind="loading" label="Loading occupied states…" />;
  else body = <>
    {data.missing === null && !data.complete && <p className="occupation-note">Some occupied states are missing.</p>}
    {data.missing === null && data.states.some(state => state[role] === null) && widget.countries.kind !== 'all'
      && <p className="occupation-note">Some states couldn’t be matched to these countries.</p>}
    <OccupiedTableSurface widget={widget} runtime={runtime} clock={runtime.clock} data={data} context={context} follow={head.workerFollow} shownAt={head.at ?? hour}
      live={head.live} countryColours={countryColours} onHover={(key, at, drawn) => {
        const content = key !== null && at && drawn ? tooltip(key, drawn) : null;
        if (content && at) showTooltip(tooltipOwner, content, at.x, at.y, key!); else hideTooltip(tooltipOwner);
      }} setOption={setOption} exactReply={exactReply} headerSlot={headerSlot}
      fixed={widget.time.mode === 'follow' || head.hour === null ? undefined : head.hour - (track?.axisOffset ?? 0)} />
  </>;
  return <div className="occupied-states" ref={head.root} onContextMenu={event => {
    if (!track) return;
    event.preventDefault();
    contextMenu.open(event.clientX, event.clientY, [jumpItem(runtime, track.id, hour)]);
  }}>
    {groups.error && groups.data ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null}
    {body}
    {contextMenu.menu}
  </div>;
}
