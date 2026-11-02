import { contentTitle } from '../../dashboard/contentTitles.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import { PreparedKey } from '../../charts/PreparedKey';
import { overlapsSpans } from '../../charts/prepared';
import { bandMaxOf, runState, stretchAt } from '../../charts/timeline/runState';
import { laneRequests } from '../../charts/timeline/laneRequests';
import { Lineage } from '../../dashboard/identity';
import { runAt } from '../../charts/series';
import Timeline from '../../charts/timeline/Timeline';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { mix } from '../../shell/appearance/color';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { countryIdentityIndex } from '../../dashboard/countryIdentity';
import type { WidgetProps } from '../../dashboard/definitions';
import { groupsKnownAt, MEMBERSHIP_PREPARING, worldFilter } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { pendingAt, preparedPrefix, unobservedOrPending } from '../../dashboard/preparedPrefix';
import { useParityRows } from '../../dashboard/parityRows';
import { useCountriesAt } from '../../dashboard/useCountriesAt';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useHoverLabels } from '../../dashboard/useHoverLabels';
import type { SubjectLevel, SubjectReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { laneCountries, levelAt, levelColours, levelName, overlordTagAt, overlordViews, runNear, subjectLanes, subjectTagAt, validateSubjectReply } from './subjectsData';
import type { OverlordOrder, SubjectOrder } from './subjectsData';
import './subjects.css';
import { motionOf } from '../../charts/easing.ts';

const NO_WARNINGS: string[] = [];

/** Every overlord and its subjects over time: each
 * overlord heads a band as thick as its number of subjects, and each subject has a
 * bar from when it became one to when it stopped, coloured by its level. Levels and
 * their names come from the campaign's content; colours follow their order. */
export default function SubjectsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const playbackHour = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'subjects', params: { branchId: track.branch.id } } : null;
  const product = useProduct<SubjectReply>(runtime.client, request, active, validateSubjectReply, track?.source);
  const data = product.data;
  const root = useRef<HTMLDivElement>(null);
  // The native catalogue identifies both relation endpoints and dated aliases.
  const groups = useCountryGroups(runtime, track, active);
  useParityRows(root, () => ({ kind: 'chart', label: 'Subject relations and levels', complete: !!data && !!groups.data,
    total: data?.runs.length ?? 0, rows: data?.runs ?? [], levels: data?.levels, unobserved: data?.unobserved }));
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const first = widget.time.mode === 'range' && window ? window.firstHour : track?.branch.firstHour ?? 0;
  const last = widget.time.mode === 'range' && window ? window.lastHour : track?.branch.lastHour ?? 0;
  const at = Math.floor(playhead ?? last);
  const extent: [number, number] = [first, Math.max(first + 1, options.grow && playhead != null ? playhead : last + 1)];
  const identityChanges = groups.data?.identityChanges;
  const identities = useMemo(() => groups.data ? countryIdentityIndex(groups.data.countryIdentities) : null, [groups.data]);
  // Before preparation covers the branch, the prefix's relations, then each later
  // block's keyframe (approximate); hours still being prepared have no known
  // relations, so a relation reaching them is not an ending.
  const prefix = useMemo(() => preparedPrefix(data), [data]);
  const approximate = data?.approximate;
  const known = useMemo(() => data && prefix ? { ...data, unobserved: unobservedOrPending(data.unobserved, prefix) } : data, [data, prefix]);
  const views = useMemo(() => known && identities ? overlordViews(known, new Lineage(identityChanges ?? []), identities) : [], [known, identityChanges, identities]);
  const levels = useMemo(() => new Map<string, SubjectLevel>((data?.levels ?? []).map(level => [level.token, level])), [data]);
  // The categorical palette in the content's level order; levels past it share one colour.
  const other = mix(tokens.surface, tokens.text, 0.35);
  const colours = useMemo(() => levelColours(data?.levels ?? [], tokens.categories, other), [data, tokens.categories, other]);
  const dashboardTag = track ? defaultCountryTag(track) : null;
  const lastHour = track?.branch.lastHour ?? 0;
  const filter = useMemo(() => worldFilter(widget.countries, dashboardTag ? [dashboardTag] : [], groups.data, at, lastHour),
    [widget.countries, dashboardTag, groups.data, at, lastHour]);
  const [inView, setInView] = useState<[number, number]>([0, 24]);
  const [named, setNamed] = useState<string[]>([]);
  const [sortNames, setSortNames] = useState<string[]>([]);
  const identityHistories = useMemo(() => new Map(groups.data?.countryIdentities.map(history => [history.tag, history.identities]) ?? []), [groups.data]);
  const segmentAt = useCallback((tag: string, hour: number) => runAt(identityHistories.get(tag) ?? [], hour)?.firstHour ?? null, [identityHistories]);
  const { country, revision } = useCountriesAt(runtime.client, track, named, at, active, sortNames, segmentAt);
  // A tooltip names its countries as recorded at the hour hovered, asking only for those.
  const hover = useHoverLabels(runtime.client, track, active, country);
  const unobserved = views[0]?.unknown ?? known?.unobserved;
  // An overlord's band is neutral: its subjects carry the colours.
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
    const warnings = [...(data?.warnings ?? []), ...(groups.data?.warnings ?? []),
      ...(views[0]?.identityGaps.length || data?.runs.length && identities && !views.length ? ['Some subject relations have unknown countries; those hours stay unknown.'] : []),
      ...(groups.error ? [`Country history unavailable: ${groups.error}`] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groups.data, groups.error, identities, views, onDiagnostics]);

  return <div ref={root} style={{ display: 'contents' }}>{(() => {
  if (!track) return <WidgetState kind="empty" icon="hierarchy" message="Load a campaign to see its overlords and subjects." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (product.error) return <WidgetState kind="error" failure={product.error} onRetry={product.retry} />;
  const groupFailure = groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (!data || !identities) return <>{groupFailure}<WidgetState kind="loading" label="Loading subjects…" /></>;
  if (filter.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={filter.reason} /></>;
  if (!data.runs.length && prefix?.pending.length) return <>{groupFailure}<WidgetState kind="loading" label="This range is still being prepared." /></>;
  if (!data.runs.length) return <>{groupFailure}<WidgetState kind="empty" message={data.warnings.length ? 'Subject relations are unavailable.' : 'No subjects recorded.'} /></>;
  // Countries and group members at hours still being prepared are not known yet: no lane is not proof of none.
  if (!lanes.length && (!filter.known || !groupsKnownAt(groups.data, at))) return groupFailure ?? <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  if (!views.length || !lanes.length && views[0].identityGaps.some(span => span.firstHour <= at)) return <>{groupFailure}<WidgetState kind="unavailable" message="Countries in these relations could not be identified." /></>;
  if (!lanes.length) return <>{groupFailure}<WidgetState kind="empty" message={filter.tags ? 'None of the chosen countries has been an overlord or a subject yet.' : 'No subjects yet.'} /></>;

  // The levels shown, in the content's order, each named once; past the palette they share "Other".
  const shownLevels = new Set(lanes.flatMap(lane => lane.depth ? info.get(lane.id)?.subject?.runs.flatMap(run => run.steps.map(step => step.level)) ?? [] : []));
  const legendLevels = data.levels.filter(level => shownLevels.has(level.token));
  const own = legendLevels.filter(level => colours.get(level.token) !== other);
  const folded = legendLevels.filter(level => colours.get(level.token) === other);
  const legend = [...own.map(level => ({ id: level.token, label: contentTitle(level.name, level.token, 'Autonomy level name unavailable'), color: colours.get(level.token)! })),
    ...(folded.length ? [{ id: '\u0000other', label: 'Other levels', color: other, tooltip: folded.map(level => contentTitle(level.name, level.token, 'Autonomy level name unavailable')).join(', ') }] : [])];
  const day = dayLabel;
  const tooltip = (laneId: string, hour: number, full: boolean): TooltipContent | null => {
    const about = info.get(laneId);
    if (!about) return null;
    const named = hover.labelAt(hour);
    const unknown = (unobserved ?? []).some(span => span.firstHour <= hour && hour <= span.lastHour);
    const preparing = pendingAt(prefix, hour);
    // Sampled at its block's keyframe: exact there, approximate over the block.
    const sampled = overlapsSpans(approximate, hour, hour) ? ' · approximate' : '';
    if (!about.subject) {
      const overlord = about.overlord;
      const overlordTag = overlordTagAt(overlord, hour);
      const held = unknown ? [] : overlord.subjects.flatMap(view => {
        const run = view.runs.find(item => item.firstHour <= hour && hour <= item.lastHour);
        const tag = subjectTagAt(overlord, view, hour);
        return run && tag !== null ? [{ tag, level: levelAt(run, hour) ?? null }] : [];
      });
      hover.ask(hour, [...(overlordTag === null ? [] : [overlordTag]), ...held.map(item => item.tag)]);
      held.sort((a, b) => named(a.tag).name.localeCompare(named(b.tag).name));
      const rows: TooltipRow[] = [{ label: 'Subjects', value: preparing ? 'Still being prepared' : unknown ? 'Not recorded' : String(held.length) },
        ...held.map((item, index) => ({ label: named(item.tag).name, countryFlag: named(item.tag), value: levelName(levels, item.level),
          swatch: item.level != null ? colours.get(item.level) : undefined, indent: true, short: true, divider: index === 0 }))];
      const label = overlordTag === null ? { name: 'Unknown country', flagReason: 'Unknown country at this hour.' } : named(overlordTag);
      const content: TooltipContent = { title: label.name, countryFlag: label, subtitle: `Overlord · ${day(hour)}${sampled}`, rows };
      return full ? content : fitTooltip([content], viewport, { more: hidden => `${hidden} more ${hidden === 1 ? 'subject' : 'subjects'}` });
    }
    const view = about.subject;
    const run = runNear(view, hour);
    // Named by the tags their countries had at the hour, whatever the nearest run says.
    const subjectTag = subjectTagAt(about.overlord, view, hour);
    const overlordTag = overlordTagAt(about.overlord, hour);
    hover.ask(hour, [subjectTag, overlordTag].filter((tag): tag is string => tag !== null));
    const subjectLabel = subjectTag === null ? { name: 'Unknown country', flagReason: 'Unknown country at this hour.' } : named(subjectTag);
    const overlordLabel = overlordTag === null ? { name: 'Unknown country', flagReason: 'Unknown country at this hour.' } : named(overlordTag);
    const holding = run.firstHour <= hour && hour <= run.lastHour;
    const level = holding ? levelAt(run, hour) ?? null : null;
    // Ended as its relation says: runs of one relation joined across unrecorded hours.
    const stretch = stretchAt(view.stretches, hour) ?? { firstHour: run.firstHour, lastHour: run.lastHour };
    const state = runState([stretch], at, unobserved);
    const rows: TooltipRow[] = [
      { label: 'Overlord', value: overlordLabel.name, countryFlag: overlordLabel },
      // An unrecorded hour is unknown, not an hour it was no subject.
      { label: 'Level', value: holding ? levelName(levels, level) : preparing ? 'Still being prepared' : unknown ? 'Not recorded' : 'Not a subject then',
        swatch: level != null ? colours.get(level) : undefined, tone: holding && level != null ? undefined : 'muted' },
      { label: 'Subject since', value: day(run.startHour) },
      // An end inside sampled hours is known only to the block it was sampled in.
      { label: 'Ended', value: state.kind === 'ended' ? (overlapsSpans(approximate, stretch.lastHour, stretch.lastHour + 1) ? `around ${day(state.at)} (approximate)`
          : state.lastSeen == null ? day(state.at) : `by ${day(state.at)}`)
        : state.kind === 'unknown' ? (pendingAt(prefix, stretch.lastHour + 1) ? 'Still being prepared' : `not recorded after ${day(state.lastSeen)}`) : 'Still a subject' },
    ];
    return { title: subjectLabel.name, countryFlag: subjectLabel, subtitle: `Subject · ${day(hour)}${sampled}`, rows };
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
  return <div className="subjects-widget">{groupFailure}
    {/* Shown for one level too: its colour is what the bars mean. */}
    {options.legend === true && <Legend label="Subject levels" items={legend} always />}
    <Timeline key={`${track.source.id}:${track.source.revision}:${track.branch.id}`} label="Overlords and subjects" extent={extent} playhead={playhead} labelHour={at} preserveZoomOnExtentChange={options.grow === true} lanes={lanes} showIcons={false} showNames={options.names === true}
      description={description} unobserved={unobserved} approximate={approximate} pending={prefix?.pending} bandMax={bandMax} hoursPerSecond={motionOf(runtime.clock.getSnapshot())}
      active={active} revision={`${revision}:${hover.revision}:${colours.size}`} onLanesInView={onLanesInView}
      tooltip={(lane, _span, hour, full) => tooltip(lane.id, hour, full === true)}
      onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    <PreparedKey approximate={approximate} pending={prefix?.pending} />
    {menu.menu}
  </div>;
  })()}</div>;
}
