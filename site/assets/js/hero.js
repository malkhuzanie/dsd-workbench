// Home page hero: a DSD stream is cut at the CUE points, the tracks drift apart and a
// verification sweep turns every bit teal, then everything flows back together.
import { THREE, Ribbon, makeRenderer, runWhileVisible, reducedMotion, ease, span } from './ribbon.js';

const PERIOD = 17;

function timeline(t) {
  const p = t % PERIOD;
  const cut = ease(span(p, 2, 3.4)) * (1 - ease(span(p, 13.6, 15.4)));
  const spread = ease(span(p, 4.4, 6.6)) * (1 - ease(span(p, 13.6, 15.6)));
  const verify = span(p, 6.8, 11.2) * (p < 15 ? 1 : 0);
  return { cut, spread, verify };
}

export function initHero(canvas, chips) {
  let renderer, fit;
  try {
    ({ renderer, fit } = makeRenderer(canvas));
  } catch {
    canvas.closest('.hero')?.classList.add('no-webgl');
    return;
  }
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(42, 1, 0.1, 100);
  camera.position.set(0, 0.4, 12);

  const ribbon = new Ribbon({ cols: 1300, rows: 3, length: 15, gap: 0.42, rowGap: 0.46 });
  const stage = new THREE.Group();
  stage.add(ribbon.group);
  scene.add(stage);

  // Slowly drifting dust.
  const n = 650;
  const pos = new Float32Array(n * 3);
  for (let i = 0; i < n; i++) {
    pos[i * 3] = (Math.random() - 0.5) * 40;
    pos[i * 3 + 1] = (Math.random() - 0.5) * 18;
    pos[i * 3 + 2] = -Math.random() * 18 - 2;
  }
  const dustGeo = new THREE.BufferGeometry();
  dustGeo.setAttribute('position', new THREE.BufferAttribute(pos, 3));
  const dust = new THREE.Points(
    dustGeo,
    new THREE.PointsMaterial({ color: 0xe8a55a, size: 0.05, transparent: true, opacity: 0.45, depthWrite: false }),
  );
  scene.add(dust);

  const pointer = { x: 0, y: 0, sx: 0, sy: 0 };
  window.addEventListener('pointermove', (e) => {
    pointer.x = (e.clientX / innerWidth - 0.5) * 2;
    pointer.y = (e.clientY / innerHeight - 0.5) * 2;
  }, { passive: true });

  const layout = () => {
    const { w, h } = fit();
    camera.aspect = w / h;
    camera.updateProjectionMatrix();
    const narrow = w < 820;
    // Wide screens: the ribbon fills the right half; narrow screens: it sits above the chips.
    // Fill the right half of what the camera sees (camera z 12, fov 42), whatever the aspect ratio.
    const half = 12 * Math.tan((42 / 2) * (Math.PI / 180)) * (w / h);
    const left = half * 0.12;
    const right = half * 0.95;
    stage.scale.setScalar(narrow ? Math.min(0.5, w / 900) : Math.min(1, (right - left) / 19));
    stage.position.set(narrow ? 0 : (left + right) / 2, narrow ? -3.4 : 0.35, 0);
  };
  layout();
  addEventListener('resize', layout);

  const chipEls = [...chips.children];
  const setChips = (spread, verify) => {
    chipEls.forEach((el, j) => {
      el.classList.toggle('on', spread > 0.55);
      el.classList.toggle('ok', verify > (j + 1) / chipEls.length - 0.001);
    });
  };

  const draw = (t) => {
    const s = timeline(t);
    ribbon.update({ t, ...s, amp: 1.5 });
    setChips(s.spread, s.verify);

    pointer.sx += (pointer.x - pointer.sx) * 0.04;
    pointer.sy += (pointer.y - pointer.sy) * 0.04;
    stage.rotation.y = -0.3 + pointer.sx * 0.12;
    stage.rotation.x = 0.06 + pointer.sy * 0.06;
    dust.rotation.z = t * 0.01;
    dust.position.x = -pointer.sx * 0.6;
    renderer.render(scene, camera);
  };

  // `?t=8` freezes the scene at that second (used for screenshots).
  const frozen = new URLSearchParams(location.search).get('t');
  if (frozen !== null) {
    runWhileVisible(canvas, () => draw(Number(frozen)));
    return;
  }
  if (reducedMotion()) {
    ribbon.update({ t: 3, cut: 0, spread: 1, verify: 1, amp: 0.6 });
    setChips(1, 1);
    renderer.render(scene, camera);
    return;
  }
  runWhileVisible(canvas, draw);
}
