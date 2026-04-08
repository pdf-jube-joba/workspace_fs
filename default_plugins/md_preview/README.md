# md_preview

## Purpose

This plugin builds browser assets for markdown preview and directory browsing.
The source code lives in `src/`.
`build.mjs` bundles the source into an explicit output directory.

## Build

Run `npm install` once if dependencies are missing.
Run the build after changing `src/`.

```bash
node ./build.mjs --out-dir <path>
```

The build command requires `--out-dir <path>`.
All generated files are written under that directory.
The built output includes the viewer HTML, CSS, and JS from `src/viewer/`.
The built output also includes the bundled KaTeX stylesheet wrapper.

## workspace_fs Plugin Use

Mount the plugin output at any prefix.
Open the generated viewer files from that mount.

For `workspace_fs` plugins, pass `{OUTPUT_DIRECTORY}` to `--out-dir`.
Configure optional transforms and enhancers under `[plugin.md_preview]`.
Each transform or enhancer entry must specify `url` and `entrypoint`.

## Markdown Preview

Markdown is rendered in the browser with `remark` and `rehype`.
Configured source transforms run before markdown parsing.

GitHub-style alerts are converted to custom HTML blocks.
Examples include `> [!NOTE]` and `> [!TIP]`.

The default build includes a KaTeX transform.
It handles math written as `\(...\)` and `\[...\]`.

## Macros

KaTeX macros are not loaded implicitly by the viewer.

Set `[plugin.md_preview].macro_path` to copy a repository-local macros file.
The copied file is emitted as `macros.txt`.
If `macro_path` is omitted, `macros.txt` is not generated.

Callers should load `macros.txt`.
Callers should convert it with `from_text(text)`.
Callers should pass the result as `macros`.

## Link Index

The build generates `link_index.json`.
It scans repository markdown files.
The directory viewer uses this index for backlink-style link pages.

## Directory View

`directory_view.html` browses repository directories.
It uses the workspace listing API.

For a path such as `docs/`, it requests `GET /docs/`.
The listing response is the source of candidate entries.

## Directory Cards

Directory entries are shown as directory cards.
Each directory card links to another `directory_view.html` page.

The card preview uses the child listing.
If the directory contains `README.md`, the card tries to use that file as preview text.
If `README.md` cannot be read, the card keeps the listing preview.

## File Cards

Not every listed file becomes a card.
The viewer currently renders file cards only for these extensions:

- `md`
- `txt`
- `rs`

Files with other extensions stay available through the API.
They are not shown as directory view cards.
For example, `.gitmodules`, `Cargo.toml`, and `package.json` are not shown as file cards.

Markdown file cards link to `md_preview.html`.
Text and Rust file cards link to the raw repository path.

## Policy

The directory view follows `workspace_fs` policy.
It does not bypass repository access rules.

The initial listing request must be allowed.
Each candidate entry also needs readable path info.
Entries whose path info request fails are skipped.
This can happen when `GET` is denied by policy.

## Ignore

The directory view follows `workspace_fs` ignore rules.
Ignored entries are removed by the listing API.
Ignored paths are not visible to the directory view.

Direct requests to ignored paths are rejected by `workspace_fs`.

## Generated Settings

When `WORKSPACE_FS_PLUGIN_SETTINGS_JSON` contains `[[plugin.md_preview.transform]]`, `build.mjs` appends those transforms after the default KaTeX transform.
When `WORKSPACE_FS_PLUGIN_SETTINGS_JSON` contains `[plugin.md_preview]`, `build.mjs` generates `enhance_runner.js` from `[[plugin.md_preview.enhance]]`.
