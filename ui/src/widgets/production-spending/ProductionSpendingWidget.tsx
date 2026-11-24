import { useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import type { WidgetProps } from '../../dashboard/definitions';
import { icDecimals } from '../../dashboard/icDecimals';
import { resolveOptions } from '../../dashboard/options';
import FamilyBreakdownHost from '../../families/breakdown/FamilyBreakdownHost.tsx';
import { breakdownViewOf, levelsShown } from '../../families/breakdown/options.ts';
import { OPTIONS, type FactoryKind } from './definition';
import { productionSpendingProjection, type ProductionSpendingProjectionOptions } from './productionSpendingProjection.ts';
import { LEVELS, spendingRecorded, spendingWidgetGate, validateProductionSpendingReply } from './spendingData';
import { useSpending } from './useSpending';

const EMPTY_SUBS = [] as const;

export default function ProductionSpendingWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const view = breakdownViewOf(options);
  const namesInRing = options.namesInRing !== false;
  const kind: FactoryKind = options.factories === 'dockyards' ? 'dockyards' : 'military';
  const refits = options.refits === true;
  const spending = useSpending(widget, runtime, active, false, validateProductionSpendingReply, () => '');
  const { track, tag, data, error, retry, hour, from, end } = spending;
  const following = widget.time.mode === 'follow';
  const levelKey = LEVELS.map(level => options[level] !== false).join();
  const levels = useMemo(() => levelsShown(options, LEVELS), [levelKey]);
  const decimals = data?.decimals ?? 5;
  const digits = icDecimals(options, decimals);

  const projectionOptions = useMemo<ProductionSpendingProjectionOptions>(() => ({
    kind,
    refits,
    from,
    firstHour: track?.branch.firstHour ?? 0,
    digits,
    levels,
  }), [kind, refits, from, track?.branch.firstHour, digits, levels]);

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

  const gate = spendingWidgetGate({
    hasTarget: Boolean(track && tag),
    hasError: Boolean(error),
    branch: track?.branch,
    hour,
    end,
    hasData: Boolean(data),
    recorded: Boolean(data && spendingRecorded(data)),
  });

  const blocked = gate === 'empty'
    ? <WidgetState kind="empty" icon="sunburst" message="Load a campaign and choose a country to see its production spending." />
    : gate === 'error'
      ? <WidgetState kind="error" failure={error!} onRetry={retry} />
      : gate === 'outside'
        ? <WidgetState kind="outside" />
        : gate === 'loading'
          ? <WidgetState kind="loading" label="Loading production spending…" />
          : gate === 'unavailable'
            ? <WidgetState kind="unavailable" message="Production spending was not recorded." />
            : null;

  return (
    <FamilyBreakdownHost
      declaration={productionSpendingProjection}
      options={projectionOptions}
      view={view}
      namesInRing={namesInRing}
      label={widget.title}
      subject="production spending"
      widget={widget}
      runtime={runtime}
      active={active}
      subscriptions={EMPTY_SUBS}
      staticSources={staticSources}
      offset={track?.axisOffset ?? 0}
      pin={!following && hour != null ? Math.floor(hour) : null}
      bounds={bounds}
      onSetOption={setOption}
      onDiagnostics={onDiagnostics}
      blockedState={blocked}
      loadingLabel="Loading production spending…"
    />
  );
}
