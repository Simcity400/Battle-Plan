import { useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { powerDecimals } from '../../dashboard/powerDecimals';
import ReadView from '../../dashboard/ReadView';
import { usePowerSpending } from '../../dashboard/usePowerSpending.ts';
import FamilyBreakdownHost from '../../families/breakdown/FamilyBreakdownHost.tsx';
import { breakdownViewOf, levelsShown } from '../../families/breakdown/options.ts';
import { OPTIONS } from './definition';
import { ppSpendingProjection, type PowerSpendingPanelData, type PowerSpendingProjectionOptions } from './ppSpendingProjection.ts';

const LEVELS = ['categoryLevel', 'itemLevel'];
const EMPTY_SUBS = [] as const;

export default function PoliticalPowerSpendingBreakdownWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const view = breakdownViewOf(options);
  const namesInRing = options.namesInRing !== false;
  const digits = powerDecimals(options);
  const { track, read, tags, ledgers, error, groupError, retry, ready, reason, selection, groupWarnings, span } = usePowerSpending(widget, runtime, active, 'Political');
  const following = widget.time.mode === 'follow';
  const from = widget.time.mode === 'range' ? span?.firstHour ?? null : null;
  const levelKey = LEVELS.map(level => options[level] !== false).join();
  const levels = useMemo(() => levelsShown(options, LEVELS), [levelKey]);

  const pinnedWindow = useMemo(
    () => !following && track ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour()) : null,
    [following, track, widget, runtime.tracks, runtime.clock],
  );
  const pinHour = !following && pinnedWindow ? Math.floor(pinnedWindow.lastHour) : null;

  const panelData = useMemo<PowerSpendingPanelData | null>(() => {
    if (!ledgers || !track) return null;
    return {
      ledgers,
      from,
      firstHour: track.branch.firstHour,
      selection,
    };
  }, [ledgers, track, from, selection]);

  const staticSources = useMemo(() => {
    if (!panelData || !track) return null;
    return {
      panels: { spending: panelData },
      hour: track.branch.firstHour,
      endHour: track.branch.lastHour,
    };
  }, [panelData, track]);

  const projectionOptions = useMemo<PowerSpendingProjectionOptions>(() => ({ digits, levels }), [digits, levels]);

  const bounds = useMemo<readonly [number, number] | null>(
    () => track ? [track.branch.firstHour, track.branch.lastHour] : null,
    [track],
  );

  const blocked = !track
    ? <WidgetState kind="empty" icon="sunburst" message="Load a campaign and choose a country to see its political power spending." />
    : read.state === 'outside' || read.state === 'refused'
      ? <ReadView read={read} loading="">{() => null}</ReadView>
      : error
        ? <WidgetState kind="error" failure={error} onRetry={retry} />
        : !ready
          ? <WidgetState kind="loading" label="Loading spending…" />
          : reason
            ? <WidgetState kind="empty" message={reason} />
            : !tags.length
              ? <WidgetState kind="empty" message="Choose a country to see its political power spending." />
              : !ledgers
                ? <WidgetState kind="loading" label="Loading political power spending…" />
                : null;

  const groupNotice = groupError && <WidgetState kind="error" failure={groupError} onRetry={retry} />;

  return (
    <div style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
      {groupNotice}
      <FamilyBreakdownHost
        declaration={ppSpendingProjection}
        options={projectionOptions}
        view={view}
        namesInRing={namesInRing}
        label="Political power spending breakdown"
        subject="political power spending"
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
        extraWarnings={groupWarnings}
        blockedState={blocked}
        loadingLabel="Loading political power spending…"
      />
    </div>
  );
}
