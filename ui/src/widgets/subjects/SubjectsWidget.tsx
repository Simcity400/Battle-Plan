import { useCallback, useEffect, useMemo, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import { bandMaxOf, runState, stretchAt } from '../../charts/timeline/runState';
import { laneRequests } from '../../charts/timeline/laneRequests';
import { Lineage } from '../../dashboard/identity';
import Timeline from '../../charts/timeline/Timeline';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { mix } from '../../shell/appearance/color';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { worldFilter } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountriesAt } from '../../dashboard/useCountriesAt';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useHoverLabels } from '../../dashboard/useHoverLabels';
import type { SubjectLevel, SubjectReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { laneCountries, levelAt, levelColours, levelName, overlordTagAt, overlordViews, runNear, subjectLanes, subjectTagAt, validateSubjectReply } from './subjectsData';
import type { OverlordOrder, SubjectOrder } from './subjectsData';
import './subjects.css';

const NO_WARNINGS: string[] = [];
const NOTE = 'Subject relations are recorded every hour; the date a country became a subject is the game’s own.';

/** Every overlord and its subjects over time (mock-up style-4's timeline): each
 * overlord heads a band as thick as its number of subjects, and each subject has a
 * bar from when it became one to when it stopped, coloured by its level. Levels and
 * their names come from the campaign's content; colours follow their order. */
export default function SubjectsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'subjects', params: { branchId: track.branch.id } } : null;
  const product = useProduct<SubjectReply>(runtime.client, request, active, validateSubjectReply);
  const data = product.data;
  // Group history: the recorded changes of tag (a subject or overlord renamed is one
  // country), and a chosen group's countries.
  const groups = useCountryGroups(runtime, track, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const at = Math.floor(playhead ?? extent[1]);
  const identityChanges = groups.data?.identityChanges;
  const views = useMemo(() => data ? overlordViews(data, new Lineage(identityChanges ?? [])) : [], [data, identityChanges]);
  const levels = useMemo(() => new Map<string, SubjectLevel>((data?.levels ?? []).map(level => [level.token, level])), [data]);
  // The categorical palette in the content's level order; levels past it share one colour.
  const other = mix(tokens.surface, tokens.text, 0.35);
  const colours = useMemo(() => levelColours(data?.levels ?? [], tokens.categories, other), [data, tokens.categories, other]);
  const dashboardTag = track ? defaultCountryTag(track) : null;
  const lastHour = track?.branch.lastHour ?? 0;
  const filter = useMemo(() => worldFilter(widget.countries, dashboardTag ? [dashboardTag] : [], groups.data, at, lastHour, views[0]?.lineage),
    [widget.countries, dashboardTag, groups.data, at, lastHour, views]);
  const [inView, setInView] = useState<[number, number]>([0, 24]);
  const [named, setNamed] = useState<string[]>([]);
  const [sortNames, setSortNames] = useState<string[]>([]);
  const { country, revision } = useCountriesAt(runtime.client, track, named, at, active, sortNames);
  // A tooltip names its countries as recorded at the hour hovered, asking only for those.
  const hover = useHoverLabels(runtime.client, track, active, country);
  const unobserved = data?.unobserved;
  // An overlord's band is neutral, as the mock-up's war bars: its subjects carry the colours.
  const bandColor = mix(tokens.text, tokens.surface, 0.45);
  const { lanes, info } = useMemo(() => subjectLanes(views, levels, colours, at, {
    order: options.order as OverlordOrder, subjects: options.subjects as SubjectOrder, ended: options.ended === 'shown', countries: filter.tags, countryIds: filter.countryIds, bandColor,
  }, { country, day: dayLabel }, unobserved ?? []), [views, levels, colours, at, options.order, options.subjects, options.ended, filter.tags, filter.countryIds, country, unobserved, bandColor]);
  const requests = laneRequests(lanes, inView);
  const wantedKey = JSON.stringify(laneCountries(requests.looks, info, at));
  const sortNamesKey = JSON.stringify(laneCountries(requests.names, info, at));
  useEffect(() => { setNamed(JSON.parse(wantedKey) as string[]); }, [wantedKey]);
  useEffect(() => { setSortNames(JSON.parse(sortNamesKey) as string[]); }, [sortNamesKey]);
  const onLanesInView = useCallback((first: number, last: number) => setInView([first, last]), []);
  const menu = useContextMenu('Timeline actions');
  const bandMax = useMemo(() => bandMaxOf(views.map(view => view.counts)), [views]);

  useEffect(() => {
    // Without the group history a change of tag cannot be told from a release: say so.
    const warnings = [...(data?.warnings ?? []), ...(groups.error ? [`Changes of tag could not be read, so a renamed country may show as released and taken again: ${groups.error}`] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groups.error, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon="hierarchy" message="Load a campaign to see its overlords and subjects." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
  if (playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <WidgetState kind="unavailable" message="This time is outside the campaign’s recorded branch." />;
  if (product.error) return <WidgetState kind="error" message={product.error} onRetry={product.retry} />;
  if (groups.error && widget.countries.kind === 'group') return <WidgetState kind="error" message={groups.error} onRetry={groups.retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading subjects…" />;
  if (filter.reason) return <WidgetState kind="unavailable" message={filter.reason} />;
  if (!data.runs.length) return <WidgetState kind="empty" message={data.warnings.length ? 'No subject relations could be read from this campaign branch.' : 'No subjects were recorded in this campaign branch.'} />;
  if (!lanes.length) return <WidgetState kind="empty" message={filter.tags ? 'None of the chosen countries has been an overlord or a subject yet.' : 'No subjects yet.'} />;

  // The levels shown, in the content's order, each named once; past the palette they share "Other".
  const shownLevels = new Set(lanes.flatMap(lane => lane.depth ? info.get(lane.id)?.subject?.runs.flatMap(run => run.steps.map(step => step.level)) ?? [] : []));
  const legendLevels = data.levels.filter(level => shownLevels.has(level.token));
  const own = legendLevels.filter(level => colours.get(level.token) !== other);
  const folded = legendLevels.filter(level => colours.get(level.token) === other);
  const legend = [...own.map(level => ({ id: level.token, label: level.name ?? level.token, color: colours.get(level.token)! })),
    ...(folded.length ? [{ id: '\u0000other', label: `Other levels (${folded.map(level => level.name ?? level.token).join(', ')})`, color: other }] : [])];
  const day = dayLabel;
  const tooltip = (laneId: string, hour: number, full: boolean): TooltipContent | null => {
    const about = info.get(laneId);
    if (!about) return null;
    const named = hover.labelAt(hour);
    const unknown = data.unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour);
    if (!about.subject) {
      const overlord = about.overlord;
      const overlordTag = overlordTagAt(overlord, hour);
      const held = unknown ? [] : overlord.subjects.flatMap(view => {
        const run = view.runs.find(item => item.firstHour <= hour && hour <= item.lastHour);
        return run ? [{ tag: run.subject, level: levelAt(run, hour) ?? null }] : [];
      });
      hover.ask(hour, [overlordTag, ...held.map(item => item.tag)]);
      held.sort((a, b) => named(a.tag).name.localeCompare(named(b.tag).name));
      const rows: TooltipRow[] = [{ label: 'Subjects', value: unknown ? 'Not recorded' : String(held.length) },
        ...held.map((item, index) => ({ label: named(item.tag).name, icon: named(item.tag).flag, value: levelName(levels, item.level),
          swatch: item.level != null ? colours.get(item.level) : undefined, indent: true, short: true, divider: index === 0 }))];
      const content: TooltipContent = { title: named(overlordTag).name, icon: named(overlordTag).flag, subtitle: `Overlord · ${day(hour)}`, rows, note: NOTE };
      return full ? content : fitTooltip([content], viewport, { more: hidden => `${hidden} more ${hidden === 1 ? 'subject' : 'subjects'}` });
    }
    const view = about.subject;
    const run = runNear(view, hour);
    // Named by the tags their countries had at the hour, whatever the nearest run says.
    const subjectTag = subjectTagAt(about.overlord, view, hour);
    const overlordTag = overlordTagAt(about.overlord, hour);
    hover.ask(hour, [subjectTag, overlordTag]);
    const holding = run.firstHour <= hour && hour <= run.lastHour;
    const level = holding ? levelAt(run, hour) ?? null : null;
    // Ended as its relation says: runs of one relation joined across unrecorded hours.
    const stretch = stretchAt(view.stretches, hour) ?? { firstHour: run.firstHour, lastHour: run.lastHour };
    const state = runState([stretch], at, data.unobserved);
    const rows: TooltipRow[] = [
      { label: 'Overlord', value: named(overlordTag).name, icon: named(overlordTag).flag },
      // An unrecorded hour is unknown, not an hour it was no subject.
      { label: 'Level', value: holding ? levelName(levels, level) : unknown ? 'Not recorded' : 'Not a subject then',
        swatch: level != null ? colours.get(level) : undefined, tone: holding && level != null ? undefined : 'muted' },
      { label: 'Subject since', value: day(run.startHour) },
      { label: 'Ended', value: state.kind === 'ended' ? (state.lastSeen == null ? day(state.at) : `by ${day(state.at)}`)
        : state.kind === 'unknown' ? `not recorded after ${day(state.lastSeen)}` : 'Still a subject' },
    ];
    return { title: named(subjectTag).name, icon: named(subjectTag).flag, subtitle: `Subject · ${day(hour)}`, rows, note: NOTE };
  };
  // What the timeline shows at the moment shown, for keyboard and screen-reader users:
  // each overlord and the subjects listed after it, in one pass.
  const groupsSaid: { overlord: string; subjects: string[] }[] = [];
  for (const lane of lanes) {
    const said = `${lane.identity?.label}, ${lane.label}`;
    if (!lane.depth) groupsSaid.push({ overlord: said, subjects: [] });
    else groupsSaid.at(-1)?.subjects.push(said);
  }
  const description = `${dayLabel(at)}: ${groupsSaid.map(group => group.subjects.length ? `${group.overlord}: ${group.subjects.join('; ')}` : group.overlord).join('. ')}.`;
  return <div className="subjects-widget">
    {/* Shown for one level too: its colour is what the bars mean. */}
    {options.legend === true && <Legend label="Subject levels" items={legend} always />}
    <Timeline label="Overlords and subjects" extent={extent} playhead={playhead} lanes={lanes} showIcons={false} showNames={options.names === true}
      description={description} unobserved={data.unobserved} bandMax={bandMax} hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond}
      active={active} revision={`${revision}:${hover.revision}:${colours.size}`} onLanesInView={onLanesInView}
      tooltip={(lane, _span, hour, full) => tooltip(lane.id, hour, full === true)}
      onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    {menu.menu}
  </div>;
}
