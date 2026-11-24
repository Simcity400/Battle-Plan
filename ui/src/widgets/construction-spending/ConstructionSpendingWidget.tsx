import { useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { icDecimals } from '../../dashboard/icDecimals';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { productRequest } from '../../dashboard/request.ts';
import FamilyBreakdownHost from '../../families/breakdown/FamilyBreakdownHost.tsx';
import { breakdownViewOf, levelsShown } from '../../families/breakdown/options.ts';
import { OPTIONS } from './definition';
import { constructionSpendingProjection, type ConstructionProjectionOptions } from './constructionSpendingProjection.ts';
import { LEVELS, scaleDigits, validateSpendingReply } from './spendingData';

const EMPTY_SUBS = [] as const;

export default function ConstructionSpendingWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const view = breakdownViewOf(options);
  const namesInRing = options.namesInRing !== false;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryKey(track) ?? undefined;
  const request = useMemo(
    () => track && tag
      ? productRequest({
          sourceId: track.source.id,
          revision: track.source.revision,
          product: 'construction-spending',
          params: { branchId: track.branch.id, country: countryFromKey(tag) },
        })
      : null,
    [track?.source.id, track?.source.revision, track?.branch.id, tag],
  );
  const { data, error, retry } = useProduct(runtime.client, request, active, validateSpendingReply, track?.source);

  const following = widget.time.mode === 'follow';
  const range = widget.time.mode === 'range';
  const resolvedWin = useMemo(
    () => track ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour()) : null,
    [track, widget, runtime.tracks, runtime.clock],
  );
  const from = range ? resolvedWin?.firstHour ?? null : null;
  const pinHour = !following && resolvedWin ? Math.floor(resolvedWin.lastHour) : null;
  const levelKey = LEVELS.map(level => options[level] !== false).join();
  const levels = useMemo(() => levelsShown(options, LEVELS), [levelKey]);
  const scale = data?.costScale ?? 1;
  const digits = icDecimals(options, scaleDigits(scale));

  const projectionOptions = useMemo<ConstructionProjectionOptions>(() => ({
    from,
    firstHour: track?.branch.firstHour ?? 0,
    digits,
    levels,
  }), [from, track?.branch.firstHour, digits, levels]);

  const staticSources = useMemo(() => {
    if (!data || !track) return null;
    return {
      panels: { spending: data },
      hour: track.branch.firstHour,
      endHour: track.branch.lastHour,
    };
  }, [data, track]);

  const bounds = useMemo<readonly [number, number] | null>(
    () => track ? [track.branch.firstHour, track.branch.lastHour] : null,
    [track],
  );

  const blocked = !track || !tag
    ? <WidgetState kind="empty" icon="sunburst" message="Load a campaign and choose a country to see its construction spending." />
    : error
      ? <WidgetState kind="error" failure={error} onRetry={retry} />
      : !data
        ? <WidgetState kind="loading" label="Loading construction spending…" />
        : null;

  return (
    <FamilyBreakdownHost
      declaration={constructionSpendingProjection}
      options={projectionOptions}
      view={view}
      namesInRing={namesInRing}
      label="Construction spending"
      subject="construction spending"
      widget={widget}
      runtime={runtime}
      active={active}
      subscriptions={EMPTY_SUBS}
      staticSources={staticSources}
      offset={track?.axisOffset ?? 0}
      pin={pinHour}
      bounds={bounds}
      onSetOption={setOption}
      onDiagnostics={onDiagnostics}
      blockedState={blocked}
      loadingLabel="Loading construction spending…"
    />
  );
}
