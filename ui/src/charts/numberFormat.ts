/** Shared number formats. `toLocaleString` given a locale or options builds a new
 * formatter on every call; these are built once and write exactly what it writes. */
export const usNumber = new Intl.NumberFormat('en-US');
export const usWhole = new Intl.NumberFormat('en-US', { maximumFractionDigits: 0 });
export const usFiveDecimals = new Intl.NumberFormat('en-US', { maximumFractionDigits: 5 });
export const gbNumber = new Intl.NumberFormat('en-GB');
export const gbTwoSignificant = new Intl.NumberFormat('en-GB', { maximumSignificantDigits: 2 });
export const enTwoDecimals = new Intl.NumberFormat('en', { maximumFractionDigits: 2 });
