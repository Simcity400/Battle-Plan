import { useGate } from '../../dashboard/regionGate';
import { countryKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import CountryPicker from '../../components/CountryPicker';
import MeasureRace from '../../charts/race/MeasureRace';
import { mixCountry } from '../../charts/race/options.ts';
import type { LabelAt } from '../../charts/race/lines';
import { countryDrillSpans } from '../../dashboard/countryIdentity.ts';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion, useTransitionValue } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { countryMatch } from '../../dashboard/groups';
import { axisHour, regionHour, SPEC } from '../../dashboard/regionTime.ts';
import { coverageOf, useRegion, useRegionOutside } from '../../dashboard/widgetTime.ts';
import WidgetState from '../../components/WidgetState';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { OPTIONS } from './definition';
import { SERIES, measureName, occupationPercent, occupationTooltip, otherMeasure, rankingMeasure, validateOccupationDetail } from './occupationTrendData.ts';
import type { DetailAt, OccupationMeasure, ShownMeasure } from './occupationTrendData';
import type { OccupationDetailReply } from '../../generated/dashboard';
import '../../charts/race/countryRace.css';
import { useChangedState } from '../../dashboard/changedState.ts';
import { productRequest } from '../../dashboard/request.ts';

const NO_WARNINGS: string[] = [];

export default function OccupationTrendWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const shown = options.measure as ShownMeasure;
  const rank = rankingMeasure(shown, options.rankingMeasure as OccupationMeasure);
  const collaboration = options.collaboration === true;
  const digits = Number(options.digits);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const playing = (usePlaybackMotion(runtime.clock) ?? 0) > 0;
  const measure = useMemo(() => ({ kind: SERIES, measure: rank }), [rank]);
  const second = useMemo(() => shown === 'both' ? { kind: SERIES, measure: otherMeasure(rank) } : undefined, [shown, rank]);
  const format = useCallback((raw: string) => occupationPercent(raw, digits), [digits]);
  const formatValue = useCallback((value: number) => `${value.toFixed(digits)}%`, [digits]);

  const groups = useCountryGroups(runtime, track, active);
  const fallback = defaultCountryKey(track);
  const readHour = useCallback((): number | null => regionHour({ widget, action: SPEC }, track, runtime.tracks, axisHour(runtime.clock)),
    [widget, track, runtime.tracks, runtime.clock]);
  const mixOutside = useRegionOutside(runtime, useRegion(widget), active);
  const gate = useGate(runtime, widget, SPEC, active);
  const settled = useTransitionValue(!playing ? runtime.clock.subscribe : null, readHour);
  const held = useRef(settled);
  if (!playing) held.current = settled;
  const choiceHour = held.current;
  const choiceAt = gate.recorded(choiceHour);
  const occupiers = useProduct<OccupationDetailReply>(runtime.client, track && choiceAt !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'occupation-at', params: { branchId: track.branch.id, hour: choiceAt, countries: [], collaboration: false } }) : null, active, validateOccupationDetail, track?.source);
  const match = useMemo(() => countryMatch(widget.countries, fallback ? [fallback] : [], groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const candidates = useMemo(() => choiceHour === null ? [] : (occupiers.data?.occupiers ?? []).filter(row => row.missing === null && match.matches(countryKey(row.country), choiceHour)).map(row => countryKey(row.country)),
    [occupiers.data, match, choiceHour]);
  const [picked, setPicked] = useState<{ tag: string; hour: number } | null>(null);
  const originHour = picked?.hour ?? choiceHour;
  const resolvedTag = mixCountry(widget.countries, fallback, candidates.map(tag => ({ tag })));
  const mixTag = picked && candidates.includes(picked.tag) ? picked.tag : resolvedTag !== null && candidates.includes(resolvedTag) ? resolvedTag : candidates.length === 1 ? candidates[0] : null;
  const mixOrigin = picked?.tag === mixTag ? originHour : choiceHour;
  const spans = useMemo(() => mixTag && groups.data && mixOrigin !== null ? countryDrillSpans(groups.data.countryIdentities, mixTag, mixOrigin) : null,
    [mixTag, mixOrigin, groups.data]);
  const spansKey = spans ? JSON.stringify([...spans]) : null;
  const mix = useMemo(() => mixTag && spans ? { tag: mixTag, spans } : null, [mixTag, spansKey]);

  const [detailHour, setDetailHour] = useState<number | null>(null);
  const asked = useRef<number | null>(null);
  const ask = useCallback((hour: number | null) => {
    const at = hour === null ? null : Math.floor(hour);
    if (asked.current === at) return;
    asked.current = at;
    queueMicrotask(() => setDetailHour(at));
  }, []);
  const detailAt = gate.recorded(detailHour);
  const request = track && detailAt !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'occupation-at',
    params: { branchId: track.branch.id, hour: detailAt, countries: [], collaboration } }) : null;
  const read = useProduct<OccupationDetailReply>(runtime.client, request, active, validateOccupationDetail, track?.source);
  const detail = useMemo<DetailAt | null>(() => read.data && detailHour !== null ? { hour: detailHour, reply: read.data, values: repliedCells(read.data).read() } : null,
    [read.data, detailHour]);
  const tooltip = useCallback((tag: string, hour: number, _name: string, labelAt: LabelAt) => {
    ask(hour);
    return [occupationTooltip(tag, Math.floor(hour), detail, labelAt, shown, collaboration)];
  }, [ask, detail, shown, collaboration]);

  const [warnings, setWarnings] = useChangedState<readonly string[]>(NO_WARNINGS);
  const onWarnings = useCallback((items: readonly string[]) => setWarnings(before => before.join('\0') === items.join('\0') ? before : items), []);
  const diagnosticKey = JSON.stringify([...warnings, ...(read.data?.warnings ?? [])]);
  useEffect(() => {
    const all = JSON.parse(diagnosticKey) as string[];
    onDiagnostics(all.length ? all : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [diagnosticKey, onDiagnostics]);

  const legend = shown === 'both'
    ? <p className="chart-line-key"><i aria-hidden="true" />Solid: {measureName(rank).toLowerCase()} · <i className="is-dashed" aria-hidden="true" />dashed: {measureName(otherMeasure(rank)).toLowerCase()}</p> : null;
  const chooser = track && choiceHour !== null ? <CountryPicker countries={track.source.metadata.countries} allowedTags={candidates} selectedTag={mixTag} playerCountryId={null}
    source={track.source} client={runtime.client} field flagAt={{ branchId: track.branch.id, hour: choiceHour }}
    onChange={tag => { if (tag && candidates.includes(tag)) setPicked({ tag, hour: choiceHour }); }} /> : null;
  const unavailable = occupiers.data && !candidates.length ? <p className="chart-line-key">No occupied-state data at this time.</p>
    : !mixTag ? <p className="chart-line-key">Choose an occupier.</p>
    : groups.data && spans && spans.size === 0 ? <p className="chart-line-key">Country unavailable at this time.</p> : null;
  if (mixOutside) return <WidgetState kind="outside" read={coverageOf(mixOutside)} />;
  return <MeasureRace widget={widget} runtime={runtime} active={active} measure={measure} decimals={5} view="mix"
    options={options} label={`Average ${measureName(rank).toLowerCase()}`} icon="globe" format={format} formatValue={formatValue}
    onWarnings={onWarnings} second={second} mix={mix} tooltip={tooltip} onHover={ask}
    beforeChart={<>{chooser}{unavailable}{legend}{read.error ? <p className="chart-line-key">Occupation details could not be read: {read.error.message}</p> : null}</>} />;
}
