/** Presence denotes a native country flag, independently of an available URL. */
export interface CountryFlagLook { flag?: string; flagReason?: string; flagPending?: boolean }

/** One unavailable explanation for HTML flags and canvas decoding. */
export function countryFlagUnavailable(reason?: string, pending = false, decodeFailed = false) {
  const label = pending ? 'Flag loading' : 'Flag not available';
  const detail = decodeFailed ? 'The flag image could not be displayed.' : reason;
  return { reason: detail, text: detail ? `${label}: ${detail}` : label };
}
