export interface CountryFlagLook {
  nameFailure?: Error; flag?: string; flagReason?: string; flagDetail?: string; flagPending?: boolean }

export function countryFlagUnavailable(reason?: string, pending = false, decodeFailed = false) {
  const detail = decodeFailed ? 'Couldn’t display the flag image.' : reason;
  return { reason: detail, text: pending ? 'Flag loading' : 'Flag unavailable' };
}

export function fitCountryFlag(imageWidth: number, imageHeight: number, slotWidth: number, slotHeight: number) {
  const scale = Math.min(slotWidth / imageWidth, slotHeight / imageHeight);
  const width = imageWidth * scale, height = imageHeight * scale;
  return { x: (slotWidth - width) / 2, y: (slotHeight - height) / 2, width, height };
}
