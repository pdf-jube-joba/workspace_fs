import {build} from "esbuild";
import {cp, mkdir, writeFile} from "node:fs/promises";
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
const transforms = normalizeModuleHooks(pluginSettings.md_preview?.transform, "transform");
const enhancers = normalizeEnhancers(pluginSettings.md_preview?.enhance);
const macrosSource = resolveMacrosSource(
  pluginSettings.md_preview?.macro_path ?? pluginSettings.md_preview?.macros_path,
  repositoryRoot,
);
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
await writeEnhanceRunner(path.join(outDir, "enhance_runner.js"), enhancers);
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

function normalizeEnhancers(rawEnhancers) {
  return normalizeModuleHooks(rawEnhancers, "enhance");
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

async function writeEnhanceRunner(outputPath, enhancers) {
  const source = `const enhancerSpecs = ${JSON.stringify(enhancers, null, 2)};

let loadedEnhancersPromise = null;

export async function runEnhancers(root, context = {}) {
  const loadedEnhancers = await loadEnhancers();
  for (const enhance of loadedEnhancers) {
    await enhance(root, context);
  }
}

async function loadEnhancers() {
  if (!loadedEnhancersPromise) {
    loadedEnhancersPromise = Promise.all(enhancerSpecs.map(spec => loadHook(spec, "enhancer")));
  }
  return loadedEnhancersPromise;
}

${sharedHookLoaderSource()}
`;
  await writeFile(outputPath, source);
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
