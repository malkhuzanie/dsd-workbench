# Website

The project site (home, How it works, Features, Docs). Plain HTML, CSS and JavaScript with no
third-party requests: fonts are self-hosted and three.js is vendored. The only build step is the
documentation pages, which are generated from `docs/`.

## Preview

```sh
site/tools/build-docs.sh        # needs pandoc; writes site/docs/ (not committed)
cd site
python3 -m http.server 8000     # then open http://localhost:8000
```

## Documentation pages

`tools/build-docs.sh` turns the Markdown files in `docs/` into `site/docs/*.html` with pandoc, using
`tools/docs-template.html` (the site's header, footer and a sidebar) and `tools/docs.lua` (rewrites
links between documents, sends links out of `docs/` to GitHub, wraps tables). The page list and its
order are at the top of the script; a new document must be added there to appear. The script fails
on a link that points nowhere, and the Pages workflow runs it before publishing, so the Markdown
stays the single source and the repository and the site never disagree.

## Publish

`.github/workflows/pages.yml` deploys `site/` to GitHub Pages on every push to `main` that touches
`site/` or `docs/`.
Enable it once under Settings → Pages → Source: *GitHub Actions*. All links are relative, so the site
works under `https://<user>.github.io/<repo>/`.

## Layout

| Path | What |
|------|------|
| `index.html`, `how-it-works.html`, `features.html` | The pages |
| `assets/css/style.css` | All styles; sized in `rem`, and the root size grows with the viewport |
| `assets/css/docs.css` | Extra styles for the documentation pages |
| `tools/build-docs.sh`, `docs-template.html`, `docs.lua` | Build the documentation pages from `docs/` |
| `docs/` | Generated; git-ignored |
| `assets/js/main.js` | Navigation, reveal on scroll, hover effects; loads the scenes on demand |
| `assets/js/ribbon.js` | The 3D "DSD stream" (instanced bits that cut, spread and verify) |
| `assets/js/hero.js` | Home page hero scene (looping timeline) |
| `assets/js/pipeline.js` | How it works: scroll-driven 3D walk through read → cut → write → verify |
| `assets/js/cutdemo.js`, `verifydemo.js`, `bits.js` | 2D canvas demos and animated background |
| `assets/vendor/three.min.js` | Tree-shaken three.js bundle; rebuild with `tools/build-vendor.sh` |
| `assets/img/` | Screenshots (copies of `docs/images/`) and the logo |

## Notes

- Every animation pauses when off screen or in a background tab, and honours `prefers-reduced-motion`.
- Without WebGL the scenes are skipped and the page falls back to its dark background.
- Add `?t=8` to the home page (freeze the hero at second 8) or `?c=2.3` to How it works (freeze the
  pipeline at chapter 2.3) to inspect a moment.
- Keep claims in sync with `docs/FEATURES.md`.
