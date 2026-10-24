-- Pandoc filter used by build-docs.sh: makes the Markdown in docs/ link correctly on the site.

local GITHUB = "https://github.com/malkhuzanie/dsd-workbench/blob/main/"

-- Links between documents point at the generated pages; links out of docs/ point at GitHub.
function Link(el)
  local target = el.target
  if target:match("^%a[%w+.-]*:") or target:match("^#") then
    return nil
  end
  local path, anchor = target:match("^([^#]*)(#?.*)$")
  if path == "FEATURES.md" then
    el.target = "../features.html" .. anchor
  elseif path == "README.md" then
    el.target = "index.html" .. anchor
  elseif path:match("^%.%./") then
    el.target = GITHUB .. (path:gsub("^%.%./", "")) .. anchor
  elseif path:match("%.md$") then
    el.target = (path:gsub("%.md$", ".html")) .. anchor
  end
  return el
end

-- Code is styled by the site's CSS, so pandoc's own highlighter has nothing to do.
function CodeBlock(el)
  el.classes = {}
  return el
end

-- Wide tables scroll inside their own box instead of widening the page.
function Table(el)
  return pandoc.Div({ el }, pandoc.Attr("", { "table-wrap" }))
end
