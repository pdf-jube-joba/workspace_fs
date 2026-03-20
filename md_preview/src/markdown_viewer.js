import katex from "katex";
import remarkRehype from "remark-rehype";
import rehypeStringify from "rehype-stringify";
import {createMarkdownParser, prepareMarkdownSource} from "./markdown_extensions.js";
import {createRemarkRehypeOptions} from "./markdown_to_hast.js";

export async function renderMarkdownToElement({text, element, basePath = "", macros = {}}) {
  const html = await renderMarkdownToHtml({text, basePath, macros});
  element.innerHTML = html;
  element.classList.add("md-view");
  await runConfiguredEnhancers(element, {basePath});
  return element;
}

export async function renderMarkdownToHtml({text, basePath = "", macros = {}}) {
  const katexMacros = normalizeKatexMacros(macros);
  const processor = createMarkdownParser()
    .use(remarkRehype, createRemarkRehypeOptions({
      basePath,
      katexMacros,
      renderKatexNode,
      renderMathError,
    }))
    .use(rehypeStringify, {allowDangerousHtml: true});

  const file = await processor.process(prepareMarkdownSource(text));
  return String(file);
}

export function from_text(text) {
  return parseKatexMacros(text);
}

function parseKatexMacros(text) {
  const macros = {};
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#") || line.startsWith("%")) {
      continue;
    }

    const separator = line.indexOf(":");
    if (separator === -1) {
      continue;
    }

    const key = line.slice(0, separator).trim();
    const value = line.slice(separator + 1).trim();
    if (!key || !value) {
      continue;
    }
    macros[key] = value;
  }

  return macros;
}

function normalizeKatexMacros(macros) {
  if (!macros || typeof macros !== "object" || Array.isArray(macros)) {
    return {};
  }
  return macros;
}

function renderKatexNode(node, macros, displayMode) {
  try {
    return katex.renderToString(node.value, {
      displayMode,
      throwOnError: true,
      macros,
      output: "html",
    });
  } catch (error) {
    return renderMathError(node.value, String(error), displayMode);
  }
}

function renderMathError(source, message, displayMode) {
  const tagName = displayMode ? "div" : "span";
  return `<${tagName} class="md-math-error"><span class="md-math-error-label">KaTeX Error</span><code>${escapeHtml(source)}</code><span class="md-math-error-message">${escapeHtml(message)}</span></${tagName}>`;
}

function escapeHtml(value) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

let enhanceRunnerPromise = null;

async function runConfiguredEnhancers(root, context) {
  if (!enhanceRunnerPromise) {
    const enhanceRunnerUrl = new URL("./enhance_runner.js", import.meta.url);
    enhanceRunnerPromise = import(enhanceRunnerUrl.href);
  }
  const {runEnhancers} = await enhanceRunnerPromise;
  await runEnhancers(root, context);
}
