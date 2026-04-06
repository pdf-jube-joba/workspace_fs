# md_preview

This directory contains the browser-side markdown preview build setup.
Source code lives in `src/`, and `build.mjs` bundles it into an explicitly specified output directory.
The build script requires `--out-dir <path>` and writes all generated files under that directory.
It generates `link_index.json` alongside the browser assets by scanning all repository markdown files.
The built output includes the viewer HTML/CSS/JS from `src/viewer/`.
It also copies the bundled KaTeX stylesheet wrapper from `katex/katex_pre.css`.
When `WORKSPACE_FS_PLUGIN_SETTINGS_JSON` contains `[[plugin.md_preview.transform]]`, `build.mjs` also generates `transform_runner.js` from those entries.
When `WORKSPACE_FS_PLUGIN_SETTINGS_JSON` contains `[plugin.md_preview]`, `build.mjs` also generates `enhance_runner.js` from `[[plugin.md_preview.enhance]]`.
Set `[plugin.md_preview].macro_path` to copy a repository-local macros file into the built output as `macros.txt`. If it is omitted, no `macros.txt` is generated.
When used through `workspace_fs`, mount the plugin output at any prefix and open the generated viewer files from that mount.
It renders markdown to HTML in the browser with `remark` / `rehype`.
It can apply configured source transforms before markdown parsing.
The bundled `katex_transform.js` in `katex/` can be loaded via `[[plugin.md_preview.transform]]` to render KaTeX-style math written as `\(...\)` / `\[...\]`.
GitHub-style alerts such as `> [!NOTE]` and `> [!TIP]` are converted to custom HTML blocks.
KaTeX macros are not loaded implicitly by the viewer.
Callers are expected to load `macros.txt`, convert it with `from_text(text)`, and pass the result as `macros`.
Run `npm install` once in this directory if dependencies are missing, then run `node ./build.mjs --out-dir <path>` after changing `src/`.
For `workspace_fs` plugins, pass `{OUTPUT_DIRECTORY}` and configure any optional transforms or enhancers under `[plugin.md_preview]`. Each entry must specify both `url` and `entrypoint`.
