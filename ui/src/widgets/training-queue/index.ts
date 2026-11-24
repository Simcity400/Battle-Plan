import type { WidgetRenderer } from '../../dashboard/definitions';
import { trainingQueueDeclaration } from './trainingQueueProjection';

export default { declaration: trainingQueueDeclaration } satisfies WidgetRenderer;
