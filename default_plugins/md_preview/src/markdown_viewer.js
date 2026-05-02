import remarkRehype from "remark-rehype";
import rehypeStringify from "rehype-stringify";
import {fromText} from "../katex/katex_pre.js";
import {createMarkdownParser, prepareMarkdownSource} from "./markdown_extensions.js";
import {createRemarkRehypeOptions} from "./markdown_to_hast.js";

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
