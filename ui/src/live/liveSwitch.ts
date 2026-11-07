/** Whether table widgets paint their rows in a canvas from hours read ahead (the default), or keep
 * their DOM tables: `battle-plan.live-widgets` = "0" keeps the DOM tables, for measuring one against the other. */
export const LIVE_WIDGETS = (() => {
  try { return globalThis.localStorage?.getItem('battle-plan.live-widgets') !== '0'; } catch { return true; }
})();
