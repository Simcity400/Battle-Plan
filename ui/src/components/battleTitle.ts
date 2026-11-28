import { LOADING_NAME } from '../families/table/identityCell.ts';
import type { Value } from '../families/value.ts';

export function battleTitleText(title: Value<string>): string {
  return title.kind === 'known' ? title.value : title.kind === 'pending' ? LOADING_NAME : 'Battle name not recorded';
}
