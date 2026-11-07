import { createElement } from 'react';
import type { WidgetProps, WidgetRenderer } from '../../dashboard/definitions';
import { LIVE_WIDGETS } from '../../live/liveSwitch.ts';
import CampaignClockWidget from './CampaignClockWidget';
import CanvasCampaignClock from './CanvasCampaignClock';

/** The canvas table while following playback; a pinned or ranged clock keeps the DOM table. */
function CampaignClock(props: WidgetProps) {
  return createElement(LIVE_WIDGETS && props.widget.time.mode === 'follow' ? CanvasCampaignClock : CampaignClockWidget, props);
}
export default { render: CampaignClock } satisfies WidgetRenderer;
