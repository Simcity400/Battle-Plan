import type { WidgetRenderer } from '../../dashboard/definitions';
import { apmTableDeclaration } from '../player-apm/apmProjection';
import { useApmChoices } from '../player-apm/useApm';

export default { declaration: apmTableDeclaration, useOptionChoices: useApmChoices } satisfies WidgetRenderer;
