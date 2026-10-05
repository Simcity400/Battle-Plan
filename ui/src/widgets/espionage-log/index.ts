import type { WidgetRenderer } from '../../dashboard/definitions';
import EspionageLogWidget from './EspionageLogWidget';
import { useEspionageChoices } from './useEspionageLog';
export default { render: EspionageLogWidget, useOptionChoices: useEspionageChoices } satisfies WidgetRenderer;
