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
  const prepared = preprocessMathMarkdown(text, katexMacros);
  const processor = createMarkdownParser()
    .use(remarkRehype, createRemarkRehypeOptions({
      basePath,
    }))
    .use(rehypeStringify, {allowDangerousHtml: true});

  const file = await processor.process(prepareMarkdownSource(prepared.text));
  return injectMathPlaceholders(String(file), prepared.replacements);
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

function preprocessMathMarkdown(text, macros) {
  const source = String(text || "");
  const replacements = [];
  let index = 0;
  let result = "";
  let inFence = false;

  while (index < source.length) {
    if (isFenceStart(source, index)) {
      const lineEnd = findLineEnd(source, index);
      inFence = !inFence;
      result += source.slice(index, lineEnd);
      index = lineEnd;
      continue;
    }

    if (inFence) {
      result += source[index];
      index += 1;
      continue;
    }

    if (source[index] === "`") {
      const codeSpanEnd = findInlineCodeEnd(source, index);
      result += source.slice(index, codeSpanEnd);
      index = codeSpanEnd;
      continue;
    }

    const mathMatch = findNextMath(source, index);
    if (!mathMatch) {
      result += source.slice(index);
      break;
    }

    if (mathMatch.start > index) {
      result += source.slice(index, mathMatch.start);
    }

    if (mathMatch.error) {
      result += pushMathReplacement(
        replacements,
        renderMathError(source.slice(mathMatch.start), mathMatch.error, mathMatch.display),
      );
      break;
    }

    result += pushMathReplacement(
      replacements,
      renderKatexString(mathMatch.content, macros, mathMatch.display),
    );
    index = mathMatch.end;
  }

  return {
    text: result,
    replacements,
  };
}

function renderKatexString(value, macros, displayMode) {
  try {
    return katex.renderToString(value, {
      displayMode,
      throwOnError: true,
      macros,
      output: "html",
    });
  } catch (error) {
    return renderMathError(value, String(error), displayMode);
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

function isFenceStart(text, index) {
  if ((index > 0) && text[index - 1] !== "\n") {
    return false;
  }
  return text.startsWith("```", index) || text.startsWith("~~~", index);
}

function findLineEnd(text, index) {
  const newlineIndex = text.indexOf("\n", index);
  return newlineIndex === -1 ? text.length : newlineIndex + 1;
}

function findInlineCodeEnd(text, start) {
  let tickCount = 0;
  while (text[start + tickCount] === "`") {
    tickCount += 1;
  }
  const delimiter = "`".repeat(tickCount);
  const closeIndex = text.indexOf(delimiter, start + tickCount);
  return closeIndex === -1 ? text.length : closeIndex + tickCount;
}

function findNextMath(text, fromIndex) {
  const inlineStart = findInlineMathStart(text, fromIndex);
  const displayStart = findDisplayMathStart(text, fromIndex);

  let start = -1;
  let open = "";
  let close = "";
  let display = false;

  if (inlineStart !== -1 && (displayStart === -1 || inlineStart < displayStart)) {
    start = inlineStart;
    open = "\\(";
    close = "\\)";
  } else if (displayStart !== -1) {
    start = displayStart;
    open = "\\[";
    close = "\\]";
    display = true;
  }

  if (start === -1) {
    return null;
  }

  const contentStart = start + open.length;
  const closeIndex = display
    ? findDisplayMathClose(text, contentStart)
    : text.indexOf(close, contentStart);
  if (closeIndex === -1) {
    return {
      start,
      display,
      error: `missing closing delimiter for ${open}`,
    };
  }

  return {
    start,
    end: closeIndex + close.length,
    display,
    content: text.slice(contentStart, closeIndex),
  };
}

function findInlineMathStart(text, fromIndex) {
  for (let index = fromIndex; index < text.length - 2; index += 1) {
    if (text.startsWith("\\(", index)) {
      return index;
    }
  }
  return -1;
}

function findDisplayMathStart(text, fromIndex) {
  for (let index = fromIndex; index < text.length - 1; index += 1) {
    if (isLineStart(text, index) && text.startsWith("\\[", index)) {
      return index;
    }
  }
  return -1;
}

function findDisplayMathClose(text, fromIndex) {
  for (let index = fromIndex; index < text.length - 1; index += 1) {
    if (isLineStart(text, index) && text.startsWith("\\]", index)) {
      return index;
    }
  }
  return -1;
}

function pushMathReplacement(replacements, html) {
  const token = `KATEX_PLACEHOLDER_${replacements.length}_TOKEN`;
  replacements.push({token, html});
  return token;
}

function injectMathPlaceholders(html, replacements) {
  let result = html;
  for (const {token, html: replacementHtml} of replacements) {
    result = result.replaceAll(token, replacementHtml);
  }
  return result;
}

function isLineStart(text, index) {
  return index === 0 || text[index - 1] === "\n";
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
