import { useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { useHeldFailure } from '../../dashboard/heldFailures';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { PlaybackMode } from '../../charts/view';
import FuelConsumptionBreakdown from '../fuel/FuelConsumptionBreakdown';
import type { FuelConsumptionLevel, FuelConsumptionSelection } from '../fuel/fuelConsumption';
import { usePreparedFuel } from '../fuel/preparedFuel';

/** The Fuel widget's own current-consumption breakdown of one country at the drill's
 * date: its By military branch disclosure and each consumer's demand. */
export default function FuelConsumptionDrill({ runtime, track, tag, hour, extent, active, digits, onPick }: {
  runtime: WidgetRuntime; track: BoundTrack; tag: string; hour: number; extent: [number, number]; active: boolean; digits: number;
  onPick: (hour: number) => void;
}) {
  const fuel = usePreparedFuel(runtime.client, track, [tag], hour, active, ['fuel', 'capacity', 'stripGain', 'stripBurn']);
  const [selection, setSelection] = useState<FuelConsumptionSelection | null>(null);
  const [level, setLevel] = useState<FuelConsumptionLevel | null>(null);
  const reply = fuel.data?.find(item => item.tag === tag);
  const error = useHeldFailure(fuel.error, reply !== undefined, fuel.retry);
  if (error) return <WidgetState kind="error" detail={error} onRetry={fuel.retry} />;
  if (!reply) return <WidgetState kind="loading" label="Loading fuel consumption…" />;
  return <FuelConsumptionBreakdown reply={reply} runtime={runtime} track={track} hour={hour} digits={digits}
    selection={selection} onSelection={setSelection} level={level} onLevel={setLevel}
    active={active} pickOwner={tag} extent={extent} playhead={hour} playback={'cursor' as PlaybackMode} afterPlayhead
    windowHours={365 * 24} branchStart={track.branch.firstHour} hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond}
    onPick={onPick} />;
}
