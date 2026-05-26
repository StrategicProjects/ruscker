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
balancing, and reaping idle containers.

## Interactive, stateful apps

State lives on the server and the client holds a reactive WebSocket —
the Shiny model. These need **sticky sessions + WebSocket forwarding**,
which Ruscker does by default.

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

- JupyterLab / Jupyter Notebook, RStudio Server, `code-server` (VS Code
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
- **Long CPU-bound batch jobs** — use a scheduler (Slurm, Nomad, Airflow
  workers). Ruscker is for interactive sessions and short requests.

## Who it's for

- **Governments, universities and research centers** publishing analytics
  dashboards for the public or for staff.
- **BI and data-science teams** that want to ship R/Shiny and
  Python/Streamlit apps without standing up a Kubernetes cluster.
- **Consultancies** delivering analytical tools to each client at isolated
  URLs.
- **AI teams** self-hosting LLM and generative WebUIs with per-user
  isolation.

The breadth here is the point: what looks like a "ShinyProxy alternative"
is, in practice, a portal runtime for **any** containerized web app.
