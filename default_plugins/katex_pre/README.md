# katex_pre

This directory contains shared KaTeX preprocessing helpers for browser-side plugins.

`src/katex_pre.js` exports:

- `fromText(text)` for parsing `macros.txt` style macro definitions
- `renderMathMarkdown(text, {macros, renderToString})` for replacing `\(...\)` / `\[...\]` blocks with placeholder HTML
- `injectRenderedMath(html, replacements)` for restoring rendered math into the final HTML

`src/katex_pre.css` contains the KaTeX stylesheet import and markdown math error styling shared by viewers.
