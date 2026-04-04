import {currentFileUrl, normalizePath, requestHeaders} from "./markdown_runtime.js";

export function splitPath(path) {
  const normalized = normalizePath(path).replace(/\/+$/, "");
  if (!normalized) {
    return {
      directory: "",
      name: "",
    };
  }

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

export function parentDirectoryPath(path) {
  return splitPath(path).directory;
}

export function joinPath(directory, entry) {
  const normalizedDirectory = normalizePath(directory).replace(/\/+$/, "");
  const normalizedEntry = normalizePath(entry);
  if (!normalizedDirectory) {
    return normalizedEntry;
  }
  if (!normalizedEntry) {
    return normalizedDirectory;
  }
  return `${normalizedDirectory}/${normalizedEntry}`;
}

export function directoryUrl(directory) {
  const normalizedDirectory = normalizePath(directory).replace(/\/+$/, "");
  return normalizedDirectory ? `/${normalizedDirectory}/` : "/";
}

function createViewerHref(page, params = {}) {
  const url = new URL(`./${page}`, window.location.href);
  url.search = "";
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null || value === "") {
      continue;
    }
    url.searchParams.set(key, String(value));
  }
  return `${url.pathname}${url.search}`;
}

export function previewHref(path) {
  return createViewerHref("md_preview.html", {path: normalizePath(path)});
}

export function editorHref(path) {
  return createViewerHref("md_editor.html", {path: normalizePath(path)});
}

export function directoryViewHref({path = "", link = "", sort = ""} = {}) {
  return createViewerHref("directory_view.html", {
    path: normalizePath(path),
    link: link.trim(),
    sort,
  });
}

export function setLinkState(link, href) {
  if (!href) {
    link.removeAttribute("href");
    link.setAttribute("aria-disabled", "true");
    return;
  }

  link.href = href;
  link.setAttribute("aria-disabled", "false");
}

export function applyTheme({view, navigation}) {
  const body = document.body;
  body.classList.toggle("view-md", view === "md");
  body.classList.toggle("view-directory", view === "directory");
  body.classList.toggle("nav-listing", navigation === "listing");
  body.classList.toggle("nav-navigation", navigation === "navigation");
}

export async function fetchOptionalText(path) {
  const response = await fetch(currentFileUrl(normalizePath(path)), {
    method: "GET",
    headers: requestHeaders(),
  });
  if (response.status === 403 || response.status === 404) {
    return null;
  }
  if (!response.ok) {
    const error = new Error(`GET failed: ${response.status} ${response.statusText}`);
    error.status = response.status;
    throw error;
  }
  return response.text();
}

export async function fetchDirectoryEntries(directory) {
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

export async function detectNavigationMode(directory) {
  const navigationText = await fetchOptionalText(joinPath(directory, "navigation.json"));
  return navigationText !== null ? "navigation" : "listing";
}

export function isNotFoundLike(error) {
  return error?.status === 403 || error?.status === 404;
}
