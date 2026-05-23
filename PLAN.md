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
- [ ] **i18n scaffolding** (ver §5): `fluent-templates`, arquivos
      `.ftl` em `assets/i18n/{en,pt,es,fr}/`, seletor de idioma na UI
      (cookie + `Accept-Language`), strings da landing já externalizadas.
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
- Tradução completa das 4 línguas. A Fase 1 entrega **só PT-BR
  100%** e o *andaime* para EN/ES/FR (chaves existem, fallback para
  PT enquanto não há tradução).

---

## 5. Internacionalização (i18n) do painel

**Decisão:** painel admin e landing públicos suportam
**pt-BR · en · es · fr**, com seleção por usuário. O idioma escolhido
sobrescreve o do navegador.

### 5.1. Stack

- **`fluent-templates` + `fluent-bundle`** (Mozilla Fluent) — suporta
  bem pluralização e variantes; sintaxe `.ftl` legível por tradutor
  não-técnico.
- Integração com Askama via filter custom: `{{ "landing.title"|t(loc) }}`.
- Locale ativa flui pelo request via `tower` extension layer.

### 5.2. Ordem de precedência da seleção de idioma

1. Cookie `ruscker_locale=<code>` (set pelo seletor da UI).
2. Header `Accept-Language` (negociação padrão HTTP).
3. Fallback **pt-BR** (default — público primário é o SEPE).

Idiomas suportados: `pt-BR`, `en-US`, `es-ES`, `fr-FR`. Códigos
expostos sempre como ISO (`pt`, `en`, `es`, `fr`) na UI.

### 5.3. Layout dos arquivos de tradução

```
crates/ruscker-admin/assets/i18n/
├── pt/
│   ├── landing.ftl
│   ├── admin.ftl
│   └── errors.ftl
├── en/  (mesma estrutura)
├── es/  (mesma estrutura)
└── fr/  (mesma estrutura)
```

Embutidos no binário via `include_dir!` (zero arquivos no disco em
produção). Tradução faltante → fallback para `pt` e log em `tracing`
(uma vez por chave, não por request).

### 5.4. Convenções de chaves

- Hierárquicas com ponto: `landing.title`, `admin.specs.add-button`.
- Sem HTML embutido. Quando precisa de formatação, usar variáveis
  Fluent: `{ $count } aplicações`.
- Pluralização sempre via `{ $n ->`, mesmo para EN (consistência).

### 5.5. Onde aparece nas fases

| Fase | i18n entregue |
|---|---|
| **1** | Scaffolding completo + landing 100% em PT, chaves espelhadas para EN/ES/FR (texto = inglês placeholder ou idêntico ao PT). Seletor de idioma já visível. |
| **2** | Strings do painel admin externalizadas. Tradução EN completa. |
| **3** | Mensagens de erro do proxy externalizadas (visíveis em telas de "container starting", "503 saturated"). |
| **4** | Dashboard externalizado. Tradução ES completa. |
| **5** | Tradução FR completa. Auditoria final de strings hard-coded via lint custom. |

### 5.6. Tooling

- Script `scripts/i18n-check.sh` que compara chaves entre as 4
  línguas e falha o CI se a chave existe em PT mas não em EN.
- README de tradutor em `crates/ruscker-admin/assets/i18n/README.md`
  (escrito na Fase 1) com instruções para receber PRs de não-devs.

---

## 6. Distribuição e instalação

**Princípio:** instalar o Ruscker tem que ser **uma linha**. Quanto
mais fricção, menor a adoção. Alvo primário: Ubuntu 22.04/24.04
(é o que o SEPE roda); alvos secundários: outras distros Linux
amd64/arm64, macOS para dev.

### 6.1. Matriz de artefatos por release

| Artefato | Plataforma | Ferramenta de build | Prioridade |
|---|---|---|---|
| `.deb` amd64 | Ubuntu 22.04+, Debian 12+ | `cargo-deb` | **P0** (alvo primário) |
| `.deb` arm64 | mesmas, em ARM | `cargo-deb` + cross | P1 |
| Tarball estático amd64 | Qualquer Linux glibc 2.31+ | `cross` ou `cargo-zigbuild` (musl) | **P0** |
| Tarball estático arm64 | mesma | `cargo-zigbuild` | P1 |
| Imagem Docker multi-arch | qualquer host com Docker | `docker buildx` (amd64 + arm64) | **P0** |
| Homebrew formula (`brew install ruscker`) | macOS (dev) | tap próprio em `StrategicProjects/homebrew-tap` | P1 |
| `.rpm` | RHEL/Rocky/Fedora/openSUSE | `cargo-generate-rpm` | P2 (deferred) |
| Pacote Arch (AUR) | Arch/Manjaro | PKGBUILD manual | P3 (community) |
| Snap / Flatpak | desktop Linux | snapcraft | **fora de escopo** |
| Instalador Windows | Windows Server | nada — sem suporte oficial | **fora de escopo** |

### 6.2. Conteúdo do `.deb`

- `/usr/bin/ruscker` — binário principal
- `/etc/ruscker/application.yml` — config de exemplo
- `/var/lib/ruscker/` — diretório do SQLite (criado vazio)
- `/var/log/ruscker/` — logs
- `/lib/systemd/system/ruscker.service` — unit file
- `postinst`: cria usuário `ruscker`, faz `chown` dos diretórios,
  registra o serviço (sem habilitar — operador decide).
- `prerm`/`postrm`: para o serviço, opcionalmente preserva dados.

### 6.3. Linha de instalação por plataforma

```bash
# Ubuntu/Debian (alvo primário)
curl -fsSL https://ruscker.app/install.sh | sudo bash
# ou direto:
curl -L https://github.com/StrategicProjects/ruscker/releases/latest/download/ruscker_amd64.deb \
  -o /tmp/r.deb && sudo dpkg -i /tmp/r.deb

# Qualquer Linux
curl -L https://github.com/StrategicProjects/ruscker/releases/latest/download/ruscker-linux-amd64.tar.gz \
  | sudo tar -xz -C /usr/local/bin/

# Docker
docker run -d --name ruscker \
  -p 8080:8080 \
  -v /var/run/docker.sock:/var/run/docker.sock \
  -v /etc/ruscker:/etc/ruscker \
  ghcr.io/strategicprojects/ruscker:latest

# macOS (dev)
brew install StrategicProjects/tap/ruscker
```

### 6.4. CI/CD para releases

GitHub Actions workflow `.github/workflows/release.yml` que, em tag
`v*.*.*`:

1. Build matrix: linux-amd64 + linux-arm64 + macos-amd64 + macos-arm64.
2. `cargo-deb` para os dois Linux.
3. `docker buildx` para imagem multi-arch publicada em GHCR.
4. `gh release create` com todos os artefatos anexados + checksums
   SHA256 + assinatura `cosign` (P1).
5. Atualizar tap Homebrew automaticamente (P1).

Decisões pendentes:

- **APT repository próprio** (`apt.ruscker.app`) — facilita
  `apt update` ao invés de baixar `.deb` toda vez. Custo: 1 dia de
  setup + manutenção contínua. Decisão: **fazer só após primeiros 3
  releases** quando o fluxo estiver estável.
- **Assinatura `cosign`** dos binários — boa prática mas opcional
  para MVP. Decisão: incluir desde o release v0.1.0 (custo baixo).

### 6.5. Onde aparece nas fases

| Fase | Distribuição entregue |
|---|---|
| **1–4** | Apenas `cargo build` local. Nenhum artefato publicado. |
| **5** | Workflow de release completo, `.deb` amd64, tarballs amd64/arm64, imagem Docker multi-arch, Homebrew tap. Documentação de instalação no README. v0.1.0 publicado. |
| **pós-5** | arm64 `.deb`, APT repo, assinatura, RPM (se houver demanda). |

---

## 7. Tracking

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

## 8. Riscos conhecidos e mitigações

| Risco | Impacto | Mitigação |
|---|---|---|
| WebSocket hijacking + path rewriting do Shiny ser frágil | Fase 3 não entrega | Spike de 2 dias **antes** da Fase 3 testando hyper+tungstenite contra um Shiny real. |
| Tailwind 4 standalone CLI ter quebras no macOS arm64 | Build local não roda | Documentar versão exata em `scripts/`, fallback para Tailwind 3 se necessário. |
| `serde_yaml` deprecated quebrar em update transitivo | Parse falha | Pin estrito no workspace; migrar para `serde_yml` (fork ativo) se rolar incidente. |
| Migração ShinyProxy → Ruscker dar diff inesperado em campos raros | Operador descobre tarde | `ruscker validate --strict-compat` na Fase 5 lista incompatibilidades antes de subir em prod. |
| Mock visual ≠ implementação real (Jost, ícones Tabler, gradientes) | Retrabalho | Self-hostar Jost na Fase 1; revisar lado-a-lado mockup vs renderizado **antes** de fechar a Fase 1. |
| Strings hard-coded começam a aparecer depois da Fase 1 | Re-traduzir custa caro | Lint custom em CI (`scripts/i18n-check.sh`) rejeita strings literais em templates a partir da Fase 2. |
| `cargo-deb` ter quirks com `include_dir!` (assets grandes embutidos) | `.deb` quebrado na Fase 5 | Spike de 1 dia na **Fase 4** já gerando um `.deb` de teste com os assets embutidos. |
| Cross-compile glibc vs musl quebrar runtime do Docker socket | Tarball Linux não conecta no Docker | Testar imagem `bookworm-slim` e `alpine` no CI; documentar mínimo glibc. |

---

## 9. Como continuar

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
