// Shared behaviour: navigation, reveal-on-scroll, hover effects and lazy-loaded scenes.

const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;

// Navigation: solid once the page scrolls, mobile menu.
const nav = $('#nav');
const onScroll = () => nav.classList.toggle('solid', scrollY > 24 || nav.classList.contains('open'));
onScroll();
addEventListener('scroll', onScroll, { passive: true });
const toggle = $('.nav-toggle');
toggle?.addEventListener('click', () => {
  const open = nav.classList.toggle('open');
  toggle.setAttribute('aria-expanded', String(open));
  onScroll();
});
$$('.nav nav a').forEach((a) => a.addEventListener('click', () => nav.classList.remove('open')));

// Reveal on scroll.
const reveals = $$('.reveal');
if ('IntersectionObserver' in window && !reduced) {
  const io = new IntersectionObserver((entries) => {
    for (const e of entries) if (e.isIntersecting) { e.target.classList.add('in'); io.unobserve(e.target); }
  }, { threshold: 0.12, rootMargin: '0px 0px -6% 0px' });
  reveals.forEach((el) => io.observe(el));
} else {
  reveals.forEach((el) => el.classList.add('in'));
}

// Card spotlight follows the pointer.
$$('.card').forEach((card) => {
  card.addEventListener('pointermove', (e) => {
    const r = card.getBoundingClientRect();
    card.style.setProperty('--mx', `${e.clientX - r.left}px`);
    card.style.setProperty('--my', `${e.clientY - r.top}px`);
  });
});

// Screenshot frames tilt a little towards the pointer.
if (!reduced && matchMedia('(hover: hover)').matches) {
  $$('[data-tilt]').forEach((el) => {
    el.addEventListener('pointermove', (e) => {
      const r = el.getBoundingClientRect();
      const x = (e.clientX - r.left) / r.width - 0.5;
      const y = (e.clientY - r.top) / r.height - 0.5;
      el.style.transform = `rotateY(${x * 5}deg) rotateX(${-y * 4}deg) translateZ(0)`;
      el.style.boxShadow = `${-x * 28}px ${24 - y * 18}px 48px -16px rgb(20 20 19 / 35%)`;
    });
    el.addEventListener('pointerleave', () => { el.style.transform = ''; el.style.boxShadow = ''; });
  });
}

// Feature filters (features page).
const filters = $('.filters');
if (filters) {
  const cards = $$('[data-cat]');
  filters.addEventListener('click', (e) => {
    const b = e.target.closest('button');
    if (!b) return;
    $$('button', filters).forEach((x) => x.setAttribute('aria-pressed', String(x === b)));
    const cat = b.dataset.filter;
    cards.forEach((c) => { c.hidden = cat !== 'all' && !c.dataset.cat.split(' ').includes(cat); });
  });
}

// Scenes are loaded only on the pages and screens that use them.
const load = (selector, module, init) => {
  const el = $(selector);
  if (!el) return;
  import(module).then((m) => init(m, el)).catch((err) => console.warn(`${module}:`, err));
};
load('#hero-scene', './hero.js', (m, el) => m.initHero(el, $('#hero-chips')));
load('#pipeline', './pipeline.js', (m, el) => m.initPipeline(el));
load('#cut-demo', './cutdemo.js', (m, el) => m.initCutDemo(el));
load('#verify-demo', './verifydemo.js', (m, el) => m.initVerifyDemo(el));
if ($('[data-bits]')) import('./bits.js').then((m) => $$('[data-bits]').forEach((c) => m.initBits(c)));
