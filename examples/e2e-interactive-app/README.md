# Interactive-app e2e harness

A minimal containerized app that stands in for Shiny / Streamlit
/ Dash when validating Ruscker's interactive-app support — HTML
URL rewriting, the JS runtime shim, and WebSocket proxying —
without pulling a heavyweight R/Python framework image.

## What's here

- **`app.py`** — an [aiohttp](https://docs.aiohttp.org) server
  that deliberately exercises every URL flavor an interactive
  app uses behind a reverse proxy:
  - serves HTML referencing **absolute-root** static assets
    (`/static/app.js`, `/static/app.css`)
  - the page's JS does a `fetch('/api/ping')` (absolute path)
  - and opens a `WebSocket` to an absolute path (`/ws`)
- **`Dockerfile`** — `python:3.12-slim` + aiohttp.
- **`wsapp.yml`** — a Ruscker config mounting the app as a
  `type: streamlit` (InteractiveApp) spec on inner port 8080.
- **`wsclient.py`** — a dependency-free stdlib WebSocket client
  (socket + handshake + one masked frame) used to drive the WS
  echo through the proxy. No `websockets`/`websocat` needed.

## Running

```bash
cargo build --bin ruscker
./scripts/e2e-interactive.sh
```

The script builds the image, starts `ruscker serve --docker`
against `wsapp.yml`, and asserts four things end-to-end:

1. **HTML transform** — `<base href>` + JS shim + absolute-attr
   rewriting present in the served page.
2. **Asset round-trip** — a rewritten `/app/wsapp/static/app.js`
   routes back through the proxy and serves the file.
3. **JSON endpoint** — `/app/wsapp/api/ping` returns.
4. **WebSocket proxy** — bidirectional echo works against a
   genuine upstream WebSocket server.

Exit code 0 = all four passed.

## What it does NOT cover

The JS shim's **runtime** behavior (monkey-patching
`WebSocket` / `fetch` / `XMLHttpRequest` inside a browser) is
not exercised — that needs a headless browser. This harness
proves the server-side mechanisms and the WS proxy path; the
shim is covered by unit tests in `ruscker-admin`'s
`routes::rewrite` module and by visual inspection of the
injected script.
