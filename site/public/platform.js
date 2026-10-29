// Finds the operating system of the visitor for the download buttons. On a
// Mac or on Windows, data-platform on the html element gets "mac" or
// "windows", and the stylesheet makes that installer the main button.
// Otherwise the script sets nothing, and both buttons show as equals. This
// file loads in the head, so the buttons are final before the page shows.

{
  const nav = typeof navigator === "undefined" ? null : navigator;
  if (nav) {
    const name = `${nav.platform || ""} ${nav.userAgent || ""}`;
    // An iPad names a Mac too, but it has touch input. A Mac has 0 touch
    // points, or 1 in some browsers.
    if (/Macintosh|MacIntel/.test(name) && (nav.maxTouchPoints || 0) <= 1) {
      document.documentElement.dataset.platform = "mac";
    } else if (/Win/.test(name)) {
      document.documentElement.dataset.platform = "windows";
    }
  }
}
