// Small helpers shared by the scenes (no three.js here, so 2D pages stay light).

export function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export const ease = (x) => x * x * (3 - 2 * x);
export const clamp01 = (x) => Math.min(1, Math.max(0, x));
/** Maps `x` from the range [a, b] to 0..1, clamped. */
export const span = (x, a, b) => clamp01((x - a) / (b - a));

export const reducedMotion = () => matchMedia('(prefers-reduced-motion: reduce)').matches;

/** Calls `draw(timeSeconds)` every frame while `el` is on screen and the tab is visible. */
export function runWhileVisible(el, draw) {
  let on = false;
  let inView = false;
  let raf = 0;
  const start = performance.now();
  const frame = (now) => {
    if (!on) return;
    draw((now - start) / 1000);
    raf = requestAnimationFrame(frame);
  };
  const set = (v) => {
    if (v === on) return;
    on = v;
    if (on) raf = requestAnimationFrame(frame);
    else cancelAnimationFrame(raf);
  };
  new IntersectionObserver(([e]) => { inView = e.isIntersecting; set(inView && !document.hidden); }).observe(el);
  document.addEventListener('visibilitychange', () => set(inView && !document.hidden));
}
