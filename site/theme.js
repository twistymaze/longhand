// The light and dark mode button. The page follows the system mode until
// the visitor clicks the button. Then data-theme on the html element holds
// the chosen mode, and the browser keeps it for the next visit. This file
// loads in the head, so a kept mode applies before the page shows.

const KEY = "longhand-theme";
const root = document.documentElement;
const system = window.matchMedia("(prefers-color-scheme: dark)");

function stored() {
  try {
    const mode = localStorage.getItem(KEY);
    return mode === "light" || mode === "dark" ? mode : null;
  } catch {
    return null;
  }
}

function current() {
  return root.dataset.theme || (system.matches ? "dark" : "light");
}

const kept = stored();
if (kept) root.dataset.theme = kept;

document.addEventListener("DOMContentLoaded", () => {
  const button = document.querySelector(".theme");
  const label = button.querySelector(".theme-label");
  const show = () => {
    label.textContent = current() === "dark" ? "Light mode" : "Dark mode";
    button.title = `Show the page in ${label.textContent.toLowerCase()}`;
  };
  button.addEventListener("click", () => {
    const mode = current() === "dark" ? "light" : "dark";
    root.dataset.theme = mode;
    try {
      localStorage.setItem(KEY, mode);
    } catch {
      // The mode applies to this visit only.
    }
    show();
  });
  system.addEventListener("change", show);
  show();
  button.hidden = false;
});
