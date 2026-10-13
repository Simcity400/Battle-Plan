import type { WidgetRenderer } from '../../dashboard/definitions';
import ResearchWidget from './ResearchWidget';
import { useResearchChoices } from './useResearch';

export default { render: ResearchWidget, useOptionChoices: useResearchChoices } satisfies WidgetRenderer;
