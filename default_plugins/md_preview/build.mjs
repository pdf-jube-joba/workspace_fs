import {build} from "esbuild";
import {cp, mkdir, readFile, writeFile} from "node:fs/promises";
import {existsSync} from "node:fs";
import path from "node:path";
import {fileURLToPath} from "node:url";
import {generateLinkIndex} from "./build_index.mjs";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const srcDir = path.join(__dirname, "src");
const katexDir = path.join(__dirname, "katex");
const viewerSrcDir = path.join(srcDir, "viewer");
const args = parseArgs(process.argv.slice(2));
const outDir = args.outDir;
const repositoryRoot = args.repositoryRoot;
const katexDistDir = path.join(__dirname, "node_modules", "katex", "dist");
const katexOutDir = path.join(outDir, "vendor", "katex");
const pluginSettings = parsePluginSettings(process.env.WORKSPACE_FS_PLUGIN_SETTINGS_JSON);
const transforms = [
  {
    name: "katex",
    url: "./katex_transform.js",
    entrypoint: "default",
    options: {},
  },
  ...normalizeModuleHooks(pluginSettings.md_preview?.transform, "transform"),
];
const macrosSource = resolveMacrosSource(
  pluginSettings.md_preview?.macro_path,
  repositoryRoot,
);
const viewerSettings = normalizeViewerSettings(pluginSettings.md_preview);
const macrosOutPath = path.join(outDir, "macros.txt");
const katexTransformSrcPath = path.join(katexDir, "katex_transform.js");

await mkdir(outDir, {recursive: true});
await mkdir(katexOutDir, {recursive: true});

await copyMacrosFile(macrosSource, macrosOutPath);
await cp(path.join(katexDistDir, "katex.min.css"), path.join(katexOutDir, "katex.min.css"));
await cp(path.join(katexDistDir, "fonts"), path.join(katexOutDir, "fonts"), {recursive: true});
await cp(viewerSrcDir, outDir, {recursive: true});
await cp(path.join(katexDir, "katex_pre.css"), path.join(outDir, "katex_pre.css"));
await writeTransformRunner(path.join(outDir, "transform_runner.js"), transforms);
await injectHeadAssets(path.join(outDir, "md_preview.html"), viewerSettings.md_viewer);
await injectHeadAssets(path.join(outDir, "md_editor.html"), viewerSettings.md_editor);
await injectHeadAssets(path.join(outDir, "directory_view.html"), viewerSettings.directory_view);
await generateLinkIndex({outDir, repositoryRoot});

await build({
  entryPoints: [path.join(srcDir, "markdown_viewer.js")],
  bundle: true,
  format: "esm",
  platform: "browser",
  target: "es2022",
  outfile: path.join(outDir, "markdown_viewer.js"),
  sourcemap: false,
  logLevel: "info",
  nodePaths: [path.join(__dirname, "node_modules")],
});

await build({
  entryPoints: [katexTransformSrcPath],
  bundle: true,
  format: "esm",
  platform: "browser",
  target: "es2022",
  outfile: path.join(outDir, "katex_transform.js"),
  sourcemap: false,
  logLevel: "info",
  nodePaths: [path.join(__dirname, "node_modules")],
});

function parseArgs(argv) {
  let outDir = null;
  let repositoryRoot = process.cwd();

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--out-dir") {
      const value = argv[index + 1];
      if (!value) {
        throw new Error("missing value for --out-dir");
      }
      outDir = path.resolve(value);
      index += 1;
      continue;
    }
    if (arg === "--repository-root") {
      const value = argv[index + 1];
      if (!value) {
        throw new Error("missing value for --repository-root");
      }
      repositoryRoot = path.resolve(value);
      index += 1;
      continue;
    }
    throw new Error(`unknown argument: ${arg}`);
  }

  if (!outDir) {
    throw new Error("missing required argument: --out-dir <path>");
  }

  return {outDir, repositoryRoot};
}

function resolveMacrosSource(macroPath, repositoryRoot) {
  if (typeof macroPath === "string" && macroPath.trim() !== "") {
    return path.resolve(repositoryRoot, macroPath);
  }
  return null;
}

async function copyMacrosFile(macrosSource, macrosOutPath) {
  if (!macrosSource) {
    return;
  }

  if (!existsSync(macrosSource)) {
    throw new Error(`macro_path does not exist: ${macrosSource}`);
  }

  if (path.resolve(macrosSource) === path.resolve(macrosOutPath)) {
    return;
  }

  await cp(macrosSource, macrosOutPath);
}

function parsePluginSettings(text) {
  if (!text) {
    return {};
  }

  const value = JSON.parse(text);
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("WORKSPACE_FS_PLUGIN_SETTINGS_JSON must be a JSON object");
  }
  return value;
}

function normalizeModuleHooks(rawHooks, kind) {
  const values = [];
  if (Array.isArray(rawHooks)) {
    for (const rawHook of rawHooks) {
      if (!rawHook || typeof rawHook !== "object" || Array.isArray(rawHook)) {
        throw new Error(`md_preview.${kind} entries must be objects`);
      }
      const {name, url, entrypoint, ...options} = rawHook;
      if (typeof name !== "string" || name.trim() === "") {
        throw new Error(`md_preview ${kind} name must be a non-empty string`);
      }
      if (typeof url !== "string" || url.trim() === "") {
        throw new Error(`md_preview ${kind} ${name} is missing url`);
      }
      if (typeof entrypoint !== "string" || entrypoint.trim() === "") {
        throw new Error(`md_preview ${kind} ${name} is missing entrypoint`);
      }
      values.push({
        name,
        url,
        entrypoint,
        options,
      });
    }
  }
  return values;
}

function normalizeViewerSettings(mdPreviewSettings = {}) {
  return {
    md_viewer: normalizeHeadAssets(mdPreviewSettings.md_viewer ?? mdPreviewSettings["md-viewer"], "md_viewer"),
    md_editor: normalizeHeadAssets(mdPreviewSettings.md_editor ?? mdPreviewSettings["md-editor"], "md_editor"),
    directory_view: normalizeHeadAssets(
      mdPreviewSettings.directory_view ?? mdPreviewSettings["directory-view"],
      "directory_view",
    ),
  };
}

function normalizeHeadAssets(rawSettings, sectionName) {
  if (rawSettings == null) {
    return {
      additional_js: [],
      additional_module_js: [],
      additional_css: [],
    };
  }
  if (!rawSettings || typeof rawSettings !== "object" || Array.isArray(rawSettings)) {
    throw new Error(`md_preview.${sectionName} must be a table`);
  }
  return {
    additional_js: normalizePathList(
      rawSettings.additional_js ?? rawSettings["additional-js"],
      `${sectionName}.additional_js`,
    ),
    additional_module_js: normalizePathList(
      rawSettings.additional_module_js ?? rawSettings["additional-module-js"],
      `${sectionName}.additional_module_js`,
    ),
    additional_css: normalizePathList(
      rawSettings.additional_css ?? rawSettings["additional-css"],
      `${sectionName}.additional_css`,
    ),
  };
}

function normalizePathList(rawValue, fieldName) {
  if (rawValue == null) {
    return [];
  }
  if (!Array.isArray(rawValue)) {
    throw new Error(`md_preview.${fieldName} must be an array`);
  }
  return rawValue.map((value, index) => normalizeRepositoryAssetPath(value, `${fieldName}[${index}]`));
}

function normalizeRepositoryAssetPath(value, fieldName) {
  if (typeof value !== "string" || value.trim() === "") {
    throw new Error(`md_preview.${fieldName} must be a non-empty string`);
  }
  const trimmed = value.trim();
  if (trimmed.startsWith("/")) {
    throw new Error(`md_preview.${fieldName} must be a repository-relative path`);
  }
  const normalized = path.posix.normalize(trimmed);
  if (
    normalized === "." ||
    normalized === ".." ||
    normalized.startsWith("../") ||
    normalized.includes("/../")
  ) {
    throw new Error(`md_preview.${fieldName} must stay within the repository root`);
  }
  return `/${normalized}`;
}

async function writeTransformRunner(outputPath, transforms) {
  const source = `const transformSpecs = ${JSON.stringify(transforms, null, 2)};

let loadedTransformsPromise = null;

export async function runTransforms(text, context = {}) {
  const loadedTransforms = await loadTransforms();
  let value = String(text ?? "");
  for (const transform of loadedTransforms) {
    const next = await transform(value, context);
    value = String(next ?? "");
  }
  return value;
}

async function loadTransforms() {
  if (!loadedTransformsPromise) {
    loadedTransformsPromise = Promise.all(transformSpecs.map(spec => loadHook(spec, "transform")));
  }
  return loadedTransformsPromise;
}

${sharedHookLoaderSource()}
`;
  await writeFile(outputPath, source);
}

async function injectHeadAssets(htmlPath, assets) {
  if (
    assets.additional_js.length === 0 &&
    assets.additional_module_js.length === 0 &&
    assets.additional_css.length === 0
  ) {
    return;
  }

  const html = await readFile(htmlPath, "utf8");
  const headClose = html.indexOf("</head>");
  if (headClose === -1) {
    throw new Error(`viewer HTML is missing </head>: ${htmlPath}`);
  }

  const additions = [];
  for (const href of assets.additional_css) {
    additions.push(`  <link rel="stylesheet" href="${escapeHtmlAttribute(href)}">`);
  }
  for (const src of assets.additional_js) {
    additions.push(`  <script src="${escapeHtmlAttribute(src)}"></script>`);
  }
  for (const src of assets.additional_module_js) {
    additions.push(`  <script type="module" src="${escapeHtmlAttribute(src)}"></script>`);
  }

  const patched = `${html.slice(0, headClose)}${additions.join("\n")}\n${html.slice(headClose)}`;
  await writeFile(htmlPath, patched);
}

function sharedHookLoaderSource() {
  return `async function loadHook(spec, kind) {
  const mod = await import(spec.url);
  const createEnhancer = spec.entrypoint === "default"
    ? mod.default
    : mod[spec.entrypoint];
  if (typeof createEnhancer !== "function") {
    throw new Error(\`\${kind} \${spec.name} does not export \${spec.entrypoint}\`);
  }
  const hook = createEnhancer({
    ...(spec.options || {}),
    bundleBaseUrl: baseUrlFromModuleUrl(spec.url),
  });
  if (typeof hook !== "function") {
    throw new Error(\`\${kind} \${spec.name} did not return a function\`);
  }
  return hook;
}

function baseUrlFromModuleUrl(value) {
  const url = new URL(value, window.location.href);
  const pathname = url.pathname;
  const slash = pathname.lastIndexOf("/");
  url.pathname = slash === -1 ? "/" : pathname.slice(0, slash + 1);
  url.search = "";
  url.hash = "";
  return url.href;
}`;
}

function escapeHtmlAttribute(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("\"", "&quot;");
}
