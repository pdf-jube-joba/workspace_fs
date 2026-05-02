import {from_text} from "./markdown_viewer.js";

let macrosPromise;

export function normalizePath(value) {
  return (value || "").trim().replace(/^\/+/, "");
}

export function currentFileUrl(path) {
  if (!path) {
    throw new Error("path is empty");
  }
  return `/${path}`;
}

export async function fetchTextFile(path) {
  const response = await fetch(currentFileUrl(path), {
    method: "GET",
  });
  if (!response.ok) {
    throw new Error(`GET failed: ${response.status} ${response.statusText}`);
  }
  return response.text();
}

export async function loadMacros() {
  if (!macrosPromise) {
    macrosPromise = fetch(new URL("./macros.txt", window.location.href), {
      method: "GET",
    })
      .then(response => {
        if (response.status === 404) {
          return null;
        }
        if (!response.ok) {
          throw new Error(`GET failed: ${response.status} ${response.statusText}`);
        }
        return response.text();
      })
      .then(text => (text === null ? {} : from_text(text)))
      .catch(error => {
        console.error("failed to load macros.txt", error);
        throw error;
      });
  }

  return macrosPromise;
}
