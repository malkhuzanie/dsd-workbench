// Animated background: columns of 1-bit "samples" drifting past, with a slow cut sweeping
// across them. Cheap 2D canvas, paused when off screen.
import { rng, runWhileVisible } from './util.js';

const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;

export function initBits(canvas) {
  const ctx = canvas.getContext('2d');
  const cell = 14;
  let cols = 0, rows = 0, grid = new Uint8Array(0), speeds = new Float32Array(0), dpr = 1;
  const rand = rng(5);

  const resize = () => {
    dpr = Math.min(devicePixelRatio || 1, 2);
    const { clientWidth: w, clientHeight: h } = canvas;
    canvas.width = w * dpr;
    canvas.height = h * dpr;
    cols = Math.ceil(w / cell) + 1;
    rows = Math.ceil(h / cell) + 1;
    grid = Uint8Array.from({ length: cols * rows }, () => (rand() > 0.5 ? 1 : 0));
    speeds = Float32Array.from({ length: cols }, () => 4 + rand() * 18);
  };
  resize();
  addEventListener('resize', resize);

  const draw = (t) => {
    const w = canvas.clientWidth, h = canvas.clientHeight;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    ctx.font = '500 11px "JetBrains Mono", monospace';
    ctx.textBaseline = 'top';
    const sweep = ((t * 0.07) % 1.4 - 0.2) * w;
    for (let c = 0; c < cols; c++) {
      const off = (t * speeds[c]) % cell;
      const shift = Math.floor((t * speeds[c]) / cell);
      for (let r = 0; r < rows; r++) {
        const bit = grid[((r + shift) % rows) * cols + c];
        const x = c * cell, y = r * cell - off;
        const d = Math.abs(x - sweep);
        const near = Math.max(0, 1 - d / 140);
        if (near > 0.02) ctx.fillStyle = `rgba(93,184,166,${0.12 + near * 0.8})`;
        else ctx.fillStyle = bit ? 'rgba(232,165,90,0.20)' : 'rgba(250,249,245,0.07)';
        ctx.fillText(bit ? '1' : '0', x, y);
      }
    }
    // The seam itself.
    const g = ctx.createLinearGradient(sweep - 2, 0, sweep + 2, 0);
    g.addColorStop(0, 'rgba(250,249,245,0)');
    g.addColorStop(0.5, 'rgba(250,249,245,0.55)');
    g.addColorStop(1, 'rgba(250,249,245,0)');
    ctx.fillStyle = g;
    ctx.fillRect(sweep - 2, 0, 4, h);
  };

  if (reduced) { draw(3); return; }
  runWhileVisible(canvas, draw);
}
