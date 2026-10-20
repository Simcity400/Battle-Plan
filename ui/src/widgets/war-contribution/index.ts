import type { WidgetRenderer } from '../../dashboard/definitions';
import WarContributionWidget from './WarContributionWidget';
import { useWarChoices } from './useWarChoices';

export default { render: WarContributionWidget, useOptionChoices: useWarChoices } satisfies WidgetRenderer;
