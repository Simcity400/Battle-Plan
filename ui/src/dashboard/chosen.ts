import { contentKey, setToken } from './options.ts';

export const campaignToken = (campaignKey: string, token: string) => `${campaignKey}:${token}`;

export const subjectToken = (campaignKey: string, handle: string) => setToken(campaignToken(campaignKey, handle));

export function subjectHandle(campaignKey: string, value: string): string | null {
  const key = contentKey(value), prefix = campaignToken(campaignKey, '');
  return key.startsWith(prefix) ? key.slice(prefix.length) : null;
}
