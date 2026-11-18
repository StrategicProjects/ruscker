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

// Ruscker docs — diagrams follow the theme switch, and the title leads home.
//
// The diagrams are SVG files in the light palette that carry one
// <style media="(prefers-color-scheme: dark)"> block mapping it to the
// dark one (scripts/diagram-theme.py). As an <img>, a diagram can only
// follow the SYSTEM's scheme, not the theme picked in the book's menu.
// So each one is fetched and put in the page as inline SVG, and that
// block's `media` is switched on or off with the theme. Without
// JavaScript (or if the fetch fails) the <img> stays and follows the
// system — still readable either way.
(function () {
  function isDark() {
    var c = document.documentElement.classList;
    return c.contains("navy") || c.contains("coal") || c.contains("ayu");
  }
  function applyScheme() {
    var media = isDark() ? "all" : "not all";
    document.querySelectorAll("svg.rk-d style[data-rk-dark]").forEach(function (s) {
      s.setAttribute("media", media);
    });
  }
  function inline(img) {
    fetch(img.src)
      .then(function (r) { return r.ok ? r.text() : Promise.reject(); })
      .then(function (text) {
        var svg = new DOMParser().parseFromString(text, "image/svg+xml").documentElement;
        if (!svg || svg.nodeName !== "svg" || !svg.classList.contains("rk-d")) return;
        if (img.alt) svg.setAttribute("aria-label", img.alt);
        if (img.getAttribute("width")) svg.style.maxWidth = img.getAttribute("width") + "px";
        img.replaceWith(document.importNode(svg, true));
        applyScheme();
      })
      .catch(function () {});
  }
  function start() {
    document.querySelectorAll('.content img[src$=".svg"]:not(.ruscker-logo)').forEach(inline);
    new MutationObserver(applyScheme).observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class"],
    });

    // The dark theme sits in mdBook's "navy" slot; call it what it is.
    var navy = document.getElementById("mdbook-theme-navy");
    if (navy) navy.textContent = "Dark";

    // The book's title in the top bar becomes a link to the landing page.
    var title = document.querySelector(".menu-title");
    if (title && !title.querySelector("a")) {
      var root = typeof path_to_root === "string" ? path_to_root : "";
      var a = document.createElement("a");
      a.href = root + "index.html";
      a.title = "Ruscker — home";
      var logo = document.createElement("img");
      logo.src = root + "logo.svg";
      logo.alt = "";
      a.appendChild(logo);
      a.appendChild(document.createTextNode(title.textContent.trim()));
      title.textContent = "";
      title.appendChild(a);
    }
  }
  if (document.readyState !== "loading") {
    start();
  } else {
    document.addEventListener("DOMContentLoaded", start);
  }
})();
