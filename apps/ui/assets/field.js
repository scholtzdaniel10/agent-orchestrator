// Contour-map background for the empty workspace. Draws on every `canvas.ascii-field`.
// `data-agents` on the canvas, e.g. "claude:working codex:error": working agents raise a
// hill in their colour with rings, resting ones sink, failed ones turn rough and red.
// Off when local storage orch.background is "off". Still when the OS asks for reduced motion.
'use strict';
(() => {
  if (window.__legateField) return;
  window.__legateField = true;

  const SPOTS = {
    claude: [0.17, 0.18], codex: [0.84, 0.2], cursor: [0.85, 0.86],
    grok: [0.15, 0.86], lead: [0.5, 0.1], local: [0.5, 0.92],
  };
  const TOKENS = { text: '--text', bg: '--bg', bad: '--bad', lead: '--ray', claude: '--bot-claude', cursor: '--bot-cursor', grok: '--bot-grok' };
  const LABELS = { working: 'working', resting: 'queued', error: 'failed' };
  const N = 8, R = 150, PX = 12, FRAME_MS = 50;
  const still = matchMedia('(prefers-reduced-motion: reduce)');
  const started = new Map(); // agent -> time it started working, for the rings
  const t0 = performance.now();
  let colors = null, colorsAt = -1e9, metrics = null, font = '';

  function hash(x, y) {
    let h = (x | 0) * 374761393 + (y | 0) * 668265263;
    h = Math.imul(h ^ (h >>> 13), 1274126177);
    return ((h ^ (h >>> 16)) >>> 0) / 4294967295;
  }
  function vnoise(x, y) {
    const xi = Math.floor(x), yi = Math.floor(y), xf = x - xi, yf = y - yi;
    const u = xf * xf * (3 - 2 * xf), v = yf * yf * (3 - 2 * yf);
    const a = hash(xi, yi), b = hash(xi + 1, yi), c = hash(xi, yi + 1), d = hash(xi + 1, yi + 1);
    return a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v;
  }
  function fbm(x, y) {
    let s = 0, a = 0.5, f = 1;
    for (let o = 0; o < 3; o++) { s += a * vnoise(x * f, y * f); f *= 2.03; a *= 0.5; }
    return s;
  }
  const mix = (a, b, t) => a.map((v, i) => v + (b[i] - v) * t);
  const rgba = (c, a) => `rgba(${c[0] | 0},${c[1] | 0},${c[2] | 0},${a})`;

  // Theme colours, resolved through the element so color-mix() and var() come back as rgb().
  function readColors(el) {
    const css = getComputedStyle(el);
    const out = {};
    for (const [key, token] of Object.entries(TOKENS)) {
      el.style.color = `var(${token})`;
      out[key] = (getComputedStyle(el).color.match(/[\d.]+/g) || [128, 128, 128]).slice(0, 3).map(Number);
    }
    el.style.color = '';
    out.codex = out.text;
    out.local = out.text;
    out.muted = mix(out.text, out.bg, 0.36);
    const mono = css.getPropertyValue('--font-mono').trim() || 'Consolas, monospace';
    return { out, font: `500 ${PX}px ${mono}` };
  }

  function cellMetrics(g) {
    g.font = font;
    const w = g.measureText('M').width;
    const cap = g.measureText('H').actualBoundingBoxAscent || PX * 0.7;
    const h = Math.round(PX * 1.2 * 10) / 10;
    return { w, h, base: (h + cap) / 2 };
  }

  function parseAgents(text) {
    return (text || '').split(/\s+/).filter(Boolean).map((item) => {
      const [id, state] = item.split(':');
      return { id, state };
    }).filter((a) => SPOTS[a.id] && LABELS[a.state]);
  }

  function draw(cv, t, now) {
    const W = cv.clientWidth, H = cv.clientHeight;
    if (!W || !H) return;
    const dpr = window.devicePixelRatio || 1;
    if (cv.width !== Math.round(W * dpr) || cv.height !== Math.round(H * dpr)) {
      cv.width = Math.round(W * dpr); cv.height = Math.round(H * dpr);
    }
    const g = cv.getContext('2d');
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.clearRect(0, 0, W, H);
    if (now - colorsAt > 1000 || !colors) {
      const read = readColors(cv);
      colors = read.out; colorsAt = now;
      if (read.font !== font) { font = read.font; metrics = null; }
    }
    if (!metrics) metrics = cellMetrics(g);
    const m = metrics, C = colors;
    g.font = font; g.textBaseline = 'alphabetic';

    // Keep the heading and buttons clear: fade out near the canvas's siblings.
    const box = cv.getBoundingClientRect();
    let clear = null;
    for (const el of cv.parentElement.children) {
      if (el === cv) continue;
      const b = el.getBoundingClientRect();
      if (!b.width && !b.height) continue;
      const r = { left: b.left - box.left, top: b.top - box.top, right: b.right - box.left, bottom: b.bottom - box.top };
      clear = clear ? {
        left: Math.min(clear.left, r.left), top: Math.min(clear.top, r.top),
        right: Math.max(clear.right, r.right), bottom: Math.max(clear.bottom, r.bottom),
      } : r;
    }
    const fade = (x, y) => {
      if (!clear) return 1;
      const dx = Math.max(clear.left - 24 - x, 0, x - clear.right - 24), dy = Math.max(clear.top - 24 - y, 0, y - clear.bottom - 24);
      return Math.min(1, Math.hypot(dx, dy) / 90);
    };

    const agents = parseAgents(cv.dataset.agents);
    for (const a of agents) {
      if (a.state === 'working' && !started.has(a.id)) started.set(a.id, t);
      if (a.state !== 'working') started.delete(a.id);
    }
    for (const id of [...started.keys()]) if (!agents.some((a) => a.id === id)) started.delete(id);
    const hills = agents.map((a) => ({
      a, color: C[a.id] || C.text, emit: started.get(a.id) || 0,
      px: W * SPOTS[a.id][0], py: H * SPOTS[a.id][1],
    }));

    // Heights on cell corners; a cell gets a glyph only where a contour crosses it (marching squares).
    const cols = Math.ceil(W / m.w), rows = Math.ceil(H / m.h), VC = cols + 1, K = hills.length;
    const height = new Float32Array(VC * (rows + 1)), inf = new Float32Array(VC * (rows + 1) * Math.max(K, 1));
    for (let r = 0; r <= rows; r++) for (let c = 0; c <= cols; c++) {
      const x = c * m.w, y = r * m.h, i = r * VC + c;
      const px = x / 520, py = y / 520;
      const qx = fbm(px + t * 0.012, py), qy = fbm(px + 5.2, py + 1.3 - t * 0.01);
      let h = fbm(px + 1.3 * qx, py + 1.3 * qy);
      for (let k = 0; k < K; k++) {
        const hl = hills[k], d = Math.hypot(x - hl.px, y - hl.py), w = Math.exp(-((d / R) ** 2));
        inf[i * K + k] = w;
        if (hl.a.state === 'working') {
          h += 0.3 * (0.9 + 0.1 * Math.sin(t * 1.4 + k)) * w;
          if (!still.matches) for (let n = 0; n < 2; n++) {
            const age = ((t - hl.emit) % 3.2 + 3.2) % 3.2 + n * 3.2;
            h += 0.07 * Math.exp(-(((d - age * 85) / 22) ** 2)) * Math.exp(-age / 2.2);
          }
        } else if (hl.a.state === 'resting') h -= 0.08 * w;
        else h += 0.12 * w * (vnoise(x / 26, y / 26 + t * 4) - 0.5);
      }
      height[i] = h * N;
    }

    for (let r = 0; r < rows; r++) for (let c = 0; c < cols; c++) {
      const i = r * VC + c, a00 = height[i], a10 = height[i + 1], a01 = height[i + VC], a11 = height[i + VC + 1];
      const lo = Math.min(a00, a10, a01, a11), hi = Math.max(a00, a10, a01, a11);
      if (Math.floor(hi) === Math.floor(lo)) continue;
      const hc = (a00 + a10 + a01 + a11) / 4;
      let level = Math.round(hc);
      if (level < lo || level > hi) level = level < lo ? Math.ceil(lo) : Math.floor(hi);
      const gx = ((a10 - a00) + (a11 - a01)) / (2 * m.w), gy = ((a01 - a00) + (a11 - a10)) / (2 * m.h);
      const gl = Math.hypot(gx, gy); if (gl < 1e-6) continue;
      const nx = gx / gl, ny = gy / gl, sd = (level - hc) / gl, oy = ny * sd, ox = nx * sd;
      if (Math.abs(ox) > 0.5 * m.w || Math.abs(oy) > 0.5 * m.h) continue;
      const ang = ((Math.atan2(nx, ny) * 180 / Math.PI) % 180 + 180) % 180;
      const diag = (ang >= 28 && ang < 75) || (ang > 105 && ang <= 152);
      let ch;
      if (ang < 28 || ang > 152 || (diag && oy > 0.26 * m.h)) {
        const f = oy / m.h;
        ch = f > 0.26 ? '_' : f > 0.08 ? '.' : f > -0.18 ? '-' : "'";
      } else if (diag && oy < -0.26 * m.h) continue;
      else if (ang < 75) ch = '/';
      else if (ang <= 105) ch = '|';
      else ch = '\\';
      const x = c * m.w, y = r * m.h;
      const a = fade(x + m.w / 2, y + m.h / 2); if (a <= 0) continue;
      let col = C.text, w = 0;
      for (let k = 0; k < K; k++) {
        const hl = hills[k], v = inf[i * K + k];
        if (v < 0.02) continue;
        const resting = hl.a.state === 'resting';
        const tone = hl.a.state === 'error' ? C.bad : resting ? C.muted : hl.color;
        col = mix(col, tone, Math.min(1, v * 1.3) * (resting ? 0.4 : 1));
        w = Math.max(w, resting ? 0 : v);
      }
      const index = ((level % 5) + 5) % 5 === 0;
      g.fillStyle = rgba(col, a * (index ? 0.26 : 0.14) * (1 + 1.6 * w));
      g.fillText(ch, x, y + m.base);
    }

    g.font = font.replace(`${PX}px`, '10px');
    for (const hl of hills) {
      const tone = hl.a.state === 'error' ? C.bad : hl.a.state === 'resting' ? C.muted : hl.color;
      g.fillStyle = rgba(tone, 0.75 * fade(hl.px, hl.py));
      g.fillText(`+ ${hl.a.id} · ${LABELS[hl.a.state]}`, hl.px - 4, hl.py + 4);
    }
  }

  // Reduced motion: redraw only when something it shows changes.
  function tick() {
    const now = performance.now();
    let off = false;
    try { off = localStorage.getItem('orch.background') === 'off'; } catch { /* storage blocked: leave it on */ }
    const canvases = document.hidden ? [] : document.querySelectorAll('canvas.ascii-field');
    const t = still.matches ? 0 : (now - t0) / 1000;
    for (const cv of canvases) {
      if (off) {
        if (cv.width) { cv.width = 0; delete cv.dataset.drawn; }
        continue;
      }
      if (still.matches) {
        const key = `${cv.clientWidth}x${cv.clientHeight} ${cv.dataset.agents} ${document.documentElement.dataset.theme}`;
        if (key === cv.dataset.drawn) continue;
        cv.dataset.drawn = key;
      }
      draw(cv, t, now);
    }
    setTimeout(() => requestAnimationFrame(tick), canvases.length && !off && !still.matches ? FRAME_MS : 500);
  }
  tick();
})();
