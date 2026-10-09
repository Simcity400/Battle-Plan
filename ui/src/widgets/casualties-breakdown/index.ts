import type { WidgetRenderer } from '../../dashboard/definitions';
import CasualtiesBreakdownWidget from './CasualtiesBreakdownWidget';
export { definition } from './definition';
export default { render: CasualtiesBreakdownWidget } satisfies WidgetRenderer;
