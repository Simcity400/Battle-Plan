import type { WidgetRenderer } from '../../dashboard/definitions';
import { researchDeclaration } from './researchProjection';
import { useResearchChoices } from './useResearch';

export default {
  declaration: researchDeclaration,
  useOptionChoices: useResearchChoices,
} satisfies WidgetRenderer;
