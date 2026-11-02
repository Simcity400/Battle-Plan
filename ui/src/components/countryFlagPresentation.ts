/** Presence denotes a native country flag, independently of an available URL. */
export interface CountryFlagLook {
  nameFailure?: Error; flag?: string; flagReason?: string; flagDetail?: string; flagPending?: boolean }

/** One unavailable explanation for HTML flags and canvas decoding. */
export function countryFlagUnavailable(reason?: string, pending = false, decodeFailed = false) {
  const detail = decodeFailed ? 'The flag image could not be displayed.' : reason;
  return { reason: detail, text: pending ? 'Flag loading' : 'Flag unavailable' };
}

/** Fit the content's flag dimensions inside its slot without cropping or distortion. */
export function fitCountryFlag(imageWidth: number, imageHeight: number, slotWidth: number, slotHeight: number) {
  const scale = Math.min(slotWidth / imageWidth, slotHeight / imageHeight);
  const width = imageWidth * scale, height = imageHeight * scale;
  return { x: (slotWidth - width) / 2, y: (slotHeight - height) / 2, width, height };
}
