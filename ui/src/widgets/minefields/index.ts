import type { WidgetRenderer } from '../../dashboard/definitions';
import MinefieldsWidget from './MinefieldsWidget';
import { useMinefieldChoices } from './useMinefields';
export default { render: MinefieldsWidget, useOptionChoices: useMinefieldChoices } satisfies WidgetRenderer;
