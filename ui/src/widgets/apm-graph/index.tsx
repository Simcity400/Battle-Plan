import type { WidgetRenderer } from '../../dashboard/definitions';
import { useApmChoices } from '../player-apm/useApm';
import { apmGraphDeclaration } from './apmGraphProjection';

export default { declaration: apmGraphDeclaration, useOptionChoices: useApmChoices } satisfies WidgetRenderer;
