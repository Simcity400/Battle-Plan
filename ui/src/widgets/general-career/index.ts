import type { WidgetRenderer } from '../../dashboard/definitions';
import GeneralCareerWidget from './GeneralCareerWidget';
import { useLeaderChoices } from './useLeaderChoices.ts';

export default { render: GeneralCareerWidget, useOptionChoices: useLeaderChoices } satisfies WidgetRenderer;
