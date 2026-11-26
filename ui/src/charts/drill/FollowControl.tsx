import { dayLabel } from '../draw.ts';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { Region } from '../../dashboard/regionTime';
import { useRegionHour } from '../../dashboard/widgetTime.ts';
import './drill.css';

export default function FollowControl({ following, onFollow, onPause }: {
  following: boolean; onFollow: () => void; onPause?: () => void;
}) {
  return <button type="button" className="button small quiet drill-follow" aria-label="Follow playback" aria-pressed={following} disabled={following && !onPause}
    onClick={following && onPause ? onPause : onFollow}>Follow</button>;
}

export function DrawerDayLabel({ runtime, region, active, fallback = '' }: {
  readonly runtime: WidgetRuntime;
  readonly region: Region;
  readonly active: boolean;
  readonly fallback?: string;
}) {
  const hour = useRegionHour(runtime, region, active);
  return <>{hour === null ? fallback : dayLabel(Math.floor(hour))}</>;
}
