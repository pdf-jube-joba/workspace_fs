import {renderMarkdownToElement} from "./markdown_viewer.js";
import {fetchTextFile, loadMacros, normalizePath} from "./markdown_runtime.js";
import {
  applyTheme,
  detectNavigationMode,
  directoryUrl,
  directoryViewHref,
  editorHref,
  fetchDirectoryEntries,
  fetchOptionalText,
  isNotFoundLike,
  joinPath,
  parentDirectoryPath,
  previewHref,
  setLinkState,
} from "./viewer_common.js";

const pathText = document.querySelector("#path-text");
const preview = document.querySelector("#preview");
const navigation = document.querySelector("#navigation");
const sidebarToggle = document.querySelector("#sidebar-toggle");
const editLink = document.querySelector("#edit-link");
const homeLink = document.querySelector("#home-link");
const upLink = document.querySelector("#up-link");
const statusText = document.querySelector("#status-text");
const app = document.querySelector(".app");
const sidebarStateKey = "md-preview-sidebar-collapsed";
let navigationKeyboardTargets = [];

function setStatus(message, isError = false) {
  statusText.textContent = message;
  statusText.classList.toggle("status-error", isError);
}

function setSidebarCollapsed(collapsed) {
  app.classList.toggle("sidebar-collapsed", collapsed);
  sidebarToggle.setAttribute("aria-expanded", String(!collapsed));
  sidebarToggle.setAttribute("aria-label", collapsed ? "Show navigation" : "Hide navigation");
  sidebarToggle.textContent = collapsed ? "▸" : "◂";
  window.localStorage.setItem(sidebarStateKey, collapsed ? "1" : "0");
}

function initializeSidebarState() {
  setSidebarCollapsed(window.localStorage.getItem(sidebarStateKey) === "1");
}

function isExternalLink(path) {
  return /^[a-z][a-z0-9+.-]*:/i.test(path);
}

function canonicalPreviewPath(path) {
  if (!path || isExternalLink(path)) {
    return "";
  }
  return normalizePath(new URL(String(path), window.location.origin).pathname);
}

function toDisplayPath(path) {
  return normalizePath(path) || "/";
}

function updateHeaderLinks(path) {
  editLink.href = editorHref(path);
  editLink.setAttribute("aria-disabled", path ? "false" : "true");
  setLinkState(homeLink, directoryViewHref());
  setLinkState(upLink, directoryViewHref({path: parentDirectoryPath(path)}));
}

function parseNavigationEntries(json, directory) {
  const rawItems = Array.isArray(json)
    ? json
    : Array.isArray(json?.items)
      ? json.items
      : null;
  if (!rawItems) {
    throw new Error("navigation.json must be an array or an object with an `items` array.");
  }

  return rawItems.map((item, index) => {
    if (typeof item === "string") {
      return {
        label: item,
        path: joinPath(directory, item),
      };
    }

    if (!item || typeof item !== "object") {
      throw new Error(`navigation.json entry ${index + 1} is invalid.`);
    }

    const label = String(item.name ?? item.title ?? item.label ?? item.path ?? item.link ?? item.href ?? "");
    const rawPath = item.path ?? item.link ?? item.href;
    if (!label || typeof rawPath !== "string" || !rawPath.trim()) {
      throw new Error(`navigation.json entry ${index + 1} must have a name and link target.`);
    }

    return {
      label,
      path: isExternalLink(rawPath) || rawPath.startsWith("/")
        ? rawPath
        : joinPath(directory, rawPath),
    };
  });
}

async function loadNavigation(currentPath) {
  const directory = parentDirectoryPath(currentPath);
  const navigationPath = joinPath(directory, "navigation.json");
  let navigationUnavailable = false;

  try {
    const navigationText = await fetchOptionalText(navigationPath);
    if (navigationText !== null) {
      return {
        mode: "items",
        source: "navigation_json",
        items: parseNavigationEntries(JSON.parse(navigationText), directory),
      };
    }
    navigationUnavailable = true;
  } catch (error) {
    console.warn(`failed to load ${navigationPath}, falling back to directory listing`, error);
    if (isNotFoundLike(error)) {
      navigationUnavailable = true;
    }
  }

  try {
    const entries = await fetchDirectoryEntries(directory);
    return {
      mode: "items",
      source: "directory_listing",
      items: entries.map(entry => ({
        label: entry,
        path: joinPath(directory, entry),
      })),
    };
  } catch (error) {
    if (navigationUnavailable && isNotFoundLike(error)) {
      return {
        mode: "not_permitted",
        source: "directory_listing",
        items: [],
      };
    }
    throw error;
  }
}

function updateNavigationKeyboardTargets(result, currentPath) {
  if (result.mode !== "items" || result.source !== "navigation_json") {
    navigationKeyboardTargets = [];
    return;
  }

  const current = canonicalPreviewPath(currentPath);
  navigationKeyboardTargets = result.items
    .filter(item => canonicalPreviewPath(item.path).endsWith(".md"))
    .map(item => canonicalPreviewPath(item.path));

  if (!navigationKeyboardTargets.includes(current)) {
    navigationKeyboardTargets = [];
  }
}

function navigateRelativeFromKeyboard(offset) {
  const current = canonicalPreviewPath(currentPathFromLocation());
  const index = navigationKeyboardTargets.indexOf(current);
  if (index === -1) {
    return;
  }
  const target = navigationKeyboardTargets[index + offset];
  if (target) {
    void navigateTo(target);
  }
}

function createNavigationLink(item, currentPath) {
  const listItem = document.createElement("li");
  listItem.className = "navigation-item";

  const label = document.createElement("span");
  label.className = "navigation-label";
  label.textContent = item.label;

  const normalizedCurrent = canonicalPreviewPath(currentPath);
  const normalizedItemPath = canonicalPreviewPath(item.path);

  const meta = document.createElement("span");
  meta.className = "navigation-meta";
  meta.textContent = toDisplayPath(item.path);

  if (normalizedItemPath === normalizedCurrent) {
    const current = document.createElement("span");
    current.className = "navigation-current";
    current.append(label, meta);
    listItem.append(current);
    return listItem;
  }

  const link = document.createElement("a");
  link.className = "navigation-link";
  if (isExternalLink(item.path)) {
    link.href = item.path;
    link.target = "_blank";
    link.rel = "noreferrer";
  } else if (item.path.endsWith(".md")) {
    const targetPath = canonicalPreviewPath(item.path);
    link.href = previewHref(targetPath);
    link.dataset.previewPath = targetPath;
  } else if (item.path.endsWith("/")) {
    link.href = directoryUrl(item.path);
  } else if (item.path.startsWith("/")) {
    link.href = item.path;
  } else {
    link.href = `/${normalizePath(item.path)}`;
  }

  link.append(label, meta);
  listItem.append(link);
  return listItem;
}

function renderNavigation(result, currentPath) {
  navigation.innerHTML = "";
  applyTheme({
    view: "md",
    navigation: result.source === "navigation_json" ? "navigation" : "listing",
  });
  updateNavigationKeyboardTargets(result, currentPath);

  if (result.mode === "not_permitted") {
    const empty = document.createElement("div");
    empty.className = "navigation-empty";
    empty.textContent = "not_permitted";
    navigation.append(empty);
    return;
  }

  if (!result.items.length) {
    const empty = document.createElement("div");
    empty.className = "navigation-empty";
    empty.textContent = "No entries found.";
    navigation.append(empty);
    return;
  }

  const list = document.createElement("ul");
  list.className = "navigation-list";
  for (const item of result.items) {
    list.append(createNavigationLink(item, currentPath));
  }
  navigation.append(list);
}

async function loadFile(path) {
  if (!path) {
    applyTheme({view: "md", navigation: "listing"});
    setStatus("Query parameter `path` is required.", true);
    return;
  }

  const normalizedPath = normalizePath(path);
  pathText.textContent = normalizedPath;
  updateHeaderLinks(normalizedPath);
  setStatus(`Loading ${normalizedPath} ...`);
  preview.innerHTML = "";
  navigation.innerHTML = "";

  const navigationPromise = loadNavigation(normalizedPath).then(result => {
    renderNavigation(result, normalizedPath);
    return result;
  });

  try {
    const [text, macros] = await Promise.all([
      fetchTextFile(normalizedPath),
      loadMacros(),
    ]);
    await renderMarkdownToElement({
      text,
      element: preview,
      basePath: normalizedPath,
      macros,
    });
    await navigationPromise;
    setStatus(`Loaded ${normalizedPath}.`);
  } catch (error) {
    preview.innerHTML = "";
    navigation.innerHTML = "";
    applyTheme({
      view: "md",
      navigation: await detectNavigationMode(parentDirectoryPath(normalizedPath)).catch(() => "listing"),
    });
    setStatus(String(error), true);
  }
}

function currentPathFromLocation() {
  return normalizePath(new URL(window.location.href).searchParams.get("path") || "");
}

async function navigateTo(path, {pushHistory = true} = {}) {
  const normalizedPath = normalizePath(path);
  if (!normalizedPath) {
    pathText.textContent = "(missing)";
    updateHeaderLinks("");
    preview.innerHTML = "";
    navigation.innerHTML = "";
    applyTheme({view: "md", navigation: "listing"});
    setStatus("Query parameter `path` is required.", true);
    return;
  }

  if (pushHistory) {
    const url = new URL(window.location.href);
    url.searchParams.set("path", normalizedPath);
    window.history.pushState({path: normalizedPath}, "", url);
  }

  await loadFile(normalizedPath);
}

navigation.addEventListener("click", event => {
  const link = event.target.closest("a[data-preview-path]");
  if (!link) {
    return;
  }

  event.preventDefault();
  void navigateTo(link.dataset.previewPath);
});

sidebarToggle.addEventListener("click", () => {
  setSidebarCollapsed(!app.classList.contains("sidebar-collapsed"));
});

window.addEventListener("popstate", () => {
  void navigateTo(currentPathFromLocation(), {pushHistory: false});
});

window.addEventListener("keydown", event => {
  if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) {
    return;
  }
  if (document.activeElement?.tagName === "TEXTAREA") {
    return;
  }
  if (event.key === "ArrowLeft") {
    event.preventDefault();
    navigateRelativeFromKeyboard(-1);
  }
  if (event.key === "ArrowRight") {
    event.preventDefault();
    navigateRelativeFromKeyboard(1);
  }
});

initializeSidebarState();
applyTheme({view: "md", navigation: "listing"});
void navigateTo(currentPathFromLocation(), {pushHistory: false});
