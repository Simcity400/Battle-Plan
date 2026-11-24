import type { WidgetRenderer } from '../../dashboard/definitions';
import { warContributionSidesDeclaration } from './sidesProjection.ts';
import { useWarChoices } from './useWarChoices';

export default { declaration: warContributionSidesDeclaration, useOptionChoices: useWarChoices } satisfies WidgetRenderer;
