import {currentFileUrl, fetchTextFile, normalizePath} from "./markdown_runtime.js";
import {
  applyTheme,
  detectNavigationMode,
  directoryViewHref,
  fetchDirectoryEntries,
  joinPath,
  parentDirectoryPath,
  previewHref,
  setLinkState,
} from "./viewer_common.js";

const PAGE_SIZE = 24;
const PREVIEW_LIMIT = 100;
const TEXT_EXTENSIONS = new Set(["md", "txt", "rs"]);
const DEFAULT_SORT = "newest";
const SORT_VALUES = new Set(["newest", "oldest", "abc"]);
const LINK_INDEX_URL = "./link_index.json";

const modeLabel = document.querySelector("#mode-label");
const pathText = document.querySelector("#path-text");
const statusText = document.querySelector("#status-text");
const boardPane = document.querySelector(".board-pane");
const cardGrid = document.querySelector("#card-grid");
const emptyState = document.querySelector("#empty-state");
const scrollSentinel = document.querySelector("#scroll-sentinel");
const homeLink = document.querySelector("#home-link");
const upLink = document.querySelector("#up-link");
const sortSelect = document.querySelector("#sort-select");
const newForm = document.querySelector("#new-form");
const newNameInput = document.querySelector("#new-name");
const newButton = document.querySelector("#new-button");

let entries = [];
let nextIndex = 0;
let loadingMore = false;
let observer;

function setStatus(message, isError = false) {
  statusText.textContent = message;
  statusText.classList.toggle("is-error", isError);
}

function currentParams() {
  return new URL(window.location.href).searchParams;
}

function currentSortFromLocation() {
  return currentParams().get("sort") || DEFAULT_SORT;
}

function currentSortMode() {
  return sortSelect.value || DEFAULT_SORT;
}

function markdownPreviewHref(path) {
  return previewHref(path);
}

function infoPathUrl(path) {
  const normalized = normalizePath(path);
  return normalized ? `/.info/${normalized}` : "/.info";
}

function setCreateEnabled(enabled) {
  newNameInput.disabled = !enabled;
  newButton.disabled = !enabled;
}

function setCurrentSortMode(value, {replaceHistory = true} = {}) {
  const sort = SORT_VALUES.has(value) ? value : DEFAULT_SORT;
  sortSelect.value = sort;
  if (replaceHistory) {
    const url = new URL(window.location.href);
    url.searchParams.set("sort", sort);
    window.history.replaceState(window.history.state, "", url);
  }
  return sort;
}

function fileExtension(path) {
  const name = path.split("/").pop() || "";
  const dot = name.lastIndexOf(".");
  return dot === -1 ? "" : name.slice(dot + 1).toLowerCase();
}

function cleanPeek(text) {
  return text.replace(/\s+/g, " ").trim().slice(0, PREVIEW_LIMIT);
}

function currentMode() {
  const params = currentParams();
  const link = (params.get("link") || "").trim();
  if (link) {
    return {kind: "link", value: link};
  }
  return {
    kind: "directory",
    value: normalizePath(params.get("path") || "").replace(/\/+$/, ""),
  };
}

async function createEmptyMarkdownFile(path) {
  const response = await fetch(currentFileUrl(path), {
    method: "POST",
    headers: {
      "Content-Type": "text/plain; charset=utf-8",
    },
    body: "",
  });

  if (!response.ok) {
    const detail = await response.text().catch(() => "");
    throw new Error(`POST failed: ${response.status} ${detail || response.statusText}`);
  }
}

async function fetchPathInfo(path) {
  const response = await fetch(infoPathUrl(path), {
    method: "GET",
  });
  if (!response.ok) {
    throw new Error(`GET failed: ${response.status} ${response.statusText}`);
  }
  return response.json();
}

async function fetchLinkIndex() {
  const response = await fetch(LINK_INDEX_URL, {
    method: "GET",
  });
  if (!response.ok) {
    throw new Error(`GET failed: ${response.status} ${response.statusText}`);
  }
  return response.json();
}

async function loadEntriesForDirectory(directory) {
  const names = await fetchDirectoryEntries(directory);
  const results = await Promise.allSettled(names.map(async name => {
    const path = joinPath(directory, name);
    const info = await fetchPathInfo(path);
    return {
      name,
      title: name,
      path,
      modifiedAt: info.modified_at,
    };
  }));

  return results
    .filter(result => result.status === "fulfilled")
    .map(result => result.value);
}

async function loadEntriesForLinkTerm(term) {
  const linkIndex = await fetchLinkIndex();
  const pages = linkIndex?.terms?.[term]?.pages;
  if (!Array.isArray(pages)) {
    return [];
  }

  const results = await Promise.allSettled(pages.map(async page => {
    const path = normalizePath(page.path || "");
    if (!path) {
      throw new Error("invalid page path");
    }
    const info = await fetchPathInfo(path);
    return {
      name: path,
      title: path,
      path,
      modifiedAt: info.modified_at,
    };
  }));

  return results
    .filter(result => result.status === "fulfilled")
    .map(result => result.value);
}

function sortEntries(items, sortMode) {
  const collator = new Intl.Collator("en", {numeric: true, sensitivity: "base"});
  const indexed = items.map((item, index) => ({item, index}));
  indexed.sort((left, right) => {
    if (sortMode === "abc") {
      const byName = collator.compare(left.item.title, right.item.title);
      return byName || left.index - right.index;
    }

    const leftTime = left.item.modifiedAt ? Date.parse(left.item.modifiedAt) : Number.NaN;
    const rightTime = right.item.modifiedAt ? Date.parse(right.item.modifiedAt) : Number.NaN;
    const leftHasTime = Number.isFinite(leftTime);
    const rightHasTime = Number.isFinite(rightTime);

    if (leftHasTime && rightHasTime && leftTime !== rightTime) {
      return sortMode === "oldest" ? leftTime - rightTime : rightTime - leftTime;
    }
    if (leftHasTime !== rightHasTime) {
      return leftHasTime ? -1 : 1;
    }

    const byName = collator.compare(left.item.title, right.item.title);
    return byName || left.index - right.index;
  });
  return indexed.map(entry => entry.item);
}

async function buildDirectoryCard(entry) {
  const children = await fetchDirectoryEntries(entry.path);
  let peek = children.join(" ").slice(0, PREVIEW_LIMIT);

  if (children.includes("README.md")) {
    try {
      peek = cleanPeek(await fetchTextFile(joinPath(entry.path, "README.md")));
    } catch {
      // Keep listing preview when README.md cannot be read.
    }
  }

  return {
    kind: "directory",
    title: entry.title.replace(/\/$/, ""),
    peek,
    href: directoryViewHref({path: entry.path, sort: currentSortMode()}),
  };
}

async function buildTextFileCard(entry) {
  const text = await fetchTextFile(entry.path);
  const extension = fileExtension(entry.path);
  return {
    kind: extension,
    title: entry.title,
    peek: cleanPeek(text),
    href: extension === "md" ? markdownPreviewHref(entry.path) : `/${entry.path}`,
  };
}

async function buildCard(entry) {
  if (entry.name.endsWith("/")) {
    return buildDirectoryCard(entry);
  }

  const extension = fileExtension(entry.name);
  if (!TEXT_EXTENSIONS.has(extension)) {
    return null;
  }

  return buildTextFileCard(entry);
}

function renderCard(card) {
  const link = document.createElement("a");
  link.className = "card";
  link.href = card.href;

  const kind = document.createElement("div");
  kind.className = "card-kind";
  kind.textContent = card.kind;

  const title = document.createElement("h2");
  title.className = "card-title";
  title.textContent = card.title;

  const peek = document.createElement("p");
  peek.className = "card-peek";
  peek.textContent = card.peek || "(empty)";

  link.append(kind, title, peek);
  cardGrid.append(link);
}

function updateEmptyState() {
  emptyState.hidden = cardGrid.childElementCount !== 0;
}

function needsMoreToScroll() {
  return boardPane.scrollHeight <= boardPane.clientHeight + 8;
}

async function loadNextPage() {
  if (loadingMore || nextIndex >= entries.length) {
    return;
  }

  loadingMore = true;
  const batch = entries.slice(nextIndex, nextIndex + PAGE_SIZE);
  nextIndex += batch.length;
  setStatus(`Loading ${Math.min(nextIndex, entries.length)} / ${entries.length} ...`);

  const results = await Promise.allSettled(batch.map(entry => buildCard(entry)));
  let appended = 0;
  for (const result of results) {
    if (result.status !== "fulfilled" || !result.value) {
      continue;
    }
    renderCard(result.value);
    appended += 1;
  }

  updateEmptyState();
  loadingMore = false;
  setStatus(
    nextIndex >= entries.length
      ? `Loaded ${cardGrid.childElementCount} cards.`
      : `Loaded ${cardGrid.childElementCount} cards. Scroll for more.`,
  );

  if (appended === 0 && nextIndex < entries.length) {
    await loadNextPage();
    return;
  }

  if (nextIndex < entries.length && needsMoreToScroll()) {
    await loadNextPage();
  }
}

function observeInfiniteScroll() {
  observer?.disconnect();
  observer = new IntersectionObserver(intersections => {
    for (const entry of intersections) {
      if (entry.isIntersecting) {
        void loadNextPage();
      }
    }
  }, {
    root: boardPane,
    rootMargin: "0px 0px 240px 0px",
  });
  observer.observe(scrollSentinel);
}

async function rerenderCards() {
  cardGrid.innerHTML = "";
  emptyState.hidden = true;
  nextIndex = 0;
  entries = sortEntries(entries, currentSortMode());

  if (!entries.length) {
    updateEmptyState();
    return;
  }

  await loadNextPage();
}

async function applyModeTheme(mode) {
  if (mode.kind === "link") {
    applyTheme({view: "directory", navigation: "navigation"});
    return;
  }

  const navigation = await detectNavigationMode(mode.value).catch(() => "listing");
  applyTheme({view: "directory", navigation});
}

async function loadView() {
  const mode = currentMode();
  cardGrid.innerHTML = "";
  emptyState.hidden = true;
  entries = [];
  nextIndex = 0;

  setLinkState(homeLink, directoryViewHref({sort: currentSortMode()}));
  setCreateEnabled(mode.kind === "directory");
  await applyModeTheme(mode);

  if (mode.kind === "link") {
    modeLabel.textContent = "Link";
    pathText.textContent = `[[${mode.value}]]`;
    setLinkState(upLink, "");
    setStatus(`Loading [[${mode.value}]] ...`);
    entries = await loadEntriesForLinkTerm(mode.value);
  } else {
    modeLabel.textContent = "Directory";
    pathText.textContent = mode.value ? `/${mode.value}/` : "/";
    setLinkState(
      upLink,
      mode.value ? directoryViewHref({path: parentDirectoryPath(mode.value), sort: currentSortMode()}) : "",
    );
    setStatus(`Loading ${mode.value ? `/${mode.value}/` : "/"} ...`);
    entries = await loadEntriesForDirectory(mode.value);
  }

  if (!entries.length) {
    setStatus(mode.kind === "link" ? "No linked pages found." : "Directory is empty.");
    updateEmptyState();
    return;
  }

  entries = sortEntries(entries, currentSortMode());
  observeInfiniteScroll();
  await loadNextPage();
}

async function handleCreateMarkdown(event) {
  event.preventDefault();
  const mode = currentMode();
  if (mode.kind !== "directory") {
    setStatus("New markdown is only available in directory mode.", true);
    return;
  }

  const rawName = newNameInput.value.trim();
  const normalizedName = normalizePath(rawName);
  if (!normalizedName) {
    setStatus("Enter a markdown filename.", true);
    return;
  }
  if (normalizedName.includes("/")) {
    setStatus("Filename must not include '/'.", true);
    return;
  }
  if (!normalizedName.endsWith(".md")) {
    setStatus("Filename must end with .md.", true);
    return;
  }

  const path = joinPath(mode.value, normalizedName);
  setCreateEnabled(false);
  setStatus(`Creating ${path} ...`);
  try {
    await createEmptyMarkdownFile(path);
    newNameInput.value = "";
    await loadView();
    setStatus(`Created ${path}.`);
  } catch (error) {
    setStatus(String(error), true);
  } finally {
    setCreateEnabled(currentMode().kind === "directory");
  }
}

sortSelect.addEventListener("change", () => {
  setCurrentSortMode(sortSelect.value);
  void rerenderCards();
});

newForm.addEventListener("submit", event => {
  void handleCreateMarkdown(event);
});

applyTheme({view: "directory", navigation: "listing"});
setCurrentSortMode(currentSortFromLocation());
void loadView().catch(error => {
  setStatus(String(error), true);
  updateEmptyState();
});
