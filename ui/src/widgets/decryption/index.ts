import type { WidgetRenderer } from '../../dashboard/definitions';
import { decryptionDeclaration } from './decryptionProjection.ts';

export default { declaration: decryptionDeclaration } satisfies WidgetRenderer;
