#!/usr/bin/env bash
# Builds the documentation pages (site/docs/) from the Markdown in docs/.
# Needs pandoc (3.x). The output is not committed; the Pages workflow runs this before publishing.
set -euo pipefail

tools="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
site="$(dirname "$tools")"
src="$(dirname "$site")/docs"
out="$site/docs"

# slug|group, in reading order. `index` is docs/README.md.
PAGES=(
  "index|Start"
  "concepts|Start"
  "user-guide|Start"
  "metadata|Guides"
  "naming|Guides"
  "splitting-without-a-cue|Guides"
  "online-lookup|Guides"
  "verification|Guides"
  "playback|Guides"
  "cli|Reference"
  "cue-support|Reference"
  "dsf-format|Reference"
  "architecture|Contributing"
  "development|Contributing"
)

source_of() { if [[ $1 == index ]]; then echo README.md; else echo "$1.md"; fi; }

# Plain text for titles and labels: no Markdown code ticks, HTML-escaped.
plain() {
  local t=${1//\`/}
  t=${t//&/&amp;}; t=${t//</&lt;}; t=${t//>/&gt;}
  printf '%s' "$t"
}

declare -A LABEL
for entry in "${PAGES[@]}"; do
  slug=${entry%%|*}
  first=$(head -n 1 "$src/$(source_of "$slug")")
  [[ $first == "# "* ]] || { echo "$slug: the first line must be a '# ' heading" >&2; exit 1; }
  LABEL[$slug]=$(plain "${first#\# }")
done
LABEL[index]="Overview"

sidebar() {
  local current=$1 group="" slug g aria
  for entry in "${PAGES[@]}"; do
    slug=${entry%%|*}; g=${entry#*|}
    if [[ $g != "$group" ]]; then printf '<h4>%s</h4>' "$g"; group=$g; fi
    aria=""; [[ $slug == "$current" ]] && aria=' aria-current="page"'
    printf '<a href="%s.html"%s>%s</a>' "$slug" "$aria" "${LABEL[$slug]}"
  done
}

pager() {
  local i=$1 prev next
  if (( i > 0 )); then
    prev=${PAGES[i-1]%%|*}
    printf '<a class="prev" href="%s.html"><small>Previous</small>%s</a>' "$prev" "${LABEL[$prev]}"
  else
    printf '<span></span>'
  fi
  if (( i < ${#PAGES[@]} - 1 )); then
    next=${PAGES[i+1]%%|*}
    printf '<a class="next" href="%s.html"><small>Next</small>%s</a>' "$next" "${LABEL[$next]}"
  fi
}

rm -rf "$out"
mkdir -p "$out"

for i in "${!PAGES[@]}"; do
  slug=${PAGES[i]%%|*}
  file=$(source_of "$slug")
  pandoc -f gfm -t html5 -s \
    --lua-filter "$tools/docs.lua" --template "$tools/docs-template.html" \
    -V "pagetitle=${LABEL[$slug]}" -V "source=$file" \
    -V "sidebar=$(sidebar "$slug")" -V "pager=$(pager "$i")" \
    "$src/$file" -o "$out/$slug.html"
done

# Fail on a link that points nowhere (the page, not the #anchor, is checked).
status=0
for page in "$out"/*.html; do
  while IFS= read -r href; do
    case $href in http*|mailto:*|\#*) continue ;; esac
    target=${href%%#*}
    [[ -z $target ]] && continue
    if [[ ! -e "$out/$target" ]]; then
      echo "broken link in $(basename "$page"): $href" >&2
      status=1
    fi
  done < <(grep -o 'href="[^"]*"' "$page" | cut -d'"' -f2)
done
exit $status
