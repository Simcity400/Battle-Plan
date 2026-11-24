import { useCallback, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import FactionTotalsView from '../../charts/race/FactionTotalsView';
import AggregateTotalsView, { aggregated } from '../../charts/race/AggregateTotalsView';
import MeasureRace from '../../charts/race/MeasureRace';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import type { OptionValue } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import type { GatedHour } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import { formatTerritory, measureOf, ruleOf, territoryMeasureLabel, territoryNote, territorySeriesMeasure } from '../../dashboard/territoryMeasure.ts';
import type { TerritoryMeasure, TerritoryRule } from '../../dashboard/territoryMeasure';
import FamilyBreakdownHost from '../../families/breakdown/FamilyBreakdownHost.tsx';
import { useLaneIdentity } from '../../families/breakdown/useLaneIdentity.ts';
import type { FamilySourceSubscription } from '../../families/contract.ts';
import type { TerritoryShareReply } from '../../generated/dashboard';
import { validateShareReply } from './territoryData.ts';
import { territoryShareProjection, type TerritoryShareProjectionOptions } from './territoryShareProjection.ts';
import { OPTIONS } from './definition';
import '../../charts/race/countryRace.css';

const NO_WARNINGS: string[] = [];

export default function TerritoryWidget(props: WidgetProps) {
  const options = resolveOptions(OPTIONS, props.widget.options);
  const measure = measureOf(options.measure), rule = ruleOf(options.rule);
  if (props.widget.countries.kind === 'factions') return <FactionTotalsView {...props} options={options} kind="territory"
    measure={territorySeriesMeasure(measure, rule)} label="Territory" measureLabel={territoryMeasureLabel(measure, rule)}
    note={territoryNote(measure)} format={formatTerritory} />;
  if (aggregated(props.widget.countries) && options.view !== 'share') return <AggregateTotalsView {...props} options={options} kind="territory"
    measure={territorySeriesMeasure(measure, rule)} label="Territory" measureLabel={territoryMeasureLabel(measure, rule)}
    note={territoryNote(measure)} format={formatTerritory} />;
  if (options.view === 'share' && rule !== 'occupied') return <TerritoryShareView {...props} measure={measure} rule={rule} />;
  return <TerritoryRaceView {...props} options={options} measure={measure} rule={rule} />;
}

interface Chosen { measure: TerritoryMeasure; rule: TerritoryRule }

const formatRaw = (raw: string) => formatTerritory(Number(raw));

function TerritoryRaceView({ widget, runtime, active, onDiagnostics, options, measure, rule }: WidgetProps & Chosen & { options: Readonly<Record<string, OptionValue>> }) {
  const family = useMemo(() => ({ kind: 'territory', measure: territorySeriesMeasure(measure, rule) }), [measure, rule]);
  const onWarnings = useCallback((warnings: readonly string[]) => onDiagnostics(warnings.length ? [...warnings] : NO_WARNINGS), [onDiagnostics]);
  return <MeasureRace widget={widget} runtime={runtime} active={active} measure={family} decimals={0} view={options.view === 'lines' ? 'lines' : 'race'}
    options={options} label={`Territory, ${territoryMeasureLabel(measure, rule)}`} icon="map" format={formatRaw} formatValue={formatTerritory}
    note={territoryNote(measure)} onWarnings={onWarnings} />;
}

function TerritoryShareView({ widget, runtime, active, onDiagnostics, setOption, measure, rule }: WidgetProps & Chosen) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const name = territorySeriesMeasure(measure, rule);
  const count = widget.countries.kind === 'group' && widget.countries.group.type === 'top' ? widget.countries.group.count : 10;
  const following = widget.time.mode === 'follow';

  const resolvedWin = useMemo(
    () => !following && track ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour()) : null,
    [following, track, widget, runtime.tracks, runtime.clock],
  );
  const pinHour = !following && resolvedWin ? Math.floor(resolvedWin.lastHour) : null;

  const staticReq = useMemo(
    () => !following && track && pinHour !== null
      ? productRequest({
          sourceId: track.source.id,
          revision: track.source.revision,
          product: 'territory-share',
          params: { branchId: track.branch.id, hour: pinHour as GatedHour, measure: name },
        })
      : null,
    [following, track?.source.id, track?.source.revision, track?.branch.id, pinHour, name],
  );
  const staticRead = useProduct<TerritoryShareReply>(runtime.client, staticReq, active && !following, validateShareReply, track?.source);

  const subscriptions = useMemo<readonly FamilySourceSubscription[]>(() => {
    if (!following || !track) return [];
    return [{
      name: 'share',
      source: track.source,
      branch: track.branch.id,
      offset: track.axisOffset,
      channel: 'panel',
      product: 'territory-share',
      query: { kind: 'territory-share', measure: name },
    }];
  }, [following, track, name]);

  const { identitySnapshot, onIdentityDemands } = useLaneIdentity(runtime.client, track, active);

  const projectionOptions = useMemo<TerritoryShareProjectionOptions>(() => ({ measure, rule, count }), [measure, rule, count]);
  const staticSources = useMemo(() => {
    if (following || !staticRead.data || pinHour === null) return null;
    return { panels: { share: staticRead.data }, hour: pinHour };
  }, [following, staticRead.data, pinHour]);

  const bounds = useMemo<readonly [number, number] | null>(
    () => track ? [track.branch.firstHour, track.branch.lastHour] : null,
    [track],
  );

  const blocked = !track
    ? <WidgetState kind="empty" icon="map" message="Load a campaign to see its territory." />
    : !following && staticRead.error
      ? <WidgetState kind="error" failure={staticRead.error} onRetry={staticRead.retry} />
      : !following && !staticRead.data
        ? <WidgetState kind="loading" label="Loading territory share…" />
        : null;

  return (
    <FamilyBreakdownHost
      declaration={territoryShareProjection}
      options={projectionOptions}
      view="ring"
      namesInRing
      label={`Territory share, ${territoryMeasureLabel(measure, rule)}`}
      subject="territory share"
      widget={widget}
      runtime={runtime}
      active={active}
      subscriptions={subscriptions}
      staticSources={staticSources}
      offset={track?.axisOffset ?? 0}
      pin={pinHour}
      bounds={bounds}
      identitySnapshot={identitySnapshot}
      onIdentityDemands={onIdentityDemands}
      onSetOption={setOption}
      onDiagnostics={onDiagnostics}
      blockedState={blocked}
      loadingLabel="Loading territory share…"
    />
  );
}
