/* map_gl.js — WebGL recolor backend for the replay map.
 *
 * WHY: the Canvas2D path in map.js recolors by a per-pixel CPU loop over the
 * 2816x1024 id raster on every repaint (mode switch / scrub / playback frame).
 * That's ~2.9M iterations + a putImageData each frame. This module moves the
 * recolor to a fragment shader: the GPU does the per-pixel work, the CPU only
 * re-uploads a tiny color LUT (≤ ~13.4k texels) per frame.
 *
 * HOW (province-id texture + lookup textures — exactly how the game renders):
 *   - The province PNG already encodes province_id in RGB
 *     (id = R + G*256 + B*65536). We upload it RAW (UNSIGNED_BYTE RGBA, NEAREST,
 *     no mipmaps, no premultiply, no sRGB/colorspace conversion) and reconstruct
 *     the id in the shader. The CPU idRaster in map.js is KEPT for picking/centroids.
 *   - prov->state LUT (uProvState): a static UNSIGNED_BYTE RGBA texture, one texel
 *     per province_id, storing (state_id + 1) split low/high byte into R,G
 *     (0 == "no state"). Built ONCE.
 *   - color LUT (uColorLUT): a per-frame UNSIGNED_BYTE RGBA texture. The key is
 *     state_id for per-state modes, province_id for perProvince modes. RGB = color,
 *     A = 255 means "present" (distinguishes a real color from UNKNOWN). Re-uploaded
 *     each recolor; never any per-pixel CPU work.
 *
 * SEA / UNKNOWN are reproduced EXACTLY as the Canvas path:
 *   - id == 0                      -> SEA
 *   - per-state and state_id < 0   -> SEA  (matches map.js: key<0 paints SEA)
 *   - key present in color LUT     -> that color
 *   - key absent                   -> UNKNOWN
 * SEA and UNKNOWN colors are passed as uniforms so they stay in lockstep with map.js.
 *
 * FALLBACK: if WebGL is unavailable, or context creation / shader compile / a
 * required-extension probe fails, init() returns null and map.js uses its existing
 * CPU recolor. This module never throws into the render loop.
 *
 * Color exactness: all colors travel as UNSIGNED_BYTE (0..255) with NEAREST
 * sampling and integer texel-center addressing, so the GPU reproduces the same
 * bytes the CPU path packs. No float textures are required (avoids the
 * OES_texture_float / no-filter portability minefield entirely).
 */
(function (global) {
  "use strict";

  // GLSL ES 1.00 (works on both WebGL1 and WebGL2). We index LUT textures by
  // integer id decomposed into (x,y) texel coordinates and sample at texel
  // CENTERS ((i + 0.5)/dim) with NEAREST filtering — exact, no interpolation.
  const VERT = [
    "attribute vec2 aPos;",
    "varying vec2 vUV;",
    "void main(){",
    // aPos in [0,1]; flip Y so texture row 0 (north) is at the top, matching
    // Canvas putImageData orientation.
    "  vUV = vec2(aPos.x, 1.0 - aPos.y);",
    "  gl_Position = vec4(aPos * 2.0 - 1.0, 0.0, 1.0);",
    "}",
  ].join("\n");

  const FRAG = [
    // Fragment highp is OPTIONAL in GLSL ES 1.00; if it isn't present, mediump's
    // ~10-bit mantissa cannot hold province ids up to ~13413 exactly (Codex CRIT
    // #1/#2). We #error out so link fails and the caller falls back to CPU. The JS
    // side ALSO probes getShaderPrecisionFormat before compiling, so this is a
    // belt-and-braces guard. Sampler return precision follows the SAMPLER's
    // declared precision (default lowp in fragment shaders) — so each sampler is
    // declared highp explicitly, else texture bytes could come back quantized.
    "#ifdef GL_FRAGMENT_PRECISION_HIGH",
    "precision highp float;",
    "precision highp int;",
    "#else",
    "#error fragment highp required",
    "#endif",
    "varying vec2 vUV;",
    "uniform highp sampler2D uIds;",        // province-id raster (raw RGB encodes id)
    "uniform highp sampler2D uProvState;",  // province_id -> (state_id+1) lo/hi in R,G
    "uniform highp sampler2D uColorLUT;",   // key -> RGB (+A=present)
    "uniform vec2 uProvStateDim;",    // dims of the prov->state LUT texture
    "uniform vec2 uColorDim;",        // dims of the color LUT texture
    "uniform float uProvStateMax;",   // highest valid province index in prov->state LUT
    "uniform float uColorMax;",       // highest valid key index in color LUT
    "uniform float uPerProvince;",    // 1.0 = key by province, 0.0 = key by state
    "uniform vec3 uSea;",             // sea color (0..1)
    "uniform vec3 uUnknown;",         // unknown color (0..1)
    "",
    // Decode a province id from a raw RGB sample (channels are 0..1 floats of the
    // original bytes). Round each channel to the nearest integer byte before
    // weighting to avoid float drift (highp gives >=16-bit mantissa; ids <= ~2^14,
    // so 1.0*R + 256.0*G + 65536.0*B is exact once channels are integer-rounded).
    "float decodeId(vec3 c){",
    "  float r = floor(c.r * 255.0 + 0.5);",
    "  float g = floor(c.g * 255.0 + 0.5);",
    "  float b = floor(c.b * 255.0 + 0.5);",
    "  return r + g * 256.0 + b * 65536.0;",
    "}",
    "",
    // Sample a LUT texture at integer index `idx` (texel center), dims `dim`.
    // Row math is hardened against exact-boundary float error (Codex MODERATE #4):
    // compute y from (idx+0.5)/width then x = idx - y*width (avoids mod(idx,width)
    // returning width at row boundaries).
    "vec4 lutAt(highp sampler2D tex, float idx, vec2 dim){",
    "  float y = floor((idx + 0.5) / dim.x);",
    "  float x = idx - y * dim.x;",
    "  vec2 uv = (vec2(x, y) + 0.5) / dim;",
    "  return texture2D(tex, uv);",
    "}",
    "",
    "void main(){",
    "  vec3 idc = texture2D(uIds, vUV).rgb;",
    "  float pid = decodeId(idc);",
    "  if (pid < 0.5) { gl_FragColor = vec4(uSea, 1.0); return; }", // id 0 -> sea
    "  float key;",
    "  if (uPerProvince > 0.5) {",
    "    key = pid;",
    "  } else {",
    // prov -> state via static LUT. Stored as (state_id + 1); 0 means no state.
    "    if (pid > uProvStateMax) { gl_FragColor = vec4(uSea, 1.0); return; }",
    "    vec4 ps = lutAt(uProvState, pid, uProvStateDim);",
    "    float lo = floor(ps.r * 255.0 + 0.5);",
    "    float hi = floor(ps.g * 255.0 + 0.5);",
    "    float sidPlus1 = lo + hi * 256.0;",
    "    if (sidPlus1 < 0.5) { gl_FragColor = vec4(uSea, 1.0); return; }", // state_id < 0 -> sea
    "    key = sidPlus1 - 1.0;",
    "  }",
    "  if (key > uColorMax) { gl_FragColor = vec4(uUnknown, 1.0); return; }",
    "  vec4 col = lutAt(uColorLUT, key, uColorDim);",
    "  if (col.a < 0.5) { gl_FragColor = vec4(uUnknown, 1.0); return; }", // key absent -> unknown
    "  gl_FragColor = vec4(col.rgb, 1.0);",
    "}",
  ].join("\n");

  function compile(gl, type, src) {
    const sh = gl.createShader(type);
    gl.shaderSource(sh, src);
    gl.compileShader(sh);
    if (!gl.getShaderParameter(sh, gl.COMPILE_STATUS)) {
      const log = gl.getShaderInfoLog(sh);
      gl.deleteShader(sh);
      throw new Error("shader compile failed: " + log);
    }
    return sh;
  }

  function link(gl, vs, fs) {
    const p = gl.createProgram();
    gl.attachShader(p, vs); gl.attachShader(p, fs);
    gl.bindAttribLocation(p, 0, "aPos");
    gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) {
      const log = gl.getProgramInfoLog(p);
      gl.deleteProgram(p);
      throw new Error("program link failed: " + log);
    }
    return p;
  }

  // Choose LUT texture dims that cover [0..maxId] with a near-square shape and a
  // power-of-two-ish width for clean addressing (any width works with NEAREST).
  function lutDims(maxId) {
    const need = maxId + 1;
    const width = 256;                       // fixed width; height grows as needed
    const height = Math.max(1, Math.ceil(need / width));
    return { width, height, capacity: width * height };
  }

  /**
   * Try to build a GL recolor backend bound to `canvas`, sampling the province ids
   * from `idImage` (an ImageData of EXACTLY the bytes used to build the CPU idRaster
   * — uploading these, not the <img>, removes any Canvas2D-vs-WebGL decode
   * divergence) and using `stateByProvince` (Int32Array) for the static prov->state
   * LUT.
   *
   * Returns an object { recolor(packedColorByKey, perProvince, seaRGB, unknownRGB),
   * dispose() } or null if WebGL is unavailable / setup failed (caller falls back).
   */
  function create(canvas, idImage, stateByProvince, maxProvinceId) {
    let gl = null;
    try {
      const opts = { antialias: false, depth: false, stencil: false,
                     premultipliedAlpha: false, preserveDrawingBuffer: false,
                     alpha: false, desynchronized: false };
      gl = canvas.getContext("webgl2", opts) || canvas.getContext("webgl", opts)
        || canvas.getContext("experimental-webgl", opts);
    } catch (e) { gl = null; }
    if (!gl) return null;
    // Mark the canvas as GL-tainted: even if we bail below, getContext("2d") on it
    // will now fail, so map.js must discard this canvas before its CPU fallback.
    canvas.__glTried = true;

    // Require fragment-shader float precision sufficient to represent the largest
    // province id EXACTLY. mediump's ~10-bit mantissa can't hold ids up to ~13413,
    // so the id-decode would index the wrong province; we bail to CPU rather than
    // render wrong colors. The gate is DYNAMIC to this dataset (Codex MEDIUM #2):
    // we need a mantissa covering ceil(log2(maxProvId+1)) bits, plus margin for the
    // intermediate g*256 / b*65536 terms (decode multiplies by 65536 == +16 bits).
    // A conforming highp reporting precision 16 can be enough for small ids; we ask
    // for (bits-to-hold-the-decoded-id) + a safety margin and still require that the
    // float RANGE can represent it.
    try {
      const fhp = gl.getShaderPrecisionFormat(gl.FRAGMENT_SHADER, gl.HIGH_FLOAT);
      // bits needed to hold the FULLY DECODED id value exactly (the decode forms
      // r + g*256 + b*65536, whose worst case is the id itself; ids are < 2^14 here).
      const needBits = Math.ceil(Math.log2((maxProvinceId || 1) + 1)) + 2; // +2 margin
      const ok = fhp && fhp.precision >= needBits && fhp.rangeMax >= needBits;
      if (!ok) {
        if (global.console && console.warn) console.warn("[map_gl] fragment float precision insufficient (have " + (fhp ? fhp.precision : "n/a") + " bits, need " + needBits + "), falling back to CPU");
        return null;
      }
    } catch (e) { return null; }

    let prog, idsTex, provStateTex, colorTex, quad;
    try {
      const vs = compile(gl, gl.VERTEX_SHADER, VERT);
      const fs = compile(gl, gl.FRAGMENT_SHADER, FRAG);
      prog = link(gl, vs, fs);
      gl.deleteShader(vs); gl.deleteShader(fs);

      // Critical: never let the driver mangle the id bytes.
      gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
      gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
      // WebGL1 has no UNPACK_COLORSPACE_CONVERSION constant guarantee, but the
      // default NONE is what we want; set it where available.
      if (gl.UNPACK_COLORSPACE_CONVERSION_WEBGL !== undefined && gl.NONE !== undefined) {
        gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, gl.NONE);
      }
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);

      // --- id texture: upload the PNG raw as RGBA / UNSIGNED_BYTE ---
      idsTex = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, idsTex);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      // Upload the exact decoded ImageData bytes (TexImageSource form: width/height
      // come from the ImageData itself; pixel-store flags above still apply).
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, idImage);

      // --- static prov->state LUT ---
      const psDim = lutDims(maxProvinceId);
      const psBuf = new Uint8Array(psDim.capacity * 4); // zero = "no state"
      const lim = Math.min(stateByProvince.length, maxProvinceId + 1);
      for (let p = 0; p < lim; p++) {
        const sid = stateByProvince[p];
        if (sid >= 0) {
          const v = sid + 1;                 // 0 reserved for "no state"
          const o = p * 4;
          psBuf[o] = v & 0xff;               // R = low byte
          psBuf[o + 1] = (v >> 8) & 0xff;    // G = high byte
        }
      }
      provStateTex = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, provStateTex);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, psDim.width, psDim.height, 0,
        gl.RGBA, gl.UNSIGNED_BYTE, psBuf);

      // --- color LUT (per frame); sized to cover the widest key (provinces) ---
      const colDim = lutDims(maxProvinceId);
      const colBuf = new Uint8Array(colDim.capacity * 4); // A=0 -> "absent" -> UNKNOWN
      colorTex = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, colorTex);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, colDim.width, colDim.height, 0,
        gl.RGBA, gl.UNSIGNED_BYTE, colBuf);

      // --- fullscreen quad ---
      quad = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, quad);
      gl.bufferData(gl.ARRAY_BUFFER,
        new Float32Array([0, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 1]), gl.STATIC_DRAW);

      // Disable every pipeline stage that could perturb the written bytes. DITHER
      // is enabled by default and may legally jiggle low bits on an 8-bit target;
      // blend/scissor/depth/stencil must be off for byte-for-byte output (Codex
      // CRITICAL #3). This context is ours alone, so setting it once is enough.
      gl.disable(gl.BLEND);
      gl.disable(gl.DITHER);
      gl.disable(gl.SCISSOR_TEST);
      gl.disable(gl.DEPTH_TEST);
      gl.disable(gl.STENCIL_TEST);
      gl.disable(gl.CULL_FACE);
      gl.colorMask(true, true, true, true);

      // --- uniform/attrib locations + constant uniforms ---
      gl.useProgram(prog);
      const loc = {
        aPos: gl.getAttribLocation(prog, "aPos"),
        uIds: gl.getUniformLocation(prog, "uIds"),
        uProvState: gl.getUniformLocation(prog, "uProvState"),
        uColorLUT: gl.getUniformLocation(prog, "uColorLUT"),
        uProvStateDim: gl.getUniformLocation(prog, "uProvStateDim"),
        uColorDim: gl.getUniformLocation(prog, "uColorDim"),
        uProvStateMax: gl.getUniformLocation(prog, "uProvStateMax"),
        uColorMax: gl.getUniformLocation(prog, "uColorMax"),
        uPerProvince: gl.getUniformLocation(prog, "uPerProvince"),
        uSea: gl.getUniformLocation(prog, "uSea"),
        uUnknown: gl.getUniformLocation(prog, "uUnknown"),
      };
      gl.uniform1i(loc.uIds, 0);
      gl.uniform1i(loc.uProvState, 1);
      gl.uniform1i(loc.uColorLUT, 2);
      gl.uniform2f(loc.uProvStateDim, psDim.width, psDim.height);
      gl.uniform2f(loc.uColorDim, colDim.width, colDim.height);
      gl.uniform1f(loc.uProvStateMax, maxProvinceId);
      gl.uniform1f(loc.uColorMax, colDim.capacity - 1);

      // sanity: did anything above raise a GL error?
      const err = gl.getError();
      if (err !== gl.NO_ERROR) throw new Error("GL setup error 0x" + err.toString(16));

      const state = { gl, prog, idsTex, provStateTex, colorTex, quad, loc,
                      colDim, colBuf, w: idImage.width, h: idImage.height };

      return {
        backend: gl instanceof (global.WebGL2RenderingContext || function () {}) ? "webgl2" : "webgl1",

        /** The raw GL context — exposed only for the dev/QA pixel-diff self-test. */
        glctx() { return state.gl; },

        /**
         * Recolor the whole map.
         * @param packedByKey Map<int key, int packedRGBA> (same packing map.js uses;
         *   we only read the low 3 bytes as R,G,B). Keys are state ids (per-state)
         *   or province ids (perProvince).
         * @param perProvince boolean
         * @param seaRGB [r,g,b] 0..255
         * @param unknownRGB [r,g,b] 0..255
         */
        recolor(packedByKey, perProvince, seaRGB, unknownRGB) {
          const g = state.gl;
          // 1) refresh the color LUT bytes (CPU touches only present keys + a clear)
          const buf = state.colBuf;
          buf.fill(0); // A=0 everywhere -> default UNKNOWN for absent keys
          const cap = state.colDim.capacity;
          packedByKey.forEach((packed, key) => {
            if (key < 0 || key >= cap) return;
            const o = key * 4;
            buf[o] = packed & 0xff;            // R
            buf[o + 1] = (packed >> 8) & 0xff; // G
            buf[o + 2] = (packed >> 16) & 0xff;// B
            buf[o + 3] = 255;                  // present
          });
          g.bindTexture(g.TEXTURE_2D, state.colorTex);
          g.pixelStorei(g.UNPACK_ALIGNMENT, 1);
          g.texSubImage2D(g.TEXTURE_2D, 0, 0, 0, state.colDim.width,
            state.colDim.height, g.RGBA, g.UNSIGNED_BYTE, buf);

          // 2) draw
          g.viewport(0, 0, state.w, state.h);
          g.useProgram(state.prog);
          g.uniform1f(state.loc.uPerProvince, perProvince ? 1.0 : 0.0);
          g.uniform3f(state.loc.uSea, seaRGB[0] / 255, seaRGB[1] / 255, seaRGB[2] / 255);
          g.uniform3f(state.loc.uUnknown, unknownRGB[0] / 255, unknownRGB[1] / 255, unknownRGB[2] / 255);

          g.activeTexture(g.TEXTURE0); g.bindTexture(g.TEXTURE_2D, state.idsTex);
          g.activeTexture(g.TEXTURE1); g.bindTexture(g.TEXTURE_2D, state.provStateTex);
          g.activeTexture(g.TEXTURE2); g.bindTexture(g.TEXTURE_2D, state.colorTex);

          g.bindBuffer(g.ARRAY_BUFFER, state.quad);
          g.enableVertexAttribArray(state.loc.aPos);
          g.vertexAttribPointer(state.loc.aPos, 2, g.FLOAT, false, 0, 0);
          g.drawArrays(g.TRIANGLES, 0, 6);
        },

        dispose() {
          const g = state.gl;
          try {
            g.deleteTexture(state.idsTex);
            g.deleteTexture(state.provStateTex);
            g.deleteTexture(state.colorTex);
            g.deleteBuffer(state.quad);
            g.deleteProgram(state.prog);
          } catch (e) { /* ignore */ }
        },
      };
    } catch (e) {
      // Clean up partial allocations and signal fallback.
      try {
        if (idsTex) gl.deleteTexture(idsTex);
        if (provStateTex) gl.deleteTexture(provStateTex);
        if (colorTex) gl.deleteTexture(colorTex);
        if (quad) gl.deleteBuffer(quad);
        if (prog) gl.deleteProgram(prog);
      } catch (e2) { /* ignore */ }
      if (global.console && console.warn) console.warn("[map_gl] setup failed, falling back to CPU:", e.message);
      return null;
    }
  }

  global.MapGL = { create };
})(window);
