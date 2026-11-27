import { LOADING_NAME } from '../families/table/identityCell.ts';
import type { Value } from '../families/value.ts';

export function landBattleTitleKey(key: string): string {
  return `battle:${key.slice(key.indexOf(':') + 1)}`;
}

export function battleTitleText(title: Value<string>): string {
  return title.kind === 'known' ? title.value : title.kind === 'pending' ? LOADING_NAME : 'Battle name not recorded';
}
