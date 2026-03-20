import {renderMarkdownToElement} from "./markdown_viewer.js";
import {
  currentFileUrl,
  fetchTextFile,
  loadMacros,
  normalizePath,
  requestHeaders,
} from "./markdown_runtime.js";

const pathText = document.querySelector("#path-text");
const preview = document.querySelector("#preview");
const navigation = document.querySelector("#navigation");
const sidebarEdgeToggle = document.querySelector("#sidebar-edge-toggle");
const sidebarDividerToggle = document.querySelector("#sidebar-divider-toggle");
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
  sidebarEdgeToggle.setAttribute("aria-expanded", String(!collapsed));
  sidebarDividerToggle.setAttribute("aria-expanded", String(!collapsed));
  window.localStorage.setItem(sidebarStateKey, collapsed ? "1" : "0");
}

function initializeSidebarState() {
  const collapsed = window.localStorage.getItem(sidebarStateKey) === "1";
  setSidebarCollapsed(collapsed);
}

function splitPath(path) {
  const normalized = normalizePath(path);
  const lastSlash = normalized.lastIndexOf("/");
  if (lastSlash === -1) {
    return {
      directory: "",
      name: normalized,
    };
  }

  return {
    directory: normalized.slice(0, lastSlash),
    name: normalized.slice(lastSlash + 1),
  };
}

function joinPath(directory, entry) {
  const normalizedDirectory = normalizePath(directory);
  const normalizedEntry = normalizePath(entry);
  if (!normalizedDirectory) {
    return normalizedEntry;
  }
  if (!normalizedEntry) {
    return normalizedDirectory;
  }
  return `${normalizedDirectory}/${normalizedEntry}`;
}

function directoryUrl(directory) {
  const normalizedDirectory = normalizePath(directory);
  return normalizedDirectory ? `/${normalizedDirectory}/` : "/";
}

function previewHref(path) {
  return `./md_preview.html?path=${encodeURIComponent(path)}`;
}

function editorHref(path) {
  return `./md_editor.html?path=${encodeURIComponent(path)}`;
}

function directoryViewHref(path) {
  return `./directory_view.html?path=${encodeURIComponent(normalizePath(path))}`;
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

function setActionLinkState(link, href) {
  if (!href) {
    link.removeAttribute("href");
    link.setAttribute("aria-disabled", "true");
    return;
  }

  link.href = href;
  link.setAttribute("aria-disabled", "false");
}

async function updateHeaderLinks(path) {
  editLink.href = editorHref(path);
  editLink.setAttribute("aria-disabled", path ? "false" : "true");
  const {directory} = splitPath(path);
  setActionLinkState(homeLink, directoryViewHref(""));
  setActionLinkState(upLink, directoryViewHref(directory));
}

async function fetchOptionalText(path) {
  const response = await fetch(currentFileUrl(normalizePath(path)), {
    method: "GET",
    headers: requestHeaders(),
  });
  if (response.status === 404) {
    return null;
  }
  if (!response.ok) {
    const error = new Error(`GET failed: ${response.status} ${response.statusText}`);
    error.status = response.status;
    throw error;
  }
  return response.text();
}

async function fetchDirectoryEntries(directory) {
  const response = await fetch(directoryUrl(directory), {
    method: "GET",
    headers: requestHeaders(),
  });
  if (!response.ok) {
    const error = new Error(`GET failed: ${response.status} ${response.statusText}`);
    error.status = response.status;
    throw error;
  }

  const text = await response.text();
  if (!text.trim()) {
    return [];
  }
  return text.split("\n").map(entry => entry.trim()).filter(Boolean);
}

function isNotFoundLike(error) {
  return error?.status === 403 || error?.status === 404;
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
  const {directory} = splitPath(currentPath);
  const navigationPath = joinPath(directory, "navigation.json");
  let navigationUnavailable = false;
  try {
    const navigationText = await fetchOptionalText(navigationPath);
    if (navigationText !== null) {
      const parsed = JSON.parse(navigationText);
      return {
        mode: "items",
        source: "navigation_json",
        items: parseNavigationEntries(parsed, directory),
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

function isTextareaFocused() {
  return document.activeElement?.tagName === "TEXTAREA";
}

function isPreviewableNavigationTarget(path) {
  return canonicalPreviewPath(path).endsWith(".md");
}

function updateNavigationKeyboardTargets(result, currentPath) {
  if (result.mode !== "items" || result.source !== "navigation_json") {
    navigationKeyboardTargets = [];
    return;
  }

  const current = canonicalPreviewPath(currentPath);
  navigationKeyboardTargets = result.items
    .filter(item => isPreviewableNavigationTarget(item.path))
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
  if (!target) {
    return;
  }
  void navigateTo(target);
}

function createNavigationLink(item, currentPath) {
  const listItem = document.createElement("li");
  listItem.className = "navigation-item";

  const label = document.createElement("span");
  label.className = "navigation-label";
  label.textContent = item.label;

  const normalizedCurrent = canonicalPreviewPath(currentPath);
  const normalizedItemPath = canonicalPreviewPath(item.path);
  const isCurrent = normalizedItemPath === normalizedCurrent;

  const meta = document.createElement("span");
  meta.className = "navigation-meta";
  meta.textContent = toDisplayPath(item.path);

  if (isCurrent) {
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
  updateNavigationKeyboardTargets(result, currentPath);

  if (result.mode === "not_permitted") {
    const empty = document.createElement("div");
    empty.className = "navigation-empty";
    empty.textContent = "not_permitted";
    navigation.append(empty);
    return;
  }

  const {items} = result;

  if (!items.length) {
    const empty = document.createElement("div");
    empty.className = "navigation-empty";
    empty.textContent = "No entries found.";
    navigation.append(empty);
    return;
  }

  const list = document.createElement("ul");
  list.className = "navigation-list";
  for (const item of items) {
    list.append(createNavigationLink(item, currentPath));
  }
  navigation.append(list);
}

async function loadFile(path) {
  if (!path) {
    setStatus("Query parameter `path` is required.", true);
    return;
  }

  const normalizedPath = normalizePath(path);
  pathText.textContent = normalizedPath;
  void updateHeaderLinks(normalizedPath);
  setStatus(`Loading ${normalizedPath} ...`);
  preview.innerHTML = "";
  navigation.innerHTML = "";

  const navigationPromise = loadNavigation(normalizedPath)
    .then(result => {
      renderNavigation(result, normalizedPath);
      return result;
    })
    .catch(error => {
      navigation.innerHTML = "";
      throw error;
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
    try {
      await navigationPromise;
    } catch {
      navigation.innerHTML = "";
    }
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
    void updateHeaderLinks("");
    preview.innerHTML = "";
    navigation.innerHTML = "";
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

sidebarEdgeToggle.addEventListener("click", () => {
  setSidebarCollapsed(false);
});

sidebarDividerToggle.addEventListener("click", () => {
  setSidebarCollapsed(true);
});

window.addEventListener("popstate", () => {
  void navigateTo(currentPathFromLocation(), {pushHistory: false});
});

window.addEventListener("keydown", event => {
  if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) {
    return;
  }
  if (isTextareaFocused()) {
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
void navigateTo(currentPathFromLocation(), {pushHistory: false});
