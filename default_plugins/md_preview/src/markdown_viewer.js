import remarkRehype from "remark-rehype";
import rehypeStringify from "rehype-stringify";
import {fromText} from "../katex/katex_pre.js";
import {createMarkdownParser, prepareMarkdownSource} from "./markdown_extensions.js";
import {createRemarkRehypeOptions} from "./markdown_to_hast.js";

const SOURCE_MAPPED_TAGS = new Set([
  "p",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "blockquote",
  "pre",
  "hr",
  "ul",
  "ol",
  "li",
  "table",
  "thead",
  "tbody",
  "tr",
  "th",
  "td",
  "dl",
  "dt",
  "dd",
  "div",
  "img",
]);

export async function renderMarkdownToElement({text, element, basePath = "", macros = {}}) {
  const html = await renderMarkdownToHtml({text, basePath, macros});
  element.innerHTML = html;
  element.classList.add("md-view");
  element.dispatchEvent(new CustomEvent("md-preview:render", {
    bubbles: true,
    detail: {basePath},
  }));
  return element;
}

export async function renderMarkdownToHtml({text, basePath = "", macros = {}}) {
  const transformed = await runConfiguredTransforms(text, {basePath, macros});
  const processor = createMarkdownParser()
    .use(remarkRehype, createRemarkRehypeOptions({
      basePath,
    }))
    .use(rehypeAttachSourceOffsets)
    .use(rehypeStringify, {allowDangerousHtml: true});

  const file = await processor.process(prepareMarkdownSource(transformed));
  return String(file);
}

export function from_text(text) {
  return fromText(text);
}
let transformRunnerPromise = null;

async function runConfiguredTransforms(text, context) {
  if (!transformRunnerPromise) {
    const transformRunnerUrl = new URL("./transform_runner.js", import.meta.url);
    transformRunnerPromise = import(transformRunnerUrl.href);
  }
  const {runTransforms} = await transformRunnerPromise;
  return runTransforms(text, context);
}

function rehypeAttachSourceOffsets() {
  return tree => {
    annotateSourceOffsets(tree);
  };
}

function annotateSourceOffsets(node) {
  if (!node || typeof node !== "object") {
    return;
  }

  if (node.type === "element" && SOURCE_MAPPED_TAGS.has(node.tagName)) {
    const start = node.position?.start?.offset;
    const end = node.position?.end?.offset;
    if (Number.isInteger(start) && Number.isInteger(end) && end >= start) {
      node.properties = {
        ...(node.properties || {}),
        "data-source-start": String(start),
        "data-source-end": String(end),
      };
    }
  }

  if (!Array.isArray(node.children)) {
    return;
  }

  for (const child of node.children) {
    annotateSourceOffsets(child);
  }
}
