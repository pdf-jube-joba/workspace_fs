import katex from "katex";
import remarkRehype from "remark-rehype";
import rehypeStringify from "rehype-stringify";
import {
  fromText,
  injectRenderedMath,
  renderMathMarkdown,
} from "../../katex_pre/src/katex_pre.js";
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
  const prepared = renderMathMarkdown(text, {
    macros,
    renderToString(value, options) {
      return katex.renderToString(value, options);
    },
  });
  const processor = createMarkdownParser()
    .use(remarkRehype, createRemarkRehypeOptions({
      basePath,
    }))
    .use(rehypeStringify, {allowDangerousHtml: true});

  const file = await processor.process(prepareMarkdownSource(prepared.text));
  return injectRenderedMath(String(file), prepared.replacements);
}

export function from_text(text) {
  return fromText(text);
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
