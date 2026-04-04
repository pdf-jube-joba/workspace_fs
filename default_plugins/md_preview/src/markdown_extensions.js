import {unified} from "unified";
import remarkGfm from "remark-gfm";
import remarkParse from "remark-parse";

const WIKI_LINK_PATTERN = /\[\[([^[\]\n]+)\]\]/g;

export function createMarkdownParser() {
  return unified()
    .use(remarkParse)
    .use(remarkGfm)
    .use(remarkComputationExtensions);
}

export function parseMarkdown(text) {
  const parser = createMarkdownParser();
  return parser.parse(prepareMarkdownSource(text));
}

export function transformMarkdownAst(tree) {
  remarkComputationExtensions()(tree);
  return tree;
}

export function extractWikiLinkTermsFromMarkdown(text) {
  const tree = transformMarkdownAst(parseMarkdown(text));
  const seen = new Set();
  const terms = [];

  walkNode(tree, node => {
    if (node?.type !== "wikiLink" || !node.term || seen.has(node.term)) {
      return;
    }
    seen.add(node.term);
    terms.push(node.term);
  });

  return terms;
}

export function remarkComputationExtensions() {
  return tree => {
    transformNode(tree);
  };
}

export function prepareMarkdownSource(text) {
  return String(text || "");
}

function transformNode(node) {
  if (!node || !Array.isArray(node.children)) {
    return;
  }

  if (node.type === "code" || node.type === "inlineCode") {
    return;
  }

  const nextChildren = [];
  for (const child of node.children) {
    if (child.type === "paragraph") {
      const definitionList = transformDefinitionListParagraph(child);
      if (definitionList) {
        nextChildren.push(definitionList);
        continue;
      }
    }

    if (child.type === "blockquote") {
      nextChildren.push(transformBlockquoteNode(child));
      continue;
    }

    if (child.type === "text") {
      nextChildren.push(...splitSpecialTextNode(child));
      continue;
    }

    transformNode(child);
    nextChildren.push(child);
  }

  node.children = nextChildren;
  mergeAdjacentDefinitionLists(node);
}

function transformBlockquoteNode(node) {
  if (!Array.isArray(node.children) || node.children.length === 0) {
    return node;
  }

  const [firstChild, ...restChildren] = node.children;
  if (firstChild.type !== "paragraph" || !Array.isArray(firstChild.children) || firstChild.children.length === 0) {
    transformNode(node);
    return node;
  }

  const marker = parseAlertMarker(firstChild.children[0]);
  if (!marker) {
    transformNode(node);
    return node;
  }

  const nextFirstParagraph = {
    ...firstChild,
    children: trimLeadingWhitespace([
      ...marker.remainingChildren,
      ...firstChild.children.slice(1),
    ]),
  };

  const nextChildren = [];
  if (nextFirstParagraph.children.length > 0) {
    nextChildren.push(nextFirstParagraph);
  }
  nextChildren.push(...restChildren);

  const alertNode = {
    type: "alert",
    alertType: marker.alertType,
    children: nextChildren,
  };
  transformNode(alertNode);
  return alertNode;
}

function parseAlertMarker(node) {
  if (!node || node.type !== "text") {
    return null;
  }

  const match = /^(?:\s*)\[!([A-Za-z]+)\](?:\s+|$)/i.exec(node.value || "");
  if (!match) {
    return null;
  }

  const [, alertType] = match;
  const remainder = node.value.slice(match[0].length);
  const remainingChildren = remainder ? [{type: "text", value: remainder}] : [];
  return {
    alertType,
    remainingChildren,
  };
}

function splitSpecialTextNode(node) {
  const text = node.value || "";
  const wikiPieces = splitWikiLinkText(text);
  const pieces = [];

  for (const piece of wikiPieces) {
    if (piece.type === "wikiLink") {
      pieces.push({
        type: "wikiLink",
        term: piece.term,
        children: [{type: "text", value: piece.term}],
      });
      continue;
    }

    pieces.push(makeTextNode(piece.value));
  }

  return pieces.filter(isNonEmptyNode);
}

function splitWikiLinkText(text) {
  const pieces = [];
  let index = 0;

  for (const match of String(text || "").matchAll(WIKI_LINK_PATTERN)) {
    const rawTerm = match[1] ?? "";
    const term = normalizeWikiLinkTerm(rawTerm);
    const start = match.index ?? 0;
    const end = start + match[0].length;

    if (start > index) {
      pieces.push({type: "text", value: text.slice(index, start)});
    }

    if (term) {
      pieces.push({type: "wikiLink", term});
    } else {
      pieces.push({type: "text", value: match[0]});
    }

    index = end;
  }

  if (index < text.length) {
    pieces.push({type: "text", value: text.slice(index)});
  }

  return pieces;
}

function normalizeWikiLinkTerm(value) {
  return String(value || "").trim();
}

function transformDefinitionListParagraph(node) {
  const lines = splitInlineChildrenByNewline(node.children || []);
  if (lines.length < 2) {
    return null;
  }

  const termChildren = trimTrailingWhitespace(lineChildrenWithoutTrailingBreak(lines[0]));
  if (termChildren.length === 0) {
    return null;
  }

  const definitions = [];
  let currentDefinition = null;
  for (const line of lines.slice(1)) {
    const marker = extractDefinitionLine(line);
    if (marker) {
      currentDefinition = {
        type: "paragraph",
        children: trimTrailingWhitespace(trimLeadingWhitespace(marker.children)),
      };
      definitions.push(currentDefinition);
      continue;
    }

    if (!currentDefinition) {
      return null;
    }

    if (currentDefinition.children.length > 0) {
      currentDefinition.children.push(makeTextNode("\n"));
    }
    currentDefinition.children.push(...line);
    currentDefinition.children = trimTrailingWhitespace(currentDefinition.children);
  }

  if (definitions.length === 0 || definitions.some(definition => definition.children.length === 0)) {
    return null;
  }

  const termParagraph = {type: "paragraph", children: termChildren};
  transformNode(termParagraph);
  for (const definition of definitions) {
    transformNode(definition);
  }

  return {
    type: "definitionList",
    children: [{
      type: "definitionItem",
      termChildren: termParagraph.children,
      definitions,
    }],
  };
}

function splitInlineChildrenByNewline(children) {
  const lines = [[]];

  for (const child of children) {
    if (child.type !== "text") {
      lines.at(-1).push(child);
      continue;
    }

    const parts = child.value.split("\n");
    parts.forEach((part, index) => {
      if (part.length > 0) {
        lines.at(-1).push({...child, value: part});
      }
      if (index < parts.length - 1) {
        lines.push([]);
      }
    });
  }

  return lines;
}

function extractDefinitionLine(children) {
  if (children.length === 0) {
    return null;
  }

  const [firstChild, ...restChildren] = children;
  if (firstChild.type !== "text") {
    return null;
  }

  const match = /^(\s*):(?:[ \t]+|$)/.exec(firstChild.value || "");
  if (!match) {
    return null;
  }

  const remainder = firstChild.value.slice(match[0].length);
  const nextChildren = remainder.length > 0
    ? [{...firstChild, value: remainder}, ...restChildren]
    : restChildren;

  return {
    children: nextChildren,
  };
}

function trimLeadingWhitespace(children) {
  if (children.length === 0) {
    return children;
  }

  const [firstChild, ...restChildren] = children;
  if (firstChild.type !== "text") {
    return children;
  }

  const trimmedValue = firstChild.value.replace(/^\s+/, "");
  if (trimmedValue.length === 0) {
    return restChildren;
  }

  return [{...firstChild, value: trimmedValue}, ...restChildren];
}

function trimTrailingWhitespace(children) {
  if (children.length === 0) {
    return children;
  }

  const lastIndex = children.length - 1;
  const lastChild = children[lastIndex];
  if (lastChild.type !== "text") {
    return children;
  }

  const trimmedValue = lastChild.value.replace(/\s+$/, "");
  if (trimmedValue.length === 0) {
    return children.slice(0, lastIndex);
  }

  return [...children.slice(0, lastIndex), {...lastChild, value: trimmedValue}];
}

function lineChildrenWithoutTrailingBreak(children) {
  return children.filter(child => child.type !== "text" || child.value.length > 0);
}

function mergeAdjacentDefinitionLists(node) {
  if (!Array.isArray(node.children) || node.children.length < 2) {
    return;
  }

  const mergedChildren = [];
  for (const child of node.children) {
    const previous = mergedChildren.at(-1);
    if (previous?.type === "definitionList" && child.type === "definitionList") {
      previous.children.push(...child.children);
      continue;
    }
    mergedChildren.push(child);
  }
  node.children = mergedChildren;
}

function makeTextNode(value) {
  return {
    type: "text",
    value,
  };
}

function isNonEmptyNode(node) {
  return node.type !== "text" || node.value.length > 0;
}

function walkNode(node, visit) {
  if (!node || typeof node !== "object") {
    return;
  }
  visit(node);
  if (!Array.isArray(node.children)) {
    return;
  }
  for (const child of node.children) {
    walkNode(child, visit);
  }
}
