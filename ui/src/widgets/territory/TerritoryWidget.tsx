import { asFailure } from '../../dashboard/queryError.ts';
import { useCallback, useEffect, useMemo } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import CountryRaceView from '../../charts/race/CountryRaceView';
import FactionRaceView from '../../charts/race/FactionRaceView';
import { raceSeriesIndex, seriesAt, seriesRaceEntries, seriesTooltip } from '../../charts/race/series';
import { runAt } from '../../charts/series';
import { shareColourReason } from '../../charts/sunburst/share';
import { useChartTokens } from '../../charts/useChartTokens';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { countryCapitulationBadges } from '../../dashboard/countryCapitulations';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { useFactionColours } from '../../dashboard/FactionColours';
import { factionNameReasonAt, factionLines, factionMembershipNote, factionNameAt, factionTotalsModel, factionTotalsRequest } from '../../dashboard/factionTotals';
import { territoryMeasureLabel } from '../../dashboard/territoryMeasure';
import type { WidgetProps } from '../../dashboard/definitions';
import { groupLabel, groupShareReason, MEMBERSHIP_PREPARING, noMembersAt, resolveCountries, topCountries } from '../../dashboard/groups';
import NoMembersState from '../../dashboard/NoMembersState';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { answerHold, usePlaybackHour, type HourHold } from '../../dashboard/playback';
import { usePreparedFactionTotals } from '../../dashboard/preparedFactionTotals';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { groupNumbers } from '../../dashboard/groupsReply';
import { NumericReply } from '../../dashboard/numericReply';
import { factionNumbers } from '../../charts/race/factionParts';
import type { CountryGroupReply, FactionTotalsReply, TerritoryReply } from '../../generated/dashboard';
import { toHex } from '../../shell/appearance/color';
import { definition, OPTIONS } from './definition';
import { formatTerritory, MEASURES, measureOf, ruleOf, territoryCountryLines, territoryLegend, territoryModel, territoryNote, territoryRequest, territoryShareKeys, TERRITORY_PRESENTATION, territorySegmentAt, territoryWarnings } from './territoryData';
import { useRaceTooltip } from '../../charts/race/useRaceTooltip';
import { CHART_LAYOUT } from '../../charts/tokens';
import TerritoryShare from './TerritoryShare';
import { usePreparedTerritory } from './preparedTerritory';
import { useParityRows } from '../../dashboard/parityRows';
import { useRef } from 'react';
import '../../charts/race/countryRace.css';
import { motionOf } from '../../charts/easing.ts';

const NO_WARNINGS: string[] = [];
const GROUP_KEY = 'group';

/** Territory uses the Factories view structure and the race kit's drawing. */
export default function TerritoryWidget(props: WidgetProps) {
  const { widget } = props;
  const options = resolveOptions(OPTIONS, widget.options);
  const measure = measureOf(options.measure);
  const rule = ruleOf(options.rule);
  const factions = widget.countries.kind === 'factions' || (widget.countries.kind === 'group'
    && widget.countries.aggregate && widget.countries.group.type === 'faction');
  if (factions) return <FactionRaceView {...props} options={options}
    source={{ kind: 'territory', measure, rule }} max={definition.countries.max} label="Territory"
    measureLabel={unit => territoryMeasureLabel(measure, rule, unit)} formatValue={formatTerritory} note={territoryNote(measure)}
    renderMain={options.view === 'share' && rule !== 'occupied' ? (onPick, pickOwner, onDiagnostics, active) => <TerritoryCountryView {...props} active={active}
      onFactionPick={onPick} pickOwner={pickOwner} onDiagnostics={onDiagnostics} /> : undefined}
    unavailableReason={options.measure && !MEASURES.some(item => item.value === options.measure)
      ? 'This territory measure is unavailable. Choose another in settings.' : undefined} />;
  return <TerritoryCountryView {...props} />;
}

function TerritoryCountryView({ widget, runtime, active, onDiagnostics, onFactionPick, pickOwner }: WidgetProps & {
  onFactionPick?: (hour: number, id: string) => void; pickOwner?: unknown;
}) {
  const hold = useRef<HourHold | null>(null);
  const playbackHour = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow', hold);
  const tokens = useChartTokens();
  const options = resolveOptions(OPTIONS, widget.options);
  // A world share of occupied land means nothing: the Occupied rule shows its race.
  const view = options.view === 'share' && ruleOf(options.rule) !== 'occupied' ? 'share' : options.view === 'lines' ? 'lines' : 'race';
  const measure = measureOf(options.measure);
  const rule = ruleOf(options.rule);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const moment = Math.floor(playhead ?? extent[1]);
  const lastHour = track?.branch.lastHour ?? 0;
  const factionSelection = widget.countries.kind === 'factions' ? widget.countries : null;
  const aggregate = widget.countries.kind === 'group' && widget.countries.aggregate;
  const factions = factionSelection !== null || (aggregate && widget.countries.kind === 'group' && widget.countries.group.type === 'faction');
  const grouped = !factions && widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const manual = widget.countries.kind === 'tags' || widget.countries.kind === 'selected';
  const groupReply = useCountryGroups(runtime, track, active && (grouped || factions || view === 'lines'));
  const numericKey = JSON.stringify([track?.source.id, track?.source.metadata.campaignKey, track?.branch.id,
    widget.countries, measure, rule, extent]);
  const groupStore = useMemo(() => new NumericReply<CountryGroupReply>(), [runtime.client]);
  const groupData = factions ? groupStore.update(numericKey, groupReply.data, groupNumbers) : groupReply.data;
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groupReply.data, lastHour, definition.countries.max, true),
    [widget.countries, fallback, groupReply.data, lastHour]);
  const params = track ? factions ? { branchId: track.branch.id, tags: [], top: null } : territoryRequest(track.branch.id, resolved, measure, rule) : null;
  const prepared = usePreparedTerritory(runtime.client,track,params,measure,rule,moment,
    active && (view==='share' || !factions && !!params && !!(params.top || params.tags.length)),resolved.membership ?? undefined,aggregate && view!=='share' && !factions,resolved.unobserved);
  const countryProduct = prepared;
  const parityRoot=useRef<HTMLDivElement>(null);
  useParityRows(parityRoot,()=>({kind:'chart',label:'Territory values',complete:!!countryProduct.data,total:countryProduct.data?.series.length ?? 0,
    hour:moment,rows:countryProduct.data?.series.map(series=>({tag:series.tag,values:runAt(series.runs,moment)?.values ?? null})) ?? []}));
  const factionParams = track ? factionTotalsRequest(track.branch.id, widget.countries, { kind: 'territory', measure, rule }, definition.countries.max) : null;
  const factionProduct = usePreparedFactionTotals(runtime.client, track, factionParams, moment, active && factions, extent, undefined, false);
  // A head kept while playback's newest loads is ranked and named at the hour it answers.
  const shownMoment = view === 'lines' ? moment : (factions ? factionProduct.headHour : prepared.headHour) ?? moment;
  // Held where what shows at the hour is the head's alone: no time axis, and no group read beside it.
  useEffect(() => {
    hold.current = view !== 'lines' && !grouped && !factions && !aggregate
      ? answerHold(playbackHour, moment, prepared.headHour, prepared.headThrough) : null;
  });
  const { error, retry } = factions ? factionProduct : countryProduct;
  const worldStore = useMemo(() => new NumericReply<TerritoryReply>(), [runtime.client]);
  const data = factions ? worldStore.update(numericKey, countryProduct.data, reply => ({ ...reply, warnings: [] })) : countryProduct.data;
  const factionStore = useMemo(() => new NumericReply<FactionTotalsReply>(), [runtime.client]);
  const factionData = factions ? factionStore.update(numericKey, factionProduct.data, factionNumbers) : factionProduct.data;
  const sourceData = factions ? factionData : data;
  const model = useMemo(() => data ? territoryModel(data, measure, rule, manual ? resolved.tags : undefined) : null, [data, measure, rule, manual, resolved.tags]);
  const factionModel = useMemo(() => factionData ? factionTotalsModel(factionData) : null, [factionData]);
  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const countryLines = useMemo(() => model ? territoryCountryLines(model, resolved, range, aggregate && view !== 'share') : [],
    [model, resolved, range, aggregate, view]);
  const lines = useMemo(() => {
    if (factions) return factionLines(factionModel, range);
    if (!aggregate || view === 'share') return countryLines.filter(line => view==='lines' || line.runs.length || prepared.lineSummaries.get(line.tag)?.buckets.some(Boolean));
    return view==='lines' || prepared.sumRuns.length ? [{tag:GROUP_KEY,runs:prepared.sumRuns}] : [];
  }, [factions, factionModel, aggregate, view, countryLines, resolved, model, range,prepared.sumRuns,prepared.lineSummaries]);
  const series = useMemo(() => raceSeriesIndex(lines), [lines]);
  const badges = useMemo(() => [...countryCapitulationBadges(groupReply.data,
    !aggregate && view === 'lines' && model ? lines.flatMap(line => {
      const country = model.series.get(line.tag);
      return country ? [{ tag: line.tag, ...country, membership: resolved.membership?.get(line.tag) ?? country.membership }] : [];
    }) : [], [range.firstHour, range.lastHour]).values()].flat(), [groupReply.data, aggregate, view, model, lines, resolved.membership, range]);
  const count = Math.max(1, manual ? model?.selectedCount ?? resolved.tags.length : resolved.tags.length);
  const slots = factionParams?.top ?? (aggregate ? 1 : resolved.top ?? count);
  const shareCount = factionParams?.top ?? resolved.top ?? count;
  const raceTags = useMemo(() => seriesAt(series, shownMoment), [series, shownMoment]);
  const shareTags = useMemo(() => view === 'share' && !factions ? topCountries(new Map(raceTags.map(tag =>
    [tag, runAt(series.series.get(tag)!.runs, shownMoment)?.value ?? null])), shareCount) : [], [view, factions, raceTags, series, shownMoment, shareCount]);
  const shownTags = factions ? [] : view === 'share' ? shareTags : aggregate ? [] : view === 'race' ? raceTags : lines.map(line => line.tag);
  const factionLooks = useFactionColours(runtime.client, factions ? track?.source : undefined);
  const factionWarnings = factions ? factionLooks.warnings : NO_WARNINGS;
  const factionColourError = factions ? factionLooks.error : null;
  const factionColours = factions ? factionLooks.colours : null;
  const recordedFactions = useMemo(() => new Map(((groupReply.data ?? groupData)?.factions ?? []).map(faction => [faction.id, faction])), [groupReply.data, groupData]);
  const factionNames = { factions: recordedFactions, names: factionLooks.names };
  const shareOwner = useMemo(() => ({ owner: pickOwner, data, factionData, groupData }), [pickOwner, data, factionData, groupData]);
  const colourStore = useMemo(() => ({ key: '', colours: new Map<string, string>() as ReadonlyMap<string, string> }), [runtime.client]);
  if (colourStore.key !== numericKey) { colourStore.key = numericKey; colourStore.colours = new Map(); }
  if (factionColours?.size) colourStore.colours = factionColours;
  const shareFactionColours = colourStore.colours;

  const diagnosticKey = JSON.stringify([...(factions ? factionProduct.data?.warnings ?? [] : territoryWarnings(data, measure)),
    ...(groupReply.data?.warnings ?? []), ...factionWarnings,
    ...(factionColourError ? [`Faction colours are unavailable: ${factionColourError.message}`] : []),
    ...(factions && (!factionProduct.data || !groupReply.data || factionLooks.loading) ? ['Territory faction names or membership decoration are loading.'] : []),
    ...(factions && error ? [`Territory faction refresh is unavailable: ${error.message}`] : []),
    ...(factions && groupReply.error ? [`Territory faction groups refresh is unavailable: ${groupReply.error.message}`] : []),
    ...(!factions && view === 'lines' && groupReply.error ? [`Territory capitulation markers are unavailable: ${groupReply.error.message}`] : []),
    ...(factions && countryProduct.error ? [`Territory world refresh is unavailable: ${countryProduct.error.message}`] : [])]);
  const diagnostics = useMemo(() => JSON.parse(diagnosticKey) as string[], [diagnosticKey]);
  useEffect(() => {
    onDiagnostics(diagnostics.length ? diagnostics : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [diagnostics, onDiagnostics]);

  const menu = useContextMenu('Territory actions');
  const segmentAt = useCallback((tag: string, hour: number) => territorySegmentAt(model, tag, hour), [model]);
  const lookTags = view === 'share' ? shownTags : widget.countries.kind === 'group' && widget.countries.group.type === 'war' ? [widget.countries.group.tag] : [];
  const looks = useCountryLooks(runtime.client, track, lookTags, shownMoment, active, segmentAt);
  const totalName = widget.countries.kind === 'group' ? groupLabel(widget.countries.group, groupReply.data, looks.name, shownMoment) : 'Countries';
  const nameOf = (tag: string, hour?: number) => factions ? factionNameAt(factionModel, tag, hour ?? shownMoment, factionNames) : aggregate ? totalName : looks.name(tag, hour);
  const colorOf = (tag: string) => factions ? shareFactionColours.get(tag)! : tokens.accent;
  const entityLooks = useMemo(() => aggregate ? { name: () => totalName, color: () => tokens.accent } : undefined,
    [aggregate, totalName, tokens.accent]);
  const entries = useMemo(() => view === 'race' ? seriesRaceEntries(series, shownMoment, slots, formatTerritory) : [], [view, series, shownMoment, slots]);
  const { showTip, tipWanted } = useRaceTooltip(active, view === 'share' && !factions);
  const labels = useCountryLabelsAt(runtime.client, track, tipWanted, active);

  if (!track) return <WidgetState kind="empty" icon="map" message="Load a campaign to see its territory." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (error && !(factions && factionData)) return <WidgetState kind="error" failure={asFailure(error)} onRetry={retry} />;
  if (view === 'share' && countryProduct.error && !(factions && data)) return <WidgetState kind="error" failure={countryProduct.error} onRetry={countryProduct.retry} />;
  const groupFailure = (grouped || factions) && groupReply.error ? <WidgetState kind="error" failure={groupReply.error} onRetry={groupReply.retry} /> : null;
  if (groupFailure && !groupData) return groupFailure;
  if (!factions && resolved.reason) return <>{groupFailure}<WidgetState kind="empty" icon="map" message={resolved.reason} /></>;
  // A group with no member found yet asks for no country totals, so no reply will come; the
  // share view reads world totals and says so itself.
  const none = factions || view === 'share' ? null : noMembersAt(widget.countries, resolved, groupReply.data, shownMoment, lastHour);
  if (none) return <>{groupFailure}<NoMembersState reason={none.reason} icon="map" /></>;
  if (options.measure && !MEASURES.some(item => item.value === options.measure)) return <>{groupFailure}<WidgetState kind="unavailable" message={'This territory measure is unavailable. Choose another in settings.'} /></>;
  if (!sourceData || (factions ? !factionModel : !model) || ((grouped || factions) && !groupData) || (view === 'share' && !data)) return <>{groupFailure}<WidgetState kind="loading" label="Loading territory…" /></>;
  const unavailable = factions ? factionData?.unavailableReason : measure === 'area' ? data?.areaReason : null;
  if (unavailable) return <>{groupFailure}<WidgetState kind="unavailable" message={measure === 'area' ? 'Map area is unavailable.' : 'Territory is unavailable.'} /></>;
  // A group's members at hours still being prepared are not known yet: no total is not proof of none.
  const preparing = groupFailure ?? <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  if (!lines.length && view !== 'share' && view!=='lines') return factions || resolved.known(extent[0], extent[1])
    ? <>{groupFailure}<WidgetState kind="unavailable" message="Territory totals are unavailable for this selection." /></> : preparing;
  if (factions && factionLooks.loading && !shareFactionColours.size) return <>{groupFailure}<WidgetState kind="loading" label="Loading faction colours…" /></>;
  if (factions && !lines.every(line => shareFactionColours.has(line.tag))) return <>{groupFailure}<WidgetState kind="error" what="faction colours" failure={factionColourError ?? undefined} /></>;
  const measureName = MEASURES.find(item => item.value === measure)!.label;
  const unit = factions ? factionData?.unit : data?.areaUnit;
  const countName = territoryMeasureLabel(measure, rule, unit);
  const jump = (hour: number, x: number, y: number) => menu.open(x, y, [jumpItem(runtime, track.id, hour)]);
  if (view === 'share') {
    const groupReason = factions && widget.countries.kind === 'group' ? factionData?.series.find(series => series.id === factionParams?.factionId)?.unavailableReason
      : grouped && widget.countries.kind === 'group' ? groupShareReason(resolved, widget.countries.membership, shownMoment) : null;
    if (!factions && !groupReason && !resolved.known(shownMoment)) return preparing;
    const shareColours = new Map(shareTags.map(tag => {
      const colour = looks.color(tag);
      return [tag, colour ? toHex({ r: colour[0], g: colour[1], b: colour[2] }) : null] as const;
    }));
    const reason = groupReason ?? (factions ? null : shareColourReason(shareTags, tag => shareColours.get(tag)));
    const shareColour = factions ? colorOf : (tag: string) => shareColours.get(tag)!;
    return <div ref={parityRoot} className="country-race">{groupFailure}
      <div className="country-race-chart" onPointerLeave={() => showTip(null)}>
        <TerritoryShare key={`${track.source.id}:${track.branch.id}:${measure}:${rule}`} series={series} worldReply={data!} measure={measure} rule={rule}
          hour={shownMoment} count={shareCount} nameOf={factions ? nameOf : looks.name} countryFlagOf={tag => ({
            flag: looks.flag(tag), flagPending: looks.flagPending(tag), flagReason: looks.flagReason(tag),
          })} colourOf={shareColour}
          factions={factions ? { reply: factionData! } : undefined} unavailableReason={reason}
          onPick={factions ? onFactionPick : undefined} pickOwner={shareOwner}
          expectedKeys={!factions && resolved.top == null ? territoryShareKeys(model!, resolved, shownMoment) : undefined}
          membershipNote={factions ? id => factionMembershipNote(factionModel, id) : undefined}
          nameReason={factions ? id => factionNameReasonAt(factionModel, id, shownMoment, factionNames) ? 'Name unavailable' : null : undefined}
          hoverLabel={factions ? undefined : labels.labelAt(shownMoment)} onHover={factions ? undefined : tag => showTip(shownMoment, [tag])}
          active={active} hoursPerSecond={motionOf(runtime.clock.getSnapshot())} onContextMenu={(x, y) => jump(shownMoment, x, y)} />
      </div>{menu.menu}
    </div>;
  }
  return <div ref={parityRoot} className="country-race">{groupFailure}<CountryRaceView runtime={runtime} track={track} active={active} options={options} view={view} extent={extent} playhead={playhead}
    moment={shownMoment} slots={slots} ranked={entries} lines={lines} segmentAt={segmentAt} countryTags={shownTags}
    entityLooks={entityLooks} lineLegendLabel={territoryLegend(false).label}
    badges={badges}
    lineSummaries={prepared.lineSummaries.size ? prepared.lineSummaries : undefined} onSummaryView={prepared.onSummaryView} onReachHour={prepared.onHoverHour}
    navigation={prepared} pickOwner={prepared.owner}
    label="Territory" empty={factions || resolved.known(shownMoment) ? `Not recorded on ${dayLabel(shownMoment)}.` : MEMBERSHIP_PREPARING} formatValue={formatTerritory}
    labels={{ race: `Territory, ${countName}, ${dayLabel(shownMoment)}`, lines: `Territory over time, ${countName}`, actions: 'Territory actions' }}
    presentation={{ ...TERRITORY_PRESENTATION, countryLooks: !aggregate, linePadding: CHART_LAYOUT.countryEndLabels }}
    tooltipTags={aggregate ? () => [] : undefined}
    tooltip={(tag, hour, _name, _colours, labelAt) => {
      const run = runAt(series.series.get(tag)?.runs ?? [], hour);
      const country = labelAt(hour)(tag);
      return run ? [{ title: aggregate ? totalName : country.name, countryFlag: aggregate ? undefined : country,
        subtitle: `${dayLabel(hour)} · ${countName}`, rows: [{ label: measureName, value: formatTerritory(run.value) }], note: territoryNote(measure) }] : null;
    }} lineTooltip={(hour, _tags, labelAt, colour) => {
      return [{ ...seriesTooltip(series, hour, aggregate ? () => ({ name: totalName }) : labelAt(hour),
        colour, formatTerritory, dayLabel(hour), countName, !aggregate), note: territoryNote(measure) }];
    }} /></div>;
}
