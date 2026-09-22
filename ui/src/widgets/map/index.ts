import type { WidgetRenderer } from '../../dashboard/definitions';
import MapWidget from './MapWidget';
import MapWidgetControls from './MapWidgetControls';

export default { render: MapWidget, headerControls: MapWidgetControls } satisfies WidgetRenderer;
