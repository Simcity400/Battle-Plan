import type { WidgetRenderer } from '../../dashboard/definitions';
import EspionageLogWidget from './EspionageLogWidget';
import { useEspionageChoices } from './useEspionageTypes';
export default { render: EspionageLogWidget, useOptionChoices: useEspionageChoices } satisfies WidgetRenderer;
