// A DSD stream drawn as a ribbon of tiny "bits" that can be cut into tracks, spread apart and
// verified. Used by the home page hero and the How it works pipeline.
import * as THREE from '../vendor/three.min.js';
import { clamp01, ease, reducedMotion, rng, runWhileVisible, span } from './util.js';

export { clamp01, ease, reducedMotion, rng, runWhileVisible, span };

export const COLORS = {
  one: new THREE.Color('#e8a55a'),
  zero: new THREE.Color('#6b4a3a'),
  ok: new THREE.Color('#5db8a6'),
  okDim: new THREE.Color('#2e6f62'),
  cut: new THREE.Color('#faf9f5'),
  header: new THREE.Color('#cc785c'),
  tag: new THREE.Color('#5db8a6'),
};

export class Ribbon {
  /**
   * @param {object} o
   * @param {number} o.cols bits drawn per row
   * @param {number} o.rows channels
   * @param {number} o.length world width
   * @param {number[]} o.cuts cut positions, 0..1 along the ribbon
   * @param {number} o.seed
   * @param {number} [o.gap] distance between tracks when spread
   */
  constructor({ cols = 1500, rows = 2, length = 14, cuts = [0.19, 0.43, 0.6, 0.82], seed = 11, gap = 0.55, rowGap = 0.34 }) {
    Object.assign(this, { cols, rows, length, cuts, gap, rowGap });
    this.count = cols * rows;
    this.group = new THREE.Group();

    const geo = new THREE.BoxGeometry(1, 1, 1);
    const mat = new THREE.MeshBasicMaterial({ transparent: true, opacity: 1 });
    this.mesh = new THREE.InstancedMesh(geo, mat, this.count);
    this.mesh.frustumCulled = false;
    this.mesh.instanceColor = new THREE.InstancedBufferAttribute(new Float32Array(this.count * 3), 3);
    this.group.add(this.mesh);

    const r = rng(seed);
    this.bit = new Uint8Array(this.count);
    this.track = new Uint8Array(this.count);
    this.phase = new Float32Array(this.count);
    for (let i = 0; i < this.count; i++) {
      const c = i % cols;
      this.bit[i] = r() > 0.5 ? 1 : 0;
      this.phase[i] = r();
      const u = c / (cols - 1);
      this.track[i] = cuts.filter((x) => u >= x).length;
    }
    this.tracks = cuts.length + 1;
    this.bounds = [0, ...cuts, 1];
    this.colorDirty = true;
    this.last = { verify: -1, tint: -1 };

    // Vertical cut markers.
    this.cutMeshes = cuts.map(() => {
      const m = new THREE.Mesh(
        new THREE.PlaneGeometry(0.035, 1),
        new THREE.MeshBasicMaterial({ color: COLORS.cut, transparent: true, opacity: 0, blending: THREE.AdditiveBlending, depthWrite: false }),
      );
      this.group.add(m);
      return m;
    });
  }

  /** x of a position `u` (0..1) on the unspread ribbon. */
  xAt(u) {
    return (u - 0.5) * this.length;
  }

  /** Offset of track `j` from its resting place when spread by `s`. */
  trackOffset(j, s) {
    return (j - (this.tracks - 1) / 2) * this.gap * s;
  }

  /** Left and right x of track `j` for a given spread. */
  trackSpan(j, s) {
    const o = this.trackOffset(j, s);
    return [this.xAt(this.bounds[j]) + o, this.xAt(this.bounds[j + 1]) + o];
  }

  /**
   * @param {object} s
   * @param {number} s.t seconds, drives the idle wave
   * @param {number} [s.cut] 0..1 cut markers
   * @param {number} [s.spread] 0..1 tracks pulled apart
   * @param {number} [s.verify] 0..1 verification sweep (left to right, per track at once)
   * @param {number} [s.amp] wave amplitude
   * @param {number} [s.opacity]
   */
  update({ t, cut = 0, spread = 0, verify = 0, amp = 1, opacity = 1 }) {
    const { cols, rows, count, length, rowGap } = this;
    const m = this.mesh.instanceMatrix.array;
    const w = (length / cols) * 0.78;
    const sp = ease(clamp01(spread));
    for (let i = 0; i < count; i++) {
      const c = i % cols;
      const row = (i / cols) | 0;
      const u = c / (cols - 1);
      const tr = this.track[i];
      const x = this.xAt(u) + this.trackOffset(tr, sp);
      const wave =
        Math.sin(u * 17 + t * 0.9 + row * 1.7) * 0.2 * amp +
        Math.sin(u * 5.3 - t * 0.5) * 0.28 * amp * (1 - 0.5 * sp);
      const lift = Math.sin((tr + 1) * 1.9) * 0.16 * sp;
      const y = (row - (rows - 1) / 2) * rowGap + wave + lift;
      const z = (tr - (this.tracks - 1) / 2) * 0.14 * sp + Math.sin(u * 9 + row) * 0.12 * amp;
      const bit = this.bit[i];
      const sy = bit ? 0.21 + 0.09 * Math.sin(t * 2 + this.phase[i] * 6.283) : 0.09;
      const o = i * 16;
      m[o] = w; m[o + 1] = 0; m[o + 2] = 0; m[o + 3] = 0;
      m[o + 4] = 0; m[o + 5] = sy; m[o + 6] = 0; m[o + 7] = 0;
      m[o + 8] = 0; m[o + 9] = 0; m[o + 10] = 0.05; m[o + 11] = 0;
      m[o + 12] = x; m[o + 13] = y; m[o + 14] = z; m[o + 15] = 1;
    }
    this.mesh.instanceMatrix.needsUpdate = true;
    this.mesh.material.opacity = opacity;

    if (verify !== this.last.verify) {
      this.last.verify = verify;
      this.recolor(verify);
    }

    this.cutMeshes.forEach((mesh, k) => {
      const u = this.cuts[k];
      const gone = 1 - clamp01(spread * 1.6);
      mesh.position.set(this.xAt(u), 0, 0.05);
      mesh.scale.y = 0.2 + cut * 2.6;
      mesh.material.opacity = cut * gone * (0.7 + 0.3 * Math.sin(t * 3 + k));
    });
  }

  recolor(verify) {
    const col = this.mesh.instanceColor.array;
    const { cols } = this;
    for (let i = 0; i < this.count; i++) {
      const u = (i % cols) / (cols - 1);
      const t0 = this.bounds[this.track[i]];
      const t1 = this.bounds[this.track[i] + 1];
      const local = (u - t0) / (t1 - t0);
      const done = local < verify;
      const c = done ? (this.bit[i] ? COLORS.ok : COLORS.okDim) : this.bit[i] ? COLORS.one : COLORS.zero;
      col[i * 3] = c.r; col[i * 3 + 1] = c.g; col[i * 3 + 2] = c.b;
    }
    this.mesh.instanceColor.needsUpdate = true;
  }

  dispose() {
    this.mesh.geometry.dispose();
    this.mesh.material.dispose();
    this.cutMeshes.forEach((m) => { m.geometry.dispose(); m.material.dispose(); });
  }
}

/** Standard renderer setup: capped pixel ratio, transparent background, resize handling. */
export function makeRenderer(canvas) {
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true, powerPreference: 'high-performance' });
  renderer.setClearColor(0x000000, 0);
  const fit = () => {
    const { clientWidth: w, clientHeight: h } = canvas;
    renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, w < 700 ? 1.5 : 2));
    renderer.setSize(w, h, false);
    return { w, h };
  };
  return { renderer, fit };
}

export { THREE };
