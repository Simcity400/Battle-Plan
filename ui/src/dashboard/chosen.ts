import { contentKey, setToken } from './options.ts';


export interface ChoiceSource {
  lines: (token: string) => number;
  name: (token: string) => string;
  noun: string;
}

export const campaignToken = (campaignKey: string, token: string) => `${campaignKey}:${token}`;

export const subjectToken = (campaignKey: string, handle: string) => setToken(campaignToken(campaignKey, handle));

export function subjectHandle(campaignKey: string, value: string): string | null {
  const key = contentKey(value), prefix = campaignToken(campaignKey, '');
  return key.startsWith(prefix) ? key.slice(prefix.length) : null;
}
