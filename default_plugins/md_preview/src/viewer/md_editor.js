import {renderMarkdownToElement} from "./markdown_viewer.js";
import {currentFileUrl, fetchTextFile, loadMacros, normalizePath, requestHeaders} from "./markdown_runtime.js";
import {applyTheme, detectNavigationMode, directoryViewHref, parentDirectoryPath, previewHref} from "./viewer_common.js";

const preview = document.querySelector("#preview");
const editor = document.querySelector("#editor");
const statusText = document.querySelector("#status-text");
const previewButton = document.querySelector("#preview-button");
const homeButton = document.querySelector("#home-button");
const upButton = document.querySelector("#up-button");
const initialPath = normalizePath(new URL(window.location.href).searchParams.get("path") || "");
let previewTimer = null;
let currentPath = initialPath;

function setStatus(message, isError = false) {
  statusText.textContent = message;
  statusText.classList.toggle("status-error", isError);
}

function updateNavigationButtons() {
  const hasPath = Boolean(currentPath);
  homeButton.disabled = !hasPath;
  upButton.disabled = !hasPath;
}

async function updatePreview() {
  const macros = await loadMacros();
  await renderMarkdownToElement({
    text: editor.value,
    element: preview,
    basePath: currentPath,
    macros,
  });
}

function schedulePreviewUpdate() {
  if (previewTimer !== null) {
    window.clearTimeout(previewTimer);
  }
  previewTimer = window.setTimeout(() => {
    previewTimer = null;
    void updatePreview();
  }, 120);
}

function setBusy(busy) {
  editor.disabled = busy;
  previewButton.disabled = busy;
  homeButton.disabled = busy || !currentPath;
  upButton.disabled = busy || !currentPath;
}

async function updateTheme(path) {
  const navigation = path
    ? await detectNavigationMode(parentDirectoryPath(path)).catch(() => "listing")
    : "listing";
  applyTheme({view: "md", navigation});
}

async function loadFile(path) {
  const normalizedPath = normalizePath(path);
  if (!normalizedPath) {
    applyTheme({view: "md", navigation: "listing"});
    setStatus("Missing ?path=... in URL.", true);
    return;
  }

  setBusy(true);
  setStatus(`Loading ${normalizedPath} ...`);
  try {
    const [text] = await Promise.all([
      fetchTextFile(normalizedPath),
      updateTheme(normalizedPath),
    ]);
    editor.value = text;
    currentPath = normalizedPath;
    updateNavigationButtons();
    await updatePreview();
    setStatus(`Loaded ${normalizedPath}. Press Ctrl+S to save.`);
  } catch (error) {
    setStatus(String(error), true);
  } finally {
    setBusy(false);
  }
}

async function saveFile() {
  if (!currentPath) {
    setStatus("Missing ?path=... in URL.", true);
    return false;
  }

  setBusy(true);
  setStatus(`Saving ${currentPath} ...`);
  try {
    const response = await fetch(currentFileUrl(currentPath), {
      method: "PUT",
      headers: {
        ...requestHeaders(),
        "Content-Type": "text/plain; charset=utf-8",
      },
      body: editor.value,
    });

    if (!response.ok) {
      const detail = await response.text().catch(() => "");
      throw new Error(`PUT failed: ${response.status} ${detail || response.statusText}`);
    }

    await updatePreview();
    await updateTheme(currentPath);
    setStatus(`Saved ${currentPath}.`);
    return true;
  } catch (error) {
    setStatus(String(error), true);
    return false;
  } finally {
    setBusy(false);
  }
}

async function saveAndNavigate(targetHref) {
  const saved = await saveFile();
  if (saved) {
    window.location.href = targetHref;
  }
}

editor.addEventListener("input", () => {
  schedulePreviewUpdate();
});

previewButton.addEventListener("click", () => {
  void saveAndNavigate(previewHref(currentPath));
});

homeButton.addEventListener("click", () => {
  void saveAndNavigate(directoryViewHref());
});

upButton.addEventListener("click", () => {
  if (!currentPath) {
    return;
  }
  void saveAndNavigate(directoryViewHref({path: parentDirectoryPath(currentPath)}));
});

window.addEventListener("keydown", event => {
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
    event.preventDefault();
    void saveFile();
  }
});

applyTheme({view: "md", navigation: "listing"});
if (initialPath) {
  void loadFile(initialPath);
} else {
  updateNavigationButtons();
  schedulePreviewUpdate();
  setStatus("Missing ?path=... in URL.", true);
}
