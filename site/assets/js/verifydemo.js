// "Try to fool it": a bit-by-bit comparison of a source and its copy on a tiny synthetic stream.
import { rng, runWhileVisible } from './util.js';

const COLS = 40;
const ROWS = 6;
const N = COLS * ROWS;

export function initVerifyDemo(root) {
  root.innerHTML = `
    <canvas aria-label="Source and copy bit grids being compared" role="img"></canvas>
    <div class="bitstat"><span>compared <b data-k="n">0</b> / ${N} bits</span><span data-k="verdict"><b>identical so far</b></span></div>
    <div class="demo-bar" style="margin:16px 0 0">
      <button class="pill accent" data-act="flip">Flip a bit in the copy</button>
      <button class="pill" data-act="restore">Restore the copy</button>
      <button class="pill" data-act="again">Compare again</button>
    </div>`;
  const canvas = root.querySelector('canvas');
  const ctx = canvas.getContext('2d');
  const nEl = root.querySelector('[data-k=n]');
  const vEl = root.querySelector('[data-k=verdict]');

  const r = rng(99);
  const source = Uint8Array.from({ length: N }, () => (r() > 0.5 ? 1 : 0));
  const copy = Uint8Array.from(source);
  let progress = 0; // bits compared (fractional)
  let found = []; // indexes of differing bits already reached
  let flash = 0;
  let dpr = 1;

  const resize = () => {
    dpr = Math.min(devicePixelRatio || 1, 2);
    canvas.width = canvas.clientWidth * dpr;
    canvas.height = canvas.clientHeight * dpr;
  };
  resize();
  addEventListener('resize', resize);

  let scanned = 0; // bits already checked for a difference
  const restart = () => { progress = 0; scanned = 0; found = []; };
  root.querySelector('.demo-bar').addEventListener('click', (e) => {
    const act = e.target.closest('button')?.dataset.act;
    if (act === 'flip') {
      let i;
      do { i = Math.floor(Math.random() * N); } while (copy[i] !== source[i]);
      copy[i] ^= 1;
      restart();
    }
    if (act === 'restore') { copy.set(source); restart(); }
    if (act === 'again') restart();
  });

  let last = 0;
  const draw = (t) => {
    const dt = Math.min(0.05, t - last);
    last = t;
    progress = Math.min(N, progress + dt * 70);
    const done = Math.floor(progress);
    for (; scanned < done; scanned++) {
      if (copy[scanned] !== source[scanned]) { found.push(scanned); flash = 1; }
    }
    flash = Math.max(0, flash - dt * 1.6);

    const w = canvas.clientWidth, h = canvas.clientHeight;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    const cw = w / COLS;
    const gh = (h - 40) / 2;
    const ch = gh / ROWS;
    ctx.font = `500 ${Math.min(cw * 0.78, 14)}px "JetBrains Mono", monospace`;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';

    const labelY = [10, 40 + gh - 4];
    ctx.textAlign = 'left';
    ctx.fillStyle = 'rgba(160,157,150,0.9)';
    ctx.font = '500 11px "JetBrains Mono", monospace';
    ctx.fillText('SOURCE', 2, labelY[0] - 2);
    ctx.fillText('COPY', 2, 30 + gh + 6);
    ctx.textAlign = 'center';
    ctx.font = `500 ${Math.min(cw * 0.78, 14)}px "JetBrains Mono", monospace`;

    for (let g = 0; g < 2; g++) {
      const data = g ? copy : source;
      const y0 = g ? 40 + gh : 18;
      for (let i = 0; i < N; i++) {
        const col = i % COLS, row = (i / COLS) | 0;
        const cx = col * cw + cw / 2, cy = y0 + row * ch + ch / 2;
        const seen = i < done;
        const bad = found.includes(i);
        if (bad) {
          ctx.fillStyle = `rgba(229,128,107,${0.25 + 0.4 * flash})`;
          ctx.fillRect(col * cw, y0 + row * ch, cw, ch);
        }
        ctx.fillStyle = bad ? '#e5806b' : seen ? '#5db8a6' : data[i] ? '#e8a55a' : 'rgba(250,249,245,0.28)';
        ctx.fillText(String(data[i]), cx, cy);
      }
    }
    // Scan head.
    const hc = done % COLS, hr = (done / COLS) | 0;
    if (done < N) {
      ctx.fillStyle = 'rgba(250,249,245,0.5)';
      ctx.fillRect(hc * cw, 18 + hr * ch, 2, ch);
      ctx.fillRect(hc * cw, 40 + gh + hr * ch, 2, ch);
    }

    nEl.textContent = String(done);
    if (found.length) {
      vEl.innerHTML = `<b class="bad">differs at bit ${found[0]}</b>`;
    } else {
      vEl.innerHTML = done >= N ? '<b>identical</b>' : '<b>identical so far</b>';
    }
  };
  runWhileVisible(canvas, draw);
}
