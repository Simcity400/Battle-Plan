/* fonts.js — map the game bitmap-font NAMES to CSS so text fields render at the game's
 * sizes/weights/faces.
 *
 * The .gui files reference fonts by name (font="hoi_18mbs"). Each name is a the game
 * BMFont (.fnt) whose `info face=...` header tells us the real typeface + px size.
 * We resolve those from the EFFECTIVE gfx/fonts set:
 *   hoi_16mbs/18mbs/18/18b/20b/20bs -> "Ubuntu" / "Ubuntu Light" (16/18/20px, bold)
 *   hoi_24header/36header/30header  -> "Orator Std" display face (header sizes)
 *   vic_18 / garamond_14            -> Garamond (serif)
 *   cg_16b                          -> Century Gothic (geometric sans)
 *   hoi4_typewriter16/22, *typewriter -> Roboto Mono (the bundled assets/font.ttf,
 *                                        Apache-2.0, shipped under static/fonts/)
 *   Arial12 / Main_*                -> Arial / sans
 *
 * LICENSING / honesty: we do NOT bundle the proprietary DDS glyph atlases. Ubuntu
 * (UFL), Roboto Mono (Apache-2.0) are the legitimately-usable faces; everything
 * else degrades to the closest free web-stack at the correct px size, so layout
 * (maxWidth/positions) stays faithful even when the exact glyph face can't ship.
 * The @font-face for Roboto Mono + the family stacks are injected once on init.
 */
(function (global) {
  "use strict";

  // name -> { size(px), weight, stack }
  const STACK_SANS_GAME = '"Ubuntu","Ubuntu Light","Segoe UI",Tahoma,sans-serif';
  const STACK_HEADER = '"Oswald","Bebas Neue","Arial Narrow",Impact,sans-serif'; // ~ Orator Std display
  const STACK_SERIF = '"EB Garamond","Adobe Garamond Pro",Garamond,Georgia,serif';
  const STACK_GEO = '"Century Gothic","Questrial","Futura","Segoe UI",sans-serif';
  const STACK_MONO = '"the game Mono","Roboto Mono","Consolas",monospace';
  const STACK_ARIAL = 'Arial,Helvetica,sans-serif';

  const FONTS = {
    // ubuntu family — the bulk of the UI
    hoi_16mbs: { size: 16, weight: 700, stack: STACK_SANS_GAME },
    hoi_18mbs: { size: 18, weight: 700, stack: STACK_SANS_GAME },
    hoi_18:    { size: 18, weight: 400, stack: STACK_SANS_GAME },
    hoi_18b:   { size: 18, weight: 700, stack: STACK_SANS_GAME },
    hoi_20:    { size: 20, weight: 400, stack: STACK_SANS_GAME },
    hoi_20b:   { size: 20, weight: 700, stack: STACK_SANS_GAME },
    hoi_20bs:  { size: 20, weight: 700, stack: STACK_SANS_GAME },
    // headers — Orator Std display
    hoi_24header: { size: 24, weight: 700, stack: STACK_HEADER, spacing: 0.5 },
    hoi_30header: { size: 30, weight: 700, stack: STACK_HEADER, spacing: 0.5 },
    hoi_36header: { size: 36, weight: 400, stack: STACK_HEADER, spacing: 0.5 },
    // serif
    vic_18:      { size: 18, weight: 700, stack: STACK_SERIF },
    garamond_14: { size: 14, weight: 400, stack: STACK_SERIF },
    // geometric
    cg_16b: { size: 16, weight: 700, stack: STACK_GEO },
    // typewriter -> bundled Roboto Mono
    hoi4_typewriter16: { size: 16, weight: 400, stack: STACK_MONO },
    hoi4_typewriter22: { size: 22, weight: 400, stack: STACK_MONO },
    hoi_16typewriter:  { size: 16, weight: 400, stack: STACK_MONO },
    hoi_22typewriter:  { size: 22, weight: 400, stack: STACK_MONO },
    // arial / generic
    Arial12:      { size: 12, weight: 400, stack: STACK_ARIAL },
    Main_14_bold: { size: 14, weight: 700, stack: STACK_ARIAL },
    Main_14_black:{ size: 14, weight: 900, stack: STACK_ARIAL },
    Main_16:      { size: 16, weight: 400, stack: STACK_ARIAL },
  };

  const Fonts = {
    injected: false,

    /** Inject the @font-face for the bundled mono face once. */
    init() {
      if (this.injected) return;
      const css = `
@font-face{
  font-family:"the game Mono";
  src:url("/replay_game/static/fonts/roboto-mono.ttf") format("truetype");
  font-weight:300 700; font-display:swap;
}`;
      const st = document.createElement("style");
      st.id = "hoi4-fonts";
      st.textContent = css;
      document.head.appendChild(st);
      this.injected = true;
    },

    /** Resolve a the game font name to {size,weight,stack,spacing}. Falls back to ubuntu 16. */
    resolve(name) {
      if (name && FONTS[name]) return FONTS[name];
      // tolerate unknown header/typewriter/mbs variants by family hint
      const n = String(name || "").toLowerCase();
      if (n.includes("header")) return { size: 24, weight: 700, stack: STACK_HEADER };
      if (n.includes("typewriter")) return { size: 16, weight: 400, stack: STACK_MONO };
      if (n.startsWith("vic") || n.includes("garamond")) return { size: 16, weight: 400, stack: STACK_SERIF };
      return { size: 16, weight: 700, stack: STACK_SANS_GAME };
    },

    /** Apply a the game font name to a DOM element's style. */
    apply(el, name) {
      const f = this.resolve(name);
      el.style.fontFamily = f.stack;
      el.style.fontSize = f.size + "px";
      el.style.fontWeight = f.weight;
      el.style.lineHeight = f.size + "px";
      if (f.spacing) el.style.letterSpacing = f.spacing + "px";
    },

    map: FONTS,
  };

  global.Fonts = Fonts;
})(window);
