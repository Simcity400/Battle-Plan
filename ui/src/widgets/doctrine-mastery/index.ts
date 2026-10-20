import type { WidgetRenderer } from '../../dashboard/definitions';
import DoctrineMasteryWidget from './DoctrineMasteryWidget';
import { useDoctrineChoices } from './useDoctrineMastery';
export default { render: DoctrineMasteryWidget, useOptionChoices: useDoctrineChoices } satisfies WidgetRenderer;
