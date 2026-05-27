// Ruscker docs — a "collapse sidebar" chevron.
//
// mdBook's only sidebar control is the hamburger ("Toggle Table of
// Contents") in the top bar; people expect a chevron *on the sidebar*
// to close it. mdBook toggles the sidebar with a hidden checkbox
// (#mdbook-sidebar-toggle-anchor) driven by `<label for=…>`, so we add a
// second label pointing at the same anchor — native toggle, no custom
// state to keep in sync. The chevron lives inside the sidebar, so it
// naturally disappears when collapsed; reopen with the hamburger.
(function () {
  function inject() {
    var sidebar = document.getElementById("mdbook-sidebar") ||
      document.querySelector(".sidebar");
    var anchor = document.getElementById("mdbook-sidebar-toggle-anchor");
    if (!sidebar || !anchor) return;
    if (sidebar.querySelector(".ruscker-sidebar-close")) return;

    var label = document.createElement("label");
    label.className = "ruscker-sidebar-close";
    label.setAttribute("for", "mdbook-sidebar-toggle-anchor");
    label.setAttribute("title", "Collapse sidebar");
    label.setAttribute("aria-label", "Collapse sidebar");
    // Inline SVG chevron — mdBook 0.5.x no longer ships Font Awesome, so
    // an <i class="fa …"> would render blank. `currentColor` inherits
    // the label's color (and the white-on-teal hover).
    label.innerHTML =
      '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" ' +
      'stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" ' +
      'aria-hidden="true"><polyline points="15 18 9 12 15 6"></polyline></svg>';
    sidebar.appendChild(label);
  }

  if (document.readyState !== "loading") {
    inject();
  } else {
    document.addEventListener("DOMContentLoaded", inject);
  }
})();
