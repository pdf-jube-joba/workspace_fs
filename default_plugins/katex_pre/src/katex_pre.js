export function fromText(text) {
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

export function renderMathMarkdown(text, {macros = {}, renderToString}) {
  const katexMacros = normalizeKatexMacros(macros);
  return preprocessMathMarkdown(String(text || ""), {
    renderMath(value, displayMode) {
      try {
        return renderToString(value, {
          displayMode,
          throwOnError: true,
          macros: katexMacros,
          output: "html",
        });
      } catch (error) {
        return renderMathError(value, String(error), displayMode);
      }
    },
  });
}

export function injectRenderedMath(html, replacements) {
  let result = html;
  for (const {token, html: replacementHtml} of replacements) {
    result = result.replaceAll(token, replacementHtml);
  }
  return result;
}

function normalizeKatexMacros(macros) {
  if (!macros || typeof macros !== "object" || Array.isArray(macros)) {
    return {};
  }
  return macros;
}

function preprocessMathMarkdown(source, {renderMath}) {
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
      renderMath(mathMatch.content, mathMatch.display),
    );
    index = mathMatch.end;
  }

  return {
    text: result,
    replacements,
  };
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

function isLineStart(text, index) {
  return index === 0 || text[index - 1] === "\n";
}
