# Ruscker — Plano de execução

Este documento condensa **onde estamos**, **para onde vamos** e **o que
fazer primeiro**. Complementa o `docs/ROADMAP.md` (visão geral) com
foco em decisões de execução e o sprint imediato.

> Visão completa por fase: [`docs/ROADMAP.md`](docs/ROADMAP.md)
> Arquitetura: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)
> Schema YAML: [`docs/YAML_SCHEMA.md`](docs/YAML_SCHEMA.md)
> ADRs: [`docs/adr/`](docs/adr/)
> Mockups visuais: [`docs/mockups/index.html`](docs/mockups/index.html)

---

## 1. Estado atual (Fase 0 — concluída)

| Crate | Status | Observação |
|---|---|---|
| `ruscker-config` | ✅ funcional | Schema completo, interpolação de env, validação em duas fases. **24 testes verdes** (15 unit + 9 integração contra `examples/application.yml`, 31 specs). |
| `ruscker-core` | ✅ funcional | Tipos `Replica`, `Session`, traits `Router` (least-conn + round-robin testados). Pure-domain, sem I/O. |
| `ruscker-cli` | ✅ funcional | Subcomandos `validate`, `show`, `inspect` (texto + JSON, modo `--strict`). |
| `ruscker-docker` | 🚧 stub | `DockerBackend` implementa o trait mas todo método retorna `CoreError::Backend("not yet implemented")`. |
| `ruscker-proxy` | 🚧 stub | `ProxyServer::new()` ok, `run()` falha. |
| `ruscker-admin` | 🚧 stub | `AdminServer::new()` ok, `run()` falha. |

**Demonstração funcional hoje:**

```bash
DOCKER_REGISTRY_PASSWORD=test \
  cargo run --bin ruscker -- validate examples/application.yml
# → relatório legível com 31 specs, breakdown shiny/external, warnings
```

### Decisões já tomadas (ADRs)

- **0001** — Rust como linguagem (binário ~20 MB, sem JVM, baixo footprint).
- **0002** — SQLite como fonte de verdade em runtime; YAML é só
  import/export.
- **0003** — Sticky sessions por cookie HMAC-SHA256 para Shiny/Streamlit/
  Dash/Voilà; round-robin para APIs.
- **0004** — Stack de UI: **Askama** (templates compilados) + **HTMX** +
  **Alpine** + **Tailwind 4** standalone (sem Node).

### O que ainda **não** existe

- Nenhum container roda. Nenhum tráfego HTTP é proxado.
- Não há banco SQLite, nem migrations, nem painel admin web.
- A landing-page existe **só como mockup HTML**, não em Askama.

---

## 2. Linha de chegada (MVP "substitui o ShinyProxy")

Definição de pronto para colocar em produção no lugar do ShinyProxy do
SEPE:

1. Operador roda `ruscker import application.yml --db /var/ruscker/ruscker.db`.
2. Para o ShinyProxy, sobe o Ruscker na mesma porta.
3. Visitante abre a portal, vê os cards renderizados, clica e a sessão
   Shiny carrega — reactividade funciona, sessão sobrevive a refresh.
4. Operador edita app via web UI, não toca em YAML.
5. Dashboard mostra containers ativos, sessões, CPU/mem em tempo real.

Isso corresponde ao fim da **Fase 5** do ROADMAP. Estimativa: **8–10
semanas** de trabalho focado, distribuídas em 5 fases (descritas
abaixo).

---

## 3. Mapa de fases (visão executiva)

| Fase | Entregável | Duração | Risco |
|---|---|---|---|
| **1 — Landing page** | Portal renderizado a partir do YAML, em Askama + Tailwind, visualmente idêntico ao SEPE atual. Cards ainda não clicáveis. | 1 semana | Baixo |
| **2 — Persistência + Admin CRUD** | SQLite + importer/exporter + painel para CRUD de specs, credenciais e biblioteca de imagens. Sem YAML para o operador. | 3 semanas | Médio |
| **3 — Proxy + Docker backend** | Containers spawnam, sessões Shiny funcionam end-to-end (HTTP + WebSocket + sticky). Fase tecnicamente mais difícil. | 3 semanas | **Alto** |
| **4 — Dashboard de monitoramento** | Métricas live (SSE), gráficos de sessões/CPU/mem, logs com filtro/follow, endpoint Prometheus. | 2 semanas | Médio |
| **5 — Produção** | Graceful shutdown, rate-limit, CORS, healthz/readyz, Dockerfile multi-stage, systemd unit, guia de migração ShinyProxy → Ruscker. | 1 semana | Baixo |

Fases 6+ (multi-host, HA Postgres, OIDC/SAML) ficam para **depois** do
MVP em produção.

---

## 4. Próximo sprint — Fase 1 detalhada

A Fase 1 é deliberadamente curta para entregar um marco visível
rapidamente: **`localhost:8080` mostra a portal SEPE em Askama**.
Nenhum proxy ainda — só leitura do YAML e renderização.

### 4.1. Decisão de arquitetura aberta

ROADMAP diz: *"Add a new `ruscker-web` crate (or fold into `ruscker-admin`)"*.

**Recomendação:** dobrar dentro de `ruscker-admin` por enquanto.
Justificativa:

- A landing e o admin compartilham o mesmo runtime axum, o mesmo
  motor Askama, o mesmo build de Tailwind, a mesma autenticação
  futura.
- Separar em dois crates agora cria duplicação sem benefício real.
- Se a landing crescer para algo independente (multi-tenant, white-
  label), dividimos. **Decisão revisitada no fim da Fase 2.**

### 4.2. Estrutura proposta de `ruscker-admin` após Fase 1

```
crates/ruscker-admin/
├── Cargo.toml
├── build.rs                 # roda Tailwind CLI standalone
├── src/
│   ├── lib.rs
│   ├── server.rs            # AdminServer, bind axum
│   ├── routes/
│   │   ├── mod.rs
│   │   ├── landing.rs       # GET / → render Landing
│   │   └── assets.rs        # GET /assets/* → static
│   ├── templates/
│   │   ├── mod.rs           # struct Landing<'a>, struct CardCtx<'a>...
│   │   ├── landing.html
│   │   ├── _card.html
│   │   ├── _filters.html
│   │   └── _layout.html
│   ├── theme.rs             # light/dark cookie + system pref
│   └── view_model.rs        # Spec → CardCtx (cor do tipo, ícone...)
├── assets/
│   ├── tailwind.css         # entry point
│   ├── tailwind.config.js
│   └── images/              # logos default (pernambuco, ufpe, sepe)
└── static/                  # OUTPUT do build (gitignored em src,
                             # incluído no binário via include_dir!)
```

### 4.3. Checklist Fase 1 (vira issue no GitHub)

- [ ] Crate `ruscker-admin` com axum 0.7 + askama 0.12 ligados.
- [ ] Tailwind 4 standalone via `build.rs` (sem Node toolchain).
- [ ] Layout base `_layout.html` (header, footer, theme toggle).
- [ ] Template `landing.html` espelhando
      `docs/mockups/landing-filters-cards-refined.html`.
- [ ] Componente `_card.html` com:
  - cores de tipo (`app` verde, `package` âmbar, `api` ciano…)
  - ícone de acesso (`lock` / `lock_open`)
  - estado `active` / `inactive` (card desabilitado)
- [ ] Componente `_filters.html` com chips contáveis (quantos specs
      por tipo) e seletor de acesso.
- [ ] Filtro/busca client-side via Alpine (sem rota servidor).
- [ ] Toggle de tema dark/light: cookie + `prefers-color-scheme`.
- [ ] Servir `/assets/*` (fonts Jost self-hosted, ícones Tabler,
      imagens da biblioteca).
- [ ] Rota `GET /` que carrega `application.yml` via
      `ruscker-config::load`, monta `view_model::CardCtx` por spec,
      renderiza.
- [ ] Subcomando novo: `ruscker serve --config <path>` (substitui o
      bail no `run()`).
- [ ] **Teste de snapshot** (`insta`) do HTML renderizado para os 31
      specs do `examples/application.yml`.
- [ ] Smoke test manual: `cargo run --bin ruscker -- serve` →
      browser em `http://localhost:8080` → comparar com mockup.

**Critério de aceite:** abrir `http://localhost:8080` e ver a portal
SEPE igual ao mockup `landing-filters-cards-refined.html`, populada
com os 31 specs reais. Cards ainda navegam para `#` (ou para
`template-properties.link` se for `external`).

### 4.4. O que **não** entra na Fase 1

- Nada de SQLite ainda. A landing lê direto do YAML.
- Nada de admin (`/admin`). Só a portal pública.
- Nada de proxy. Cards de container apenas mostram, não abrem sessão.
- Nada de login. `authentication: none` é o único modo suportado.

---

## 5. Tracking

- **GitHub Issues** — 1 issue principal por fase (1 a 5), com a
  checklist acima como sub-tasks. Labels: `phase:1`...`phase:5`,
  `area:admin`, `area:proxy`, etc.
- **GitHub Project** — board kanban `Ruscker Roadmap` na org
  StrategicProjects, ligado às issues.
- **Milestones** — um por fase, com data alvo.
- **CLAUDE.md** (raiz + por crate) ficam **só locais** (no
  `.gitignore`). Servem de memória persistente para o Claude Code
  durante o desenvolvimento e não são publicados.

---

## 6. Riscos conhecidos e mitigações

| Risco | Impacto | Mitigação |
|---|---|---|
| WebSocket hijacking + path rewriting do Shiny ser frágil | Fase 3 não entrega | Spike de 2 dias **antes** da Fase 3 testando hyper+tungstenite contra um Shiny real. |
| Tailwind 4 standalone CLI ter quebras no macOS arm64 | Build local não roda | Documentar versão exata em `scripts/`, fallback para Tailwind 3 se necessário. |
| `serde_yaml` deprecated quebrar em update transitivo | Parse falha | Pin estrito no workspace; migrar para `serde_yml` (fork ativo) se rolar incidente. |
| Migração ShinyProxy → Ruscker dar diff inesperado em campos raros | Operador descobre tarde | `ruscker validate --strict-compat` na Fase 5 lista incompatibilidades antes de subir em prod. |
| Mock visual ≠ implementação real (Jost, ícones Tabler, gradientes) | Retrabalho | Self-hostar Jost na Fase 1; revisar lado-a-lado mockup vs renderizado **antes** de fechar a Fase 1. |

---

## 7. Como continuar

```bash
# Setup
git clone git@github.com:StrategicProjects/ruscker.git
cd ruscker
./scripts/pin-deps.sh         # fix edition2024 nas transitivas
cargo build && cargo test     # baseline: 24 testes verdes

# Próximo PR (Fase 1)
git checkout -b phase-1/landing-page
# implementar checklist da seção 4.3
```

Cada PR deve:
- Referenciar a issue da fase (`refs #N`).
- Ter pelo menos 1 teste (snapshot, unit ou integração).
- Passar `cargo fmt --check && cargo clippy --all-targets`.
- Ser pequeno o suficiente para revisar em <30 min.
