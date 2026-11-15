import type { WidgetRenderer } from '../../dashboard/definitions';
import CampaignClockWidget from './CampaignClockWidget';

/** Following, its totals at the head are a followed value painted into the table; pinned or ranged, asked for. */
export default { render: CampaignClockWidget } satisfies WidgetRenderer;
