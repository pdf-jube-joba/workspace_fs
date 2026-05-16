import {renderMarkdownToElement} from "./markdown_viewer.js";
import {currentFileUrl, fetchTextFile, loadMacros, normalizePath} from "./markdown_runtime.js";
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
let sourceMappedPreviewElements = [];
let caretMirror = null;

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
  sourceMappedPreviewElements = Array.from(preview.querySelectorAll("[data-source-start][data-source-end]"));
  syncPreviewToCursor();
}

function resizeEditorToContent() {
  editor.style.height = "0px";
  editor.style.height = `${editor.scrollHeight}px`;
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

function scheduleCursorSync() {
  window.requestAnimationFrame(() => {
    syncPreviewToCursor();
  });
}

function setBusy(busy) {
  editor.disabled = busy;
  previewButton.disabled = busy;
  homeButton.disabled = busy || !currentPath;
  upButton.disabled = busy || !currentPath;
}

function syncPreviewToCursor() {
  if (!sourceMappedPreviewElements.length) {
    return;
  }

  const selectionStart = editor.selectionStart;
  if (!Number.isInteger(selectionStart)) {
    return;
  }

  const target = findSourceMappedElement(selectionStart);
  if (!target) {
    return;
  }

  const caretTop = getCaretTopWithinPreviewViewport(editor, selectionStart);
  const previewScrollTop = computePreviewScrollTop(target, selectionStart, caretTop);
  preview.scrollTop = previewScrollTop;
}

function findSourceMappedElement(offset) {
  let bestContaining = null;
  let bestContainingLength = Number.POSITIVE_INFINITY;
  let bestPrevious = null;
  let bestPreviousStart = -1;

  for (const element of sourceMappedPreviewElements) {
    const start = Number(element.dataset.sourceStart);
    const end = Number(element.dataset.sourceEnd);
    if (!Number.isFinite(start) || !Number.isFinite(end)) {
      continue;
    }

    if (start <= offset && offset <= end) {
      const length = end - start;
      if (length < bestContainingLength) {
        bestContaining = element;
        bestContainingLength = length;
      }
      continue;
    }

    if (start <= offset && start > bestPreviousStart) {
      bestPrevious = element;
      bestPreviousStart = start;
    }
  }

  return bestContaining || bestPrevious;
}

function computePreviewScrollTop(target, selectionStart, caretTop) {
  const start = Number(target.dataset.sourceStart);
  const end = Number(target.dataset.sourceEnd);
  const range = Math.max(1, end - start);
  const ratio = clamp((selectionStart - start) / range, 0, 1);
  const elementTop = target.offsetTop;
  const elementHeight = Math.max(target.offsetHeight, 1);
  const anchorTop = elementTop + elementHeight * ratio;
  const maxScrollTop = Math.max(0, preview.scrollHeight - preview.clientHeight);
  return clamp(anchorTop - caretTop, 0, maxScrollTop);
}

function getCaretTopWithinPreviewViewport(textarea, position) {
  const mirror = getCaretMirror(textarea);
  const span = mirror.querySelector("span");
  syncMirrorStyle(textarea, mirror);
  mirror.style.width = `${textarea.clientWidth}px`;
  mirror.textContent = "";
  mirror.append(document.createTextNode(textarea.value.slice(0, position)));
  span.textContent = textarea.value[position] || "\u200b";
  mirror.append(span);

  const style = window.getComputedStyle(textarea);
  const textareaRect = textarea.getBoundingClientRect();
  const previewRect = preview.getBoundingClientRect();
  const caretViewportTop = textareaRect.top
    + Number.parseFloat(style.borderTopWidth || "0")
    + span.offsetTop
    - textarea.scrollTop;
  return clamp(caretViewportTop - previewRect.top, 0, Math.max(0, preview.clientHeight - 1));
}

function getCaretMirror(textarea) {
  if (caretMirror) {
    return caretMirror;
  }

  caretMirror = document.createElement("div");
  const marker = document.createElement("span");
  marker.textContent = "\u200b";
  caretMirror.append(marker);

  const mirrorStyle = caretMirror.style;
  mirrorStyle.position = "absolute";
  mirrorStyle.visibility = "hidden";
  mirrorStyle.pointerEvents = "none";
  mirrorStyle.whiteSpace = "pre-wrap";
  mirrorStyle.wordWrap = "break-word";
  mirrorStyle.overflowWrap = "break-word";
  mirrorStyle.left = "-9999px";
  mirrorStyle.top = "0";

  document.body.append(caretMirror);
  return caretMirror;
}

function syncMirrorStyle(textarea, mirror) {
  const style = window.getComputedStyle(textarea);
  const properties = [
    "boxSizing",
    "borderTopWidth",
    "borderRightWidth",
    "borderBottomWidth",
    "borderLeftWidth",
    "paddingTop",
    "paddingRight",
    "paddingBottom",
    "paddingLeft",
    "fontFamily",
    "fontSize",
    "fontStyle",
    "fontVariantLigatures",
    "fontWeight",
    "letterSpacing",
    "lineHeight",
    "tabSize",
    "textIndent",
    "textTransform",
    "textRendering",
  ];

  for (const property of properties) {
    mirror.style[property] = style[property];
  }
}

function clamp(value, min, max) {
  return Math.min(Math.max(value, min), max);
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
    resizeEditorToContent();
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
  resizeEditorToContent();
  schedulePreviewUpdate();
  scheduleCursorSync();
});

editor.addEventListener("click", scheduleCursorSync);
editor.addEventListener("keyup", scheduleCursorSync);
editor.addEventListener("scroll", scheduleCursorSync);
editor.addEventListener("select", scheduleCursorSync);
window.addEventListener("scroll", scheduleCursorSync, {passive: true});
window.addEventListener("resize", () => {
  resizeEditorToContent();
  scheduleCursorSync();
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
