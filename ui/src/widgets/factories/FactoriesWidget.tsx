import { useCallback, useEffect, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import CountryRaceView from '../../charts/race/CountryRaceView';
import type { CountryRacePart } from '../../charts/race/countryRace';
import type { RaceView } from '../../charts/race/options';
import { mixCountry } from '../../charts/race/options';
import { linesTooltip } from '../../charts/race/lines';
import type { Span } from '../../charts/series';
import { CHART_LAYOUT } from '../../charts/tokens';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { isMember, rankedCandidates, resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { contentSet, resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { CountryFactoriesReply } from '../../generated/dashboard';
import { MAX_COUNTRIES, OPTIONS } from './definition';
import { chosenGroups, factoriesAt, factoriesRequest, factoryModel, factorySegmentAt, factoryTooltips, formatCount, groupName, lineRuns, mixBands,
  raceEntries, rankFactories, validateFactoriesReply } from './factoriesData';
import type { CountMode } from './factoriesData';

const NO_WARNINGS: string[] = [];

/** Factory counts retain their formulas and wording; the shared race owns their views. */
export default function FactoriesWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const view = (options.view as RaceView) ?? 'race';
  const mode = (options.count as CountMode) ?? 'free';
  const chosen = contentSet(options.types);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const moment = Math.floor(playhead ?? extent[1]);
  const lastHour = track?.branch.lastHour ?? 0;
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groupReply = useCountryGroups(runtime, track, active && grouped);
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groupReply.data, lastHour, MAX_COUNTRIES),
    [widget.countries, fallback, groupReply.data, lastHour]);
  const params = track ? factoriesRequest(track.branch.id, resolved, mode, chosen) : null;
  const request = track && params && (params.top || params.tags.length)
    ? { sourceId: track.source.id, revision: track.source.revision, product: 'country-factories', params } : null;
  const { data, error, retry } = useProduct<CountryFactoriesReply>(runtime.client, request, active, validateFactoriesReply);
  const model = useMemo(() => data ? factoryModel(data) : null, [data]);
  const groups = useMemo(() => model ? chosenGroups(model, chosen) : [], [model, chosen.join(' ')]); // eslint-disable-line react-hooks/exhaustive-deps
  const slots = resolved.top ?? Math.max(1, resolved.tags.length);
  const membershipOf = (tag: string): readonly Span[] | null => model?.series.get(tag)?.membership ?? resolved.membership?.get(tag) ?? null;
  const cutTo = (tag: string): readonly Span[] | null => resolved.membership?.get(tag) ?? null;
  const ranked = useMemo(() => model ? rankFactories(model, rankedCandidates(resolved, moment, model.tags), moment, groups, mode, slots) : [],
    [model, resolved, moment, groups, mode, slots]);
  const lineTags = useMemo(() => resolved.top != null ? model?.tags ?? [] : resolved.tags, [resolved, model]);
  const mixTag = mixCountry(widget.countries, fallback, ranked);
  const shownTags = view === 'race' ? ranked.map(entry => entry.tag) : view === 'lines' ? lineTags : mixTag ? [mixTag] : [];
  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(groupReply.data?.warnings ?? []),
      ...(resolved.dropped ? [`${resolved.dropped} more countries are left out; this widget shows at most ${MAX_COUNTRIES}.`] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groupReply.data, resolved.dropped, onDiagnostics]);

  const split = options.split === true;
  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const lines = useMemo(() => !model || view !== 'lines' ? [] : lineTags.flatMap(tag => {
    const runs = lineRuns(model, tag, groups, mode, cutTo(tag), range);
    return runs.length ? [{ tag, runs }] : [];
  }), [model, view, lineTags, groups, mode, resolved.membership, range]); // eslint-disable-line react-hooks/exhaustive-deps
  const segmentAt = useCallback((tag: string, hour: number) => factorySegmentAt(model, tag, hour), [model]);
  const parts = useMemo<CountryRacePart[]>(() => (model?.reply.groups ?? []).map(group => ({ id: group.key, label: groupName(group) })), [model]);
  const chosenParts = useMemo(() => parts.filter(part => groups.includes(part.id as typeof groups[number])), [parts, groups]);
  const entries = useMemo(() => raceEntries(ranked, { mode, split }), [ranked, mode, split]);
  const mixCut = mixTag ? cutTo(mixTag) : null;
  const mixMembership = mixTag ? membershipOf(mixTag) : null;
  const bands = useMemo(() => model && view === 'mix' && mixTag ? mixBands(model, mixTag, groups, mode, mixCut, range) : null,
    [model, view, mixTag, groups, mode, mixCut, range]);
  const mix = useMemo(() => {
    if (!model || !mixTag || !bands) return null;
    const at = isMember(mixMembership ?? undefined, moment) ? factoriesAt(model, mixTag, moment, groups, mode) : null;
    return { tag: mixTag, bands, parts: chosenParts.map(part => {
      const count = at?.groups.find(group => group.key === part.id)?.count;
      return { ...part, value: count == null ? '—' : formatCount(count) };
    }) };
  }, [model, mixTag, bands, groups, mode, mixMembership, moment, chosenParts]);

  const state = (() => {
    if (!track) return <WidgetState kind="empty" icon="factory" message="Load a campaign to see its factories." />;
    if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
    if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
    if (grouped && groupReply.error) return <WidgetState kind="error" message={`Country groups are unavailable: ${groupReply.error}`} onRetry={groupReply.retry} />;
    if (resolved.reason) return <WidgetState kind="empty" icon="factory" message={resolved.reason} />;
    if (!data || !model || (grouped && !groupReply.data)) return <WidgetState kind="loading" label="Loading factories…" />;
    if (!model.tags.length) return <WidgetState kind="unavailable" message="No factories were recorded for these countries in this branch." />;
    return undefined;
  })();

  const modeLabel = OPTIONS.find(field => field.key === 'count')!;
  const countName = modeLabel.kind === 'choice' ? modeLabel.choices.find(choice => choice.value === mode)?.label ?? mode : mode;
  return <CountryRaceView runtime={runtime} track={track} state={state} active={active} options={options} view={view} extent={extent} playhead={playhead}
    moment={moment} slots={slots} ranked={entries} lines={lines} parts={parts} mix={mix} segmentAt={segmentAt} countryTags={shownTags}
    label="Factories" empty={`No factories recorded on ${dayLabel(moment)}.`} formatValue={formatCount}
    labels={{ race: `Factories, ${countName}, ${dayLabel(moment)}`, lines: `Factories over time, ${countName}`,
      mix: name => `Factories of ${name} by type, ${countName}`, actions: 'Factory actions' }}
    raceLegendParts={split && groups.length > 1 ? chosenParts : []} partsLabel="Factory types"
    presentation={{ exact: false, held: false, mixSamplePixels: .5, lineColourHours: 'labels', raceTooltip: 'sampled',
      tooltipKeep: groups.length > 1 ? 1 : 0, linePadding: CHART_LAYOUT.countryEndLabels, strongMixName: true,
      mixKeyHours: 'starts', raceEmptyState: false, resetTimeView: true }}
    lineEmpty={{ icon: 'factory', message: 'No countries to draw. Choose countries in the widget settings.' }}
    mixEmpty={{ icon: 'factory', message: widget.countries.kind === 'group'
      ? `No country of this group was recorded on ${dayLabel(moment)}.` : 'Choose a country in the widget settings.',
      unavailable: name => `No factories were recorded for ${name} in this branch.` }}
    tooltip={(tag, hour, name, colours) => {
      if (!model) return null;
      const at = isMember(membershipOf(tag) ?? undefined, hour) ? factoriesAt(model, tag, hour, groups, mode) : null;
      return at ? factoryTooltips(model, name, at, dayLabel(hour), mode, colours) : null;
    }} lineTooltip={(hour, _tags, labelAt, country) => [linesTooltip(hour, lines, labelAt, country, dayLabel(hour), countName, formatCount)]} />;
}
