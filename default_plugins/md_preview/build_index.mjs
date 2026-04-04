import {mkdir, readFile, readdir, writeFile} from "node:fs/promises";
import path from "node:path";
import {fileURLToPath} from "node:url";
import {extractWikiLinkTermsFromMarkdown} from "./src/markdown_extensions.js";

export async function generateLinkIndex({outDir, repositoryRoot}) {
  const terms = {};
  const markdownPaths = await collectMarkdownFiles(repositoryRoot);

  for (const absolutePath of markdownPaths) {
    const text = await readFile(absolutePath, "utf8");
    const relativePath = toRelativeRepositoryPath(repositoryRoot, absolutePath);
    for (const term of extractWikiLinkTermsFromMarkdown(text)) {
      if (!terms[term]) {
        terms[term] = {pages: []};
      }
      terms[term].pages.push({path: relativePath});
    }
  }

  for (const entry of Object.values(terms)) {
    entry.pages.sort((left, right) => left.path.localeCompare(right.path));
  }

  await mkdir(outDir, {recursive: true});
  await writeFile(
    path.join(outDir, "link_index.json"),
    JSON.stringify({terms}, null, 2),
    "utf8",
  );
}

if (isDirectExecution()) {
  const args = parseArgs(process.argv.slice(2));
  await generateLinkIndex({
    outDir: args.outDir,
    repositoryRoot: args.repositoryRoot,
  });
}

async function collectMarkdownFiles(root) {
  const files = [];
  await walkDirectory(root, files);
  files.sort();
  return files;
}

async function walkDirectory(directory, files) {
  const entries = await readdir(directory, {withFileTypes: true});
  for (const entry of entries) {
    if (shouldSkipEntry(entry.name)) {
      continue;
    }

    const nextPath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      await walkDirectory(nextPath, files);
      continue;
    }

    if (entry.isFile() && entry.name.toLowerCase().endsWith(".md")) {
      files.push(nextPath);
    }
  }
}

function shouldSkipEntry(name) {
  return name === ".repo" || name === ".git" || name === "node_modules" || name === "target";
}

function toRelativeRepositoryPath(repositoryRoot, absolutePath) {
  return path.relative(repositoryRoot, absolutePath).split(path.sep).join("/");
}

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

function isDirectExecution() {
  return process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
}
