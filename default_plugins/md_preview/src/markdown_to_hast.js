export function createRemarkRehypeOptions({ basePath }) {
  return {
    allowDangerousHtml: true,
    handlers: {
      definitionList(state, node) {
        return {
          type: "element",
          tagName: "dl",
          properties: {
            className: ["md-definition-list"],
          },
          children: node.children.flatMap(item => definitionItemToHast(state, item)),
        };
      },
      alert(state, node) {
        const type = normalizeAlertType(node.alertType);
        return {
          type: "element",
          tagName: "div",
          properties: {
            className: ["md-alert", `md-alert-${type}`],
          },
          children: [
            {
              type: "element",
              tagName: "p",
              properties: {
                className: ["md-alert-title"],
              },
              children: [{ type: "text", value: alertLabel(type) }],
            },
            {
              type: "element",
              tagName: "div",
              properties: {
                className: ["md-alert-body"],
              },
              children: state.all(node),
            },
          ],
        };
      },
      link(state, node) {
        const href = resolveMarkdownHref(node.url, basePath);
        return {
          type: "element",
          tagName: "a",
          properties: { href },
          children: state.all(node),
        };
      },
      wikiLink(state, node) {
        return {
          type: "element",
          tagName: "a",
          properties: { href: wikiLinkHref(node.term) },
          children: [{ type: "text", value: node.term }],
        };
      },
      image(state, node) {
        return {
          type: "element",
          tagName: "img",
          properties: {
            src: resolveAssetHref(node.url, basePath),
            alt: node.alt || "",
            title: node.title || undefined,
          },
          children: [],
        };
      },
    },
  };
}

export function wikiLinkHref(term) {
  return `./directory_view.html?link=${encodeURIComponent(term)}`;
}

function definitionItemToHast(state, item) {
  return [
    {
      type: "element",
      tagName: "dt",
      properties: {},
      children: state.all({ type: "paragraph", children: item.termChildren }),
    },
    ...item.definitions.map(definition => ({
      type: "element",
      tagName: "dd",
      properties: {},
      children: state.all({ type: "root", children: [definition] }),
    })),
  ];
}

function normalizeAlertType(value) {
  const normalized = String(value || "note").trim().toLowerCase();
  switch (normalized) {
    case "note":
    case "tip":
    case "important":
    case "warning":
    case "caution":
      return normalized;
    default:
      return "note";
  }
}

function alertLabel(type) {
  switch (type) {
    case "tip":
      return "Tip";
    case "important":
      return "Important";
    case "warning":
      return "Warning";
    case "caution":
      return "Caution";
    default:
      return "Note";
  }
}

function resolveMarkdownHref(href, basePath) {
  if (!href || isExternalHref(href) || href.startsWith("#")) {
    return href;
  }

  const { path, search, hash } = splitRelativeHref(href);
  const resolvedPath = resolveRepositoryPath(path, basePath);
  if (resolvedPath.endsWith(".md")) {
    return `./md_preview.html?path=${encodeURIComponent(resolvedPath)}${hash}`;
  }

  return `/${resolvedPath}${search}${hash}`;
}

function resolveAssetHref(href, basePath) {
  if (!href || isExternalHref(href) || href.startsWith("#")) {
    return href;
  }

  const { path, search, hash } = splitRelativeHref(href);
  return `/${resolveRepositoryPath(path, basePath)}${search}${hash}`;
}

function splitRelativeHref(href) {
  const hashIndex = href.indexOf("#");
  const beforeHash = hashIndex === -1 ? href : href.slice(0, hashIndex);
  const hash = hashIndex === -1 ? "" : href.slice(hashIndex);
  const searchIndex = beforeHash.indexOf("?");
  if (searchIndex === -1) {
    return {
      path: beforeHash,
      search: "",
      hash,
    };
  }

  return {
    path: beforeHash.slice(0, searchIndex),
    search: beforeHash.slice(searchIndex),
    hash,
  };
}

function resolveRepositoryPath(target, basePath) {
  const cleanTarget = target.replace(/^\/+/, "");
  if (target.startsWith("/")) {
    return cleanTarget;
  }

  const baseDir = basePath.includes("/") ? basePath.slice(0, basePath.lastIndexOf("/") + 1) : "";
  const joined = `${baseDir}${cleanTarget}`;
  const normalized = [];
  for (const segment of joined.split("/")) {
    if (!segment || segment === ".") {
      continue;
    }
    if (segment === "..") {
      normalized.pop();
      continue;
    }
    normalized.push(segment);
  }
  return normalized.join("/");
}

function isExternalHref(href) {
  return /^(?:[a-z]+:)?\/\//i.test(href) || href.startsWith("mailto:") || href.startsWith("tel:");
}
