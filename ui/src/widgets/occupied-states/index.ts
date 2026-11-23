import type { WidgetRenderer } from '../../dashboard/definitions';
import { occupiedStatesDeclaration } from './occupiedProjection';
import './occupiedStates.css';

export default { declaration: occupiedStatesDeclaration } satisfies WidgetRenderer;
