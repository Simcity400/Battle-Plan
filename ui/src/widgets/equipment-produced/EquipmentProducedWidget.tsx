import { useEffect, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import CountryRaceView from '../../charts/race/CountryRaceView';
import { linesTooltip } from '../../charts/race/lines';
import { mixCountry } from '../../charts/race/options';
import type { RaceView } from '../../charts/race/options';
import type { Span } from '../../charts/series';
import { CHART_LAYOUT } from '../../charts/tokens';
import { useChartTokens } from '../../charts/useChartTokens';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { isMember, rankedCandidates, resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { EquipmentProducedReply } from '../../generated/dashboard';
import { definition, OPTIONS } from './definition';
import { bandLabel, bandValue, foldsPerCountry, emptyRace, formatUnits, keyBands, MEASURES, mixBands, producedAt, producedModel, producedRequest, producedTooltips,
  raceEntries, rankProduced, seriesBands, totalRuns, validateProducedReply } from './producedData';
import type { Grouping, KeyBand, Measure } from './producedData';
import './equipmentProduced.css';

const NO_WARNINGS: string[] = [];

/** Equipment each country produced (spec 4.2 Equipment produced): a race re-ranked
 * every hour, the same values as lines over time, or one country's equipment stacked
 * by group. */
export default function EquipmentProducedWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const tokens = useChartTokens();
  const options = resolveOptions(OPTIONS, widget.options);
  const view = (options.view as RaceView) ?? 'race';
  const measure = (options.measure as Measure) ?? 'total';
  const grouping = (options.grouping as Grouping) ?? 'type';
  const included = useMemo(() => contentSet(options.included).map(contentKey), [options.included]);
  const refits = options.refits === true;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  // The moment a race or a mix shows: the playhead, the end of a range, or the branch end.
  const moment = Math.floor(playhead ?? extent[1]);
  const lastHour = track?.branch.lastHour ?? 0;
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groupReply = useCountryGroups(runtime, track, active && grouped);
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groupReply.data, lastHour, definition.countries.max),
    [widget.countries, fallback, groupReply.data, lastHour]);
  // Only what the widget draws is asked for: the chosen countries, or the top N, which
  // the product ranks at every hour and cuts to the hours each was among them.
  const params = track ? producedRequest(track.branch.id, resolved, measure, grouping, included, refits) : null;
  const request = track && params && (params.top || params.tags.length)
    ? { sourceId: track.source.id, revision: track.source.revision, product: 'equipment-produced', params } : null;
  const { data, error, retry } = useProduct<EquipmentProducedReply>(runtime.client, request, active, validateProducedReply);
  const model = useMemo(() => data ? producedModel(data) : null, [data]);
  const bands = useMemo(() => model ? keyBands(model, tokens.categories, tokens.muted) : [], [model, tokens]);
  // A country's bands: by exact variant each keeps its own largest in colour (user
  // decision, 29 Sep), else the reply's, the same for every country.
  const bandsFor = useMemo(() => {
    const own = new Map<string, KeyBand[]>();
    return (tag: string): readonly KeyBand[] => {
      if (!model || !foldsPerCountry(model)) return bands;
      if (!own.has(tag)) own.set(tag, seriesBands(model, tag, tokens.categories, tokens.muted));
      return own.get(tag)!;
    };
  }, [model, bands, tokens]);
  const slots = resolved.top ?? Math.max(1, resolved.tags.length);
  // The top N rank only countries that have produced something (user decision).
  const produced = resolved.top != null;
  // The hours a country is shown: in the top N (from the product), or a member of a
  // group counted at each hour; null shows it whenever recorded.
  const membershipOf = (tag: string): readonly Span[] | null => model?.series.get(tag)?.membership ?? resolved.membership?.get(tag) ?? null;

  // The countries at the moment, ranked by the value shown.
  const candidates = useMemo(() => model ? rankedCandidates(resolved, moment, model.tags) : [], [model, resolved, moment]);
  const ranked = useMemo(() => model ? rankProduced(model, candidates, moment, slots, produced) : [],
    [model, candidates, moment, slots, produced]);
  const lineTags = useMemo(() => resolved.top != null ? model?.tags ?? [] : resolved.tags, [resolved, model]);
  const mixTag = mixCountry(widget.countries, fallback, ranked);
  const shownTags = view === 'race' ? ranked.map(entry => entry.tag) : view === 'lines' ? lineTags : mixTag ? [mixTag] : [];

  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(groupReply.data?.warnings ?? [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groupReply.data, onDiagnostics]);

  const split = options.split === true;
  const entries = useMemo(() => model ? raceEntries(model, ranked, { split, bands: bandsFor, unknown: tokens.muted }) : [],
    [model, ranked, split, bandsFor, tokens.muted]);

  // The hours still to cut a country's runs to: a group's, counted at each hour. A
  // top-N reply's samples are already cut to its hours, so they are not cut again.
  const cutTo = (tag: string): readonly Span[] | null => resolved.membership?.get(tag) ?? null;
  // The hours shown: a date range, or the branch. Nothing outside it is drawn, fitted
  // or hovered, even where an axis is dragged past it.
  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  // Lines over time: one per country, cut to the hours it is shown.
  const lines = useMemo(() => !model || view !== 'lines' ? [] : lineTags.flatMap(tag => {
    const runs = totalRuns(model, tag, cutTo(tag), range);
    return runs.length ? [{ tag, runs }] : [];
  }), [model, view, lineTags, resolved.membership, range]); // eslint-disable-line react-hooks/exhaustive-deps
  // One country's mix: its groups stacked, cut to the hours it is shown.
  const mixMembership = mixTag ? membershipOf(mixTag) : null;
  const mixCut = mixTag ? cutTo(mixTag) : null;
  // The mix's bands: its country's own (by variant) or the reply's.
  const mixKeyBands = useMemo(() => mixTag ? bandsFor(mixTag) : bands, [mixTag, bandsFor, bands]);
  const mix = useMemo(() => {
    if (!model || view !== 'mix' || !mixTag) return null;
    return { tag: mixTag, bands: mixBands(model, mixTag, mixKeyBands, mixCut, range) };
  }, [model, view, mixTag, mixKeyBands, mixCut, range]);

  const measureInfo = MEASURES.find(item => item.value === measure) ?? MEASURES[0];
  const state = (() => {
    if (!track) return <WidgetState kind="empty" icon="layers" message="Load a campaign to see the equipment it produced." />;
    if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
    if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
    if (grouped && groupReply.error) return <WidgetState kind="error" message={`Country groups are unavailable: ${groupReply.error}`} onRetry={groupReply.retry} />;
    if (resolved.reason) return <WidgetState kind="empty" icon="layers" message={resolved.reason} />;
    // A group's members are not known until its history arrives; nothing is shown as empty meanwhile.
    if (!data || !model || (grouped && !groupReply.data)) return <WidgetState kind="loading" label="Loading equipment produced…" />;
    if (!data.reads.length) return <WidgetState kind="unavailable" message="Not in this recording: it has no count of the equipment production lines delivered." />;
    // A shorter branch has no week/month value, which is not the same as no production.
    if (!produced && !model.tags.length) return <WidgetState kind="unavailable" message={measure === 'total' ? 'No production was recorded for these countries in this branch.'
      : `No ${measureInfo.label.toLowerCase()} value yet: each needs a read of the same country a ${measure === 'week' ? 'week' : 'month'} before, and this branch has none that long.`} />;
    return null;
  })();
  // `name` names the country as recorded at the hour the tooltip is about.
  const tooltipAt = (tag: string, hour: number, name: string) => {
    if (!model) return null;
    const at = isMember(membershipOf(tag) ?? undefined, hour) ? producedAt(model, tag, hour) : null;
    return at ? producedTooltips(model, tag, name, at, hour, dayLabel(hour), bandsFor(tag)) : null;
  };
  const parts = bands.map(band => ({ id: band.id, label: bandLabel(model!, band, moment), color: band.color }));
  const mixAt = model && mixTag && isMember(mixMembership ?? undefined, moment) ? producedAt(model, mixTag, moment) : null;
  const mixSeries = model && mixTag ? model.series.get(mixTag) : null;
  const mixParts = mixKeyBands.map(band => {
    const value = mixAt && mixSeries ? bandValue(mixSeries, band, mixAt.index) : Number.NaN;
    return { id: band.id, label: bandLabel(model!, band, moment), color: band.color, value: Number.isNaN(value) ? '—' : formatUnits(value) };
  });
  const window_ = measure === 'week' ? 'week' : 'month';
  // The top N show a country once it has produced something; say so when none has.
  const nothingYet = produced && model && emptyRace(model, moment) === 'nothing'
    ? (measure === 'total' ? `No country has produced anything by ${dayLabel(moment)}; a country appears here once it has.`
      : `No country produced anything in the ${window_} before ${dayLabel(moment)}; a country appears here once it has.`)
    : measure === 'total' ? `No production recorded on ${dayLabel(moment)}.`
      : `Nothing to show on ${dayLabel(moment)}: ${measureInfo.label.toLowerCase()} needs unbroken reads over the ${window_} before.`;

  return <CountryRaceView runtime={runtime} track={track} active={active} options={options} view={view} extent={extent} playhead={playhead} state={state}
    moment={moment} slots={slots} ranked={entries} lines={lines} countryTags={shownTags} parts={parts}
    mix={mix ? { ...mix, parts: mixParts } : null} partsLabel="Equipment"
    raceLegendParts={split && model && !foldsPerCountry(model) && bands.length > 1 ? parts : []}
    beforeChart={view === 'race' && split && model && foldsPerCountry(model)
      ? <p className="equipment-produced-note">Each bar keeps its own country’s largest variants in colour; the rest are Other. Hover a bar to name them.</p> : null}
    label={`Equipment ${measureInfo.unit}`} empty={nothingYet} formatValue={formatUnits} tooltip={tooltipAt}
    lineTooltip={(hour, _tags, labelAt, country) => [linesTooltip(hour, lines, labelAt, country, dayLabel(hour), measureInfo.label, formatUnits)]}
    labels={{ race: `Equipment ${measureInfo.unit}, ${dayLabel(moment)}`, lines: `Equipment ${measureInfo.unit} over time`,
      mix: name => `Equipment of ${name} ${measureInfo.unit}, by ${grouping}`, actions: 'Equipment produced actions' }}
    presentation={{ exact: false, held: false,
      strongMixName: true, mixKeyHours: 'starts', raceEmptyState: false, linePadding: CHART_LAYOUT.countryEndLabels, resetTimeView: true }}
    lineEmpty={{ icon: 'layers', message: 'No countries to draw. Choose countries in the widget settings.' }}
    mixEmpty={{ icon: 'layers', message: widget.countries.kind === 'group'
      ? `No country of this group was recorded on ${dayLabel(moment)}.` : 'Choose a country in the widget settings.',
      unavailable: name => `No production was recorded for ${name} in this branch.` }}
    resetKey={`${track?.source.id ?? ''}:${track?.branch.id ?? ''}:${measure}:${grouping}`} />;
}
