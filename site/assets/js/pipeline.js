// How it works: a scroll-driven 3D walk through read → cut → write → verify.
import { THREE, Ribbon, COLORS, makeRenderer, runWhileVisible, ease, span, clamp01 } from './ribbon.js';

function label(text, { size = 40, color = '#faf9f5', bg = 'rgba(24,23,21,0.72)', width = 1.9 } = {}) {
  const c = document.createElement('canvas');
  const ctx = c.getContext('2d');
  ctx.font = `500 ${size}px "JetBrains Mono", monospace`;
  const w = Math.ceil(ctx.measureText(text).width) + 44;
  const h = size + 30;
  c.width = w; c.height = h;
  ctx.font = `500 ${size}px "JetBrains Mono", monospace`;
  ctx.fillStyle = bg;
  ctx.beginPath(); ctx.roundRect(0, 0, w, h, 14); ctx.fill();
  ctx.strokeStyle = 'rgba(250,249,245,0.18)'; ctx.lineWidth = 2; ctx.stroke();
  ctx.fillStyle = color; ctx.textBaseline = 'middle';
  ctx.fillText(text, 22, h / 2 + 2);
  const tex = new THREE.CanvasTexture(c);
  const sprite = new THREE.Sprite(new THREE.SpriteMaterial({ map: tex, transparent: true, opacity: 0, depthWrite: false }));
  sprite.scale.set((width * w) / 300, (width * h) / 300, 1);
  return sprite;
}

export function initPipeline(root) {
  const canvas = root.querySelector('canvas');
  const chapters = [...root.querySelectorAll('.chapter')];
  const dots = [...document.querySelectorAll('.progress a')];
  const dotsBox = document.querySelector('.progress');

  let renderer, fit;
  try {
    ({ renderer, fit } = makeRenderer(canvas));
  } catch {
    root.classList.add('no-webgl');
    return;
  }

  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(40, 1, 0.1, 100);
  const stage = new THREE.Group();
  scene.add(stage);

  const LEN = 11.5;
  const out = new Ribbon({ cols: 1000, rows: 2, length: LEN, gap: 0.72, rowGap: 0.4, seed: 4 });
  const ghost = new Ribbon({ cols: 1000, rows: 2, length: LEN, gap: 0, rowGap: 0.4, seed: 4 });
  ghost.group.position.y = 2.3;
  stage.add(out.group, ghost.group);

  // Header and tag blocks: one original pair, then one pair per written track.
  const block = (color, w, h) => {
    const m = new THREE.Mesh(
      new THREE.BoxGeometry(w, h, 0.3),
      new THREE.MeshBasicMaterial({ color, transparent: true, opacity: 0 }),
    );
    stage.add(m);
    return m;
  };
  const origHead = block(COLORS.header, 0.34, 1.35);
  const origTag = block(COLORS.tag, 0.34, 1.35);
  const heads = out.bounds.slice(0, -1).map(() => block(COLORS.header, 0.2, 1.1));
  const tags = out.bounds.slice(0, -1).map(() => block(COLORS.tag, 0.2, 1.1));

  // The folder the files land in.
  const plate = new THREE.Mesh(
    new THREE.PlaneGeometry(1, 1),
    new THREE.MeshBasicMaterial({ color: 0xfaf9f5, transparent: true, opacity: 0, depthWrite: false }),
  );
  plate.rotation.x = -Math.PI / 2;
  stage.add(plate);

  const labels = {
    head: label('DSF header'),
    tag: label('ID3 tag'),
    data: label('1-bit audio, untouched', { color: '#e8a55a' }),
    cut: label('cut at sample 8,451,200', { color: '#7fd0be' }),
    folder: label('album folder · 5 new files', { color: '#faf9f5' }),
    ok: label('0 differences', { color: '#7fd0be', size: 46, width: 2.4 }),
    src: label('source (never modified)', { color: '#a09d96' }),
  };
  Object.values(labels).forEach((s) => stage.add(s));

  let scale = 0.5;
  const layout = () => {
    const { w, h } = fit();
    camera.aspect = w / h;
    camera.updateProjectionMatrix();
    // The ribbon takes the right ~83% of the half-width the camera sees, so it grows with the screen.
    const half = 14 * Math.tan((40 / 2) * (Math.PI / 180)) * (w / h);
    const left = half * 0.14;
    const right = half * 0.96;
    scale = w < 820 ? Math.min(0.42, w / 1000) : Math.min(1, (right - left) / 16.5);
    stage.scale.setScalar(scale);
    stage.position.set(w < 820 ? 0 : (left + right) / 2, w < 820 ? -2.6 : -0.1, 0);
  };
  layout();
  addEventListener('resize', layout);

  // Scroll position, in chapters: 0 is chapter one centred, 1 chapter two, and so on.
  const rootTop = () => root.getBoundingClientRect().top;
  let c = 0;
  let cs = 0;
  const readScroll = () => { c = Math.max(0, -rootTop() / innerHeight); };
  readScroll();
  addEventListener('scroll', readScroll, { passive: true });

  const place = (obj, x, y, z = 0) => obj.position.set(x, y, z);
  const fade = (m, v) => { m.material.opacity = clamp01(v); };

  const draw = (t) => {
    cs += (c - cs) * 0.12;
    const k = cs;

    const cut = ease(span(k, 0.55, 1.0));
    const spread = ease(span(k, 1.5, 2.0));
    const written = ease(span(k, 2.0, 2.5));
    const ghostIn = ease(span(k, 2.55, 2.95));
    const verify = span(k, 2.8, 3.55);

    out.update({ t, cut, spread, verify, amp: 0.9 });
    ghost.update({ t, cut: 0, spread: 0, verify, amp: 0.9, opacity: 0.55 * ghostIn });

    // Original header / tag live at the ends of the unspread ribbon.
    const origIn = ease(span(k, 0.05, 0.45)) * (1 - spread);
    place(origHead, out.xAt(0) - 0.3, 0);
    place(origTag, out.xAt(1) + 0.3, 0);
    fade(origHead, origIn); fade(origTag, origIn);

    // Per-track header / tag blocks follow their segment.
    heads.forEach((m, j) => {
      const [l, r] = out.trackSpan(j, spread);
      place(m, l - 0.16, 0);
      place(tags[j], r + 0.16, 0);
      const a = written;
      fade(m, a); fade(tags[j], a);
      m.scale.y = tags[j].scale.y = 0.3 + 0.7 * a;
    });

    // The folder under the written files.
    const [fl] = out.trackSpan(0, spread);
    const [, fr] = out.trackSpan(out.tracks - 1, spread);
    plate.scale.set(fr - fl + 1.2, 2.4, 1);
    place(plate, (fl + fr) / 2, -1.1, 0);
    fade(plate, written * 0.09 * (1 - ghostIn * 0.4));

    // Labels.
    const L = labels;
    place(L.head, out.xAt(0) + 0.1, 1.25); L.head.material.opacity = origIn;
    place(L.tag, out.xAt(1) - 0.1, 1.25); L.tag.material.opacity = origIn;
    place(L.data, 0, -1.3); L.data.material.opacity = ease(span(k, 0.2, 0.5)) * (1 - ease(span(k, 0.7, 0.95)));
    place(L.cut, out.xAt(out.cuts[1]), 1.5); L.cut.material.opacity = ease(span(k, 0.95, 1.2)) * (1 - ease(span(k, 1.6, 1.9)));
    place(L.folder, (fl + fr) / 2, -2.3); L.folder.material.opacity = ease(span(k, 2.35, 2.6)) * (1 - ease(span(k, 2.85, 3.1)));
    place(L.src, 0, 3.55); L.src.material.opacity = ghostIn * 0.9;
    place(L.ok, (fl + fr) / 2, -2.3); L.ok.material.opacity = ease(span(k, 3.45, 3.7));

    // Camera: lean in for the cut, pull back to see source and copy together.
    const lean = ease(span(k, 0.75, 1.1)) * (1 - ease(span(k, 1.5, 1.9)));
    const out3 = ease(span(k, 2.5, 3.0));
    camera.position.set(
      -0.9 * lean * 0 + Math.sin(t * 0.15) * 0.25,
      0.4 + 0.7 * out3 - 0.3 * lean,
      14 - 2.6 * lean + 3.2 * out3,
    );
    camera.lookAt(0 + (2.2 * lean) * 0, 0.1 + 0.9 * out3, 0);
    stage.rotation.y = -0.22 + 0.1 * Math.sin(t * 0.2);
    stage.rotation.x = 0.05 + 0.1 * out3;

    renderer.render(scene, camera);

    // Navigation dots.
    const idx = Math.min(chapters.length - 1, Math.round(k));
    dots.forEach((d, i) => d.classList.toggle('on', i === idx));
    const r = rootTop();
    dotsBox?.classList.toggle('show', r < innerHeight * 0.5 && r + root.offsetHeight > innerHeight * 0.5);
  };

  const frozen = new URLSearchParams(location.search).get('c');
  if (frozen !== null) {
    c = Number(frozen);
    cs = c;
    // Screenshot mode: pin the stage to the viewport instead of scrolling to it.
    document.documentElement.classList.add('frozen');
    runWhileVisible(canvas, (t) => { cs = c; draw(t); });
    return;
  }
  runWhileVisible(canvas, draw);
}
