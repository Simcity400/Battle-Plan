import type { WidgetRenderer } from '../../dashboard/definitions';
import { ApmWidget } from '../player-apm/ApmWidget';
import { useApmChoices } from '../player-apm/useApm';
export default { render: props => <ApmWidget {...props} graph />, useOptionChoices: useApmChoices } satisfies WidgetRenderer;
