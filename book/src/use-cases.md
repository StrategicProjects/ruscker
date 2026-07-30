# What Ruscker can serve

Ruscker started as a ShinyProxy alternative, but the model underneath is
more general: **one container per session** for stateful apps, **one
container per replica** for stateless APIs. Anything that runs in a
Docker container and speaks HTTP or WebSocket is a candidate — which
makes Ruscker a *portal runtime for containerized web apps*, not just a
Shiny host.

Each app becomes a card on the landing page and a route under
`/app/{spec}` (interactive) or `/api/{spec}` (stateless). Ruscker handles
spawning, sticky sessions, WebSocket upgrades, URL rewriting, load
balancing, and reaping idle containers. Sensitive apps can require a
recent MFA proof before any container starts; signed-in identity can be
forwarded to trusted apps through opt-in headers.

## Showcase demos

A fresh Ruscker install seeds 13 demo cards automatically. Three of them
use own-fork images published on Docker Hub; the rest link to official
docs or use well-known public images:

| Card | Image | Notes |
|---|---|---|
| Shiny | `openanalytics/shinyproxy-demo:latest` | R Shiny demo app, port 3838 |
| Shiny for Python | `openanalytics/shinyproxy-shiny-for-python-demo:latest` | Python, port 8080 |
| Jupyter | `quay.io/jupyter/minimal-notebook:latest` | token-less, `base_url=/` |
| RStudio Server | `rocker/rstudio:latest` | per-session IDE, port 8787 |
| R Markdown | `openanalytics/shinyproxy-rmarkdown-demo:latest` | Shiny backend, port 3838 |
| Streamlit | `openanalytics/shinyproxy-streamlit-demo:latest` | port 8501 |
| **Dash** | **`milkway/ruscker-dash-demo:latest`** | our fork — serves at root, no env quirks; multi-arch |
| **Quarto** | **`milkway/ruscker-quarto-demo:latest`** | our fork — pre-rendered static HTML on nginx (~67 MB vs ~430 MB) |
| **FastAPI** | **`milkway/ruscker-fastapi-demo:latest`** | our fork — stateless API kind; multi-arch |
| Voilà | `openanalytics/shinyproxy-voila-demo:latest` | Jupyter notebooks as apps |
| Bokeh | *(external link)* | docs card, no container |
| Plumber | *(external link)* | docs card, no container |
| Ruscker | *(external link)* | docs card, no container |

The three own-fork images (`milkway/ruscker-dash-demo`, `milkway/ruscker-fastapi-demo`,
`milkway/ruscker-quarto-demo`) are the reference for "how to make a Ruscker-ready
container": they serve at root, ship no `SHINYPROXY_PUBLIC_PATH` dependency,
and the Dash/FastAPI forks are multi-arch (amd64 + arm64).

Seeding is idempotent — it only runs once per database. If you delete a
showcase card, it stays gone on subsequent restarts.

## Interactive, stateful apps

There are two interactive spec kinds in Ruscker:

- **`shiny`** — the Shiny model: state on the server, a reactive WebSocket
  connection per session. Sticky sessions and WebSocket forwarding are
  on by default. Covers R Shiny and Shiny for Python.
- **`app`** (interactive app) — same sticky-session + WebSocket behavior,
  but for apps that aren't Shiny specifically: Streamlit, Dash, Voilà,
  JupyterLab, RStudio Server, and similar. Use `type: app` in your spec
  (or Ruscker infers it from well-known `type:` values like `streamlit`,
  `dash`, `voila`).

Both kinds give each session its own container slot and forward WebSocket
upgrades transparently.

Supported frameworks:

- **R** — Shiny (the reference case), Quarto Live, `flexdashboard` with
  `runtime: shiny`.
- **Python** — Streamlit, Dash (Plotly), Gradio, Panel (HoloViz),
  Solara, Mesop, Reflex, Bokeh server.
- **Notebooks as apps** — Voilà, Marimo, Observable Framework.
- **Julia** — Pluto.jl, Genie + Stipple, Dash.jl.

## Stateless HTTP APIs

Any request can go to any replica — the simplest case, load-balanced
round-robin with no sticky cookie.

- **R** — Plumber and Plumber 2, Ambiorix, RestRserve.
- **Python** — FastAPI, Flask / Quart, Litestar, Django REST, Sanic.
- **Other languages** — Go (Gin, Echo, Chi), Node (Express, Fastify,
  Hono, Nest), Rust (Axum, Actix), Ruby (Rails API, Sinatra), Elixir
  (Phoenix API), PHP (Laravel, FrankenPHP).

This is something ShinyProxy doesn't do naturally — Ruscker treats APIs
as a first-class spec kind with their own scaling and rate limiting.

## ML / LLM model serving

Models exposed over HTTP fit the stateless-API path; GPU workloads
benefit from per-spec replica limits.

- **Serving runtimes** — BentoML, MLflow Models, Seldon, TorchServe,
  TensorFlow Serving (HTTP), NVIDIA Triton (HTTP).
- **LLMs** — Ollama, vLLM, Text Generation Inference, LiteLLM proxy.

## Per-user notebooks and IDEs

Each user gets an isolated container — the JupyterHub pattern, with
Ruscker's portal and admin on top.

- JupyterLab / Jupyter Notebook (included in the showcase seed),
  RStudio Server (included in the showcase seed), `code-server` (VS Code
  in the browser), Theia, Marimo Lab.

## BI and data exploration

Isolate a dashboard tool per team or per tenant.

- Apache Superset, Metabase, Redash, Apache Zeppelin, Datasette,
  Evidence, Rill, Grafana (when you want per-tenant isolation).

## Generative-AI UIs

Multiplex GPUs and isolate users in front of generative tools.

- Stable Diffusion WebUI (AUTOMATIC1111, ComfyUI, Forge), Open WebUI,
  LibreChat, AnythingLLM, Flowise, Langflow, self-hosted Gradio demos.

## Data tooling and ETL UIs

- Apache Airflow, Dagster, Prefect, Mage, Kestra, NocoDB, Baserow,
  Directus, self-hosted Supabase Studio.

## Scheduled ETL, reports and maintenance

The admin **Schedules** page (local Docker backend) can run a containerized
spec to completion on a five-field cron: nightly ETL, report generation,
cache refreshes or small maintenance tasks. Set `server.timezone` to an IANA
name for the clock the expressions use; absent or invalid values retain the
UTC default. The Schedules page labels next and last executions in that same
zone. A job reuses the spec's image, environment, volumes, resource limits
and registry credentials, with an optional command override. Runs have a
configurable timeout (1 hour by default), history and log tails; failures can
trigger the `job-failed` alert webhook.

## Database admin consoles

Surface a DB console as just another card on the portal.

- pgAdmin, phpMyAdmin, Adminer, Mongo Express, Redis Insight,
  CloudBeaver.

## Works, with caveats

- **WebRTC apps** (Jitsi, BigBlueButton) — Ruscker proxies the HTTP/WS
  signalling and the frontend, but UDP media needs a separate relay
  (e.g. coturn).
- **gRPC** — runs over HTTP/2; unary APIs work, bidirectional streaming
  with per-session routing needs extra configuration and testing.

## Not the right tool

- **Purely static sites** (Hugo, Astro output) — overkill; use nginx or
  Caddy directly.
- **Service-mesh-grade microservices** (automatic mTLS, dense distributed
  tracing, complex canaries) — that's Istio/Linkerd on Kubernetes.
  Ruscker is a portal proxy, not a service mesh.
- **Large distributed or unbounded batch pipelines** — use Slurm, Nomad,
  Airflow workers or another dedicated scheduler. Ruscker's cron runner is
  for bounded run-to-completion jobs alongside the portal workloads.

## Who it's for

- **Universities, research centers and other organizations** publishing
  analytics dashboards for broad or internal audiences.
- **BI and data-science teams** that want to ship R/Shiny and
  Python/Streamlit apps without standing up a Kubernetes cluster.
- **Consultancies** delivering analytical tools to each client at isolated
  URLs.
- **AI teams** self-hosting LLM and generative WebUIs with per-user
  isolation.

The breadth here is the point: what looks like a "ShinyProxy alternative"
is, in practice, a portal runtime for **any** containerized web app.
