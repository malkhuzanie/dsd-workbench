// "Try the cut": a small interactive sketch of the waveform editor on synthetic audio.
import { rng, ease } from './util.js';

const DURATION = 1380; // seconds
const GAPS = [298, 558, 838, 1098]; // centres of the silent gaps between the five "tracks"
const GAP_HALF = 1.6;

const fmt = (s) => {
  const m = Math.floor(s / 60);
  return `${m}:${(s - m * 60).toFixed(3).padStart(6, '0')}`;
};

/** Loudness envelope 0..1 at time `t`: music with slow swells, silent inside the gaps. */
function envelope(t, seedTable) {
  for (const g of GAPS) if (Math.abs(t - g) < GAP_HALF) return 0.02;
  const k = Math.floor(t / 7);
  const a = seedTable[k % seedTable.length];
  const swell = 0.55 + 0.45 * Math.sin(t / 23 + a * 6);
  return 0.18 + 0.7 * a * swell;
}

export function initCutDemo(root) {
  const rand = rng(21);
  const table = Array.from({ length: 64 }, () => 0.45 + rand() * 0.55);

  root.innerHTML = `
    <div class="demo-bar">
      <button class="pill accent" data-act="detect">Detect silences</button>
      <button class="pill" data-act="snap">Snap cuts to gaps</button>
      <button class="pill" data-act="clear">Clear</button>
      <span class="spacer"></span>
      <button class="pill" data-act="reset">Reset</button>
    </div>
    <canvas aria-label="Waveform with draggable cut markers" role="img"></canvas>
    <p class="demo-hint">Click to add a cut · drag a marker to move it · right-click a marker to remove it. Markers in loud music turn red.</p>
    <ol></ol>`;
  const canvas = root.querySelector('canvas');
  const list = root.querySelector('ol');
  const ctx = canvas.getContext('2d');

  let cuts = [293.4, 571, 1090.2];
  let anim = null;
  let drag = -1;
  let hover = -1;
  let peaks = [];
  let dpr = 1;

  const build = () => {
    dpr = Math.min(devicePixelRatio || 1, 2);
    canvas.width = canvas.clientWidth * dpr;
    canvas.height = canvas.clientHeight * dpr;
    const n = Math.max(120, Math.floor(canvas.clientWidth / 2.5));
    const r = rng(3);
    peaks = Array.from({ length: n }, (_, i) => {
      const t = ((i + 0.5) / n) * DURATION;
      const e = envelope(t, table);
      return e * (0.55 + 0.45 * r());
    });
  };

  const x2t = (x) => Math.min(DURATION, Math.max(0, (x / canvas.clientWidth) * DURATION));
  const t2x = (t) => (t / DURATION) * canvas.clientWidth;
  const loud = (t) => envelope(t, table) > 0.08;

  const draw = () => {
    const w = canvas.clientWidth, h = canvas.clientHeight;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    const ruler = 22, mid = (h - ruler) / 2;

    // Alternating track bands.
    const edges = [0, ...cuts, DURATION];
    for (let i = 0; i < edges.length - 1; i++) {
      ctx.fillStyle = i % 2 ? 'rgba(250,249,245,0.035)' : 'rgba(250,249,245,0.0)';
      ctx.fillRect(t2x(edges[i]), 0, t2x(edges[i + 1]) - t2x(edges[i]), h - ruler);
      ctx.fillStyle = 'rgba(250,249,245,0.28)';
      ctx.font = '500 12px "JetBrains Mono", monospace';
      ctx.fillText(String(i + 1), t2x(edges[i]) + 8, 18);
    }

    // Silence ranges, faintly, so the gaps read as seams.
    ctx.fillStyle = 'rgba(93,184,166,0.10)';
    for (const g of GAPS) ctx.fillRect(t2x(g - GAP_HALF), 0, Math.max(3, t2x(GAP_HALF * 2)), h - ruler);

    // Waveform.
    const n = peaks.length;
    const bw = w / n;
    for (let i = 0; i < n; i++) {
      const a = peaks[i] * (mid - 10);
      ctx.fillStyle = peaks[i] < 0.05 ? 'rgba(160,157,150,0.5)' : '#e8a55a';
      ctx.fillRect(i * bw, mid - a, Math.max(1, bw - 0.8), a * 2 || 1);
    }
    ctx.fillStyle = 'rgba(250,249,245,0.08)';
    ctx.fillRect(0, mid, w, 1);

    // Ruler.
    ctx.fillStyle = 'rgba(160,157,150,0.85)';
    ctx.font = '500 10.5px "JetBrains Mono", monospace';
    for (let m = 0; m * 60 <= DURATION; m += 3) {
      const x = t2x(m * 60);
      ctx.fillRect(x, h - ruler, 1, 5);
      ctx.fillText(`${m}:00`, x + 4, h - 6);
    }

    // Markers.
    cuts.forEach((t, i) => {
      const x = t2x(t);
      const bad = loud(t);
      const col = bad ? '#e5806b' : '#7fd0be';
      ctx.fillStyle = col;
      ctx.fillRect(x - 1, 0, i === drag || i === hover ? 3 : 2, h - ruler);
      ctx.beginPath();
      ctx.moveTo(x - 7, 0); ctx.lineTo(x + 7, 0); ctx.lineTo(x, 10); ctx.closePath();
      ctx.fill();
    });
  };

  const renderList = () => {
    const edges = [0, ...cuts, DURATION];
    list.innerHTML = edges.slice(0, -1).map((s, i) => {
      const warn = i > 0 && loud(s) ? ' ⚠' : '';
      return `<li><span>${String(i + 1).padStart(2, '0')}${warn}</span><span>${fmt(edges[i + 1] - s)}</span></li>`;
    }).join('');
  };

  const refresh = () => { draw(); renderList(); };

  const animateTo = (targets) => {
    cancelAnimationFrame(anim);
    // Pair existing cuts with the nearest targets so they glide into place.
    const from = [...cuts];
    while (from.length < targets.length) from.push(targets[from.length]);
    const start = performance.now();
    const step = (now) => {
      const k = ease(Math.min(1, (now - start) / 650));
      cuts = targets.map((t, i) => from[i] + (t - from[i]) * k);
      refresh();
      if (k < 1) anim = requestAnimationFrame(step);
    };
    anim = requestAnimationFrame(step);
  };

  const nearestGap = (t) => GAPS.reduce((b, g) => (Math.abs(g - t) < Math.abs(b - t) ? g : b), GAPS[0]);

  root.querySelector('.demo-bar').addEventListener('click', (e) => {
    const act = e.target.closest('button')?.dataset.act;
    if (act === 'detect') animateTo([...GAPS]);
    if (act === 'snap') animateTo(cuts.length ? cuts.map(nearestGap) : []);
    if (act === 'clear') { cancelAnimationFrame(anim); cuts = []; refresh(); }
    if (act === 'reset') animateTo([293.4, 571, 1090.2]);
  });

  const pick = (x) => cuts.findIndex((t) => Math.abs(t2x(t) - x) < 9);
  const local = (e) => e.clientX - canvas.getBoundingClientRect().left;

  canvas.addEventListener('pointerdown', (e) => {
    cancelAnimationFrame(anim);
    if (e.button === 2) return;
    const x = local(e);
    let i = pick(x);
    if (i < 0) {
      cuts.push(x2t(x));
      cuts.sort((a, b) => a - b);
      i = pick(x);
    }
    drag = i;
    canvas.setPointerCapture(e.pointerId);
    refresh();
  });
  canvas.addEventListener('pointermove', (e) => {
    const x = local(e);
    if (drag >= 0) {
      const lo = drag > 0 ? cuts[drag - 1] + 1 : 1;
      const hi = drag < cuts.length - 1 ? cuts[drag + 1] - 1 : DURATION - 1;
      cuts[drag] = Math.min(hi, Math.max(lo, x2t(x)));
      // Ctrl snaps to the middle of a gap, as in the app.
      if (e.ctrlKey) cuts[drag] = nearestGap(cuts[drag]);
    } else {
      hover = pick(x);
      canvas.style.cursor = hover >= 0 ? 'ew-resize' : 'crosshair';
    }
    refresh();
  });
  const end = () => { drag = -1; refresh(); };
  canvas.addEventListener('pointerup', end);
  canvas.addEventListener('pointercancel', end);
  canvas.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    const i = pick(local(e));
    if (i >= 0) { cuts.splice(i, 1); refresh(); }
  });

  build();
  refresh();
  addEventListener('resize', () => { build(); refresh(); });
}
