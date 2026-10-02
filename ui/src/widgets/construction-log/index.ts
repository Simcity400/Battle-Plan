import type { WidgetRenderer } from '../../dashboard/definitions';
import ConstructionLogWidget from './ConstructionLogWidget';
import { useConstructionChoices } from './useConstructionLog';

export default { render: ConstructionLogWidget, useOptionChoices: useConstructionChoices } satisfies WidgetRenderer;
