# Ruscker — plano de execução

Este documento registra o estado operacional do projeto, as decisões que
orientam o trabalho e as próximas frentes. O histórico detalhado das fases
fica em [`docs/ROADMAP.md`](docs/ROADMAP.md); opções estratégicas de médio
prazo ficam em [`docs/NEXT_STEPS.md`](docs/NEXT_STEPS.md).

Referências principais:

- [Arquitetura](docs/ARCHITECTURE.md)
- [Schema YAML](docs/YAML_SCHEMA.md)
- [Segurança](docs/SECURITY.md)
- [Documentação publicada](https://strategicprojects.github.io/ruscker/)
- [ADRs](docs/adr/)

---

## 1. Estado atual

**Fases 0–7 entregues. Ruscker está em produção e substitui uma instalação
real do ShinyProxy.** A release estável atual é a **v0.2.40**; `main` pode
conter correções já validadas que ainda não receberam nova tag.

O deploy de referência carrega uma configuração de 31 apps, inicia containers
sob demanda e mantém footprint ocioso de aproximadamente **14 MB**, contra
aproximadamente 540 MB do proxy JVM substituído.

| Crate | Estado | Responsabilidade atual |
|---|---|---|
| `ruscker-config` | ✅ produção | Schema compatível com ShinyProxy, interpolação de ambiente, defaults e validação normal/strict-compat. |
| `ruscker-core` | ✅ produção | Tipos e traits de domínio, catálogo de réplicas, decisões de roteamento e contratos de backend. |
| `ruscker-docker` | ✅ produção | Backend Docker local e multi-host, pull autenticado, limites, placement, métricas, logs e lifecycle. |
| `ruscker-proxy` | ✅ produção | Sticky sessions assinadas, handshake e pump WebSocket bidirecional com backpressure/diagnóstico. |
| `ruscker-admin` | ✅ produção | Landing, autenticação local/RBAC, CRUD, mídia, credenciais, dashboard, proxy HTTP e políticas operacionais. |
| `ruscker-cli` | ✅ produção | `serve`, import/export, validate/show/inspect e integração com os backends configurados. |

Também estão entregues:

- catálogo DB-first em SQLite ou Postgres, com YAML como formato de
  migração/importação/exportação;
- landing e Admin localizados em pt-BR, en-US, es-ES e fr-FR;
- apps interativos e APIs, autoscaling, métricas, logs e graceful shutdown;
- montagem em subpath, HA ativo-ativo e scheduling multi-host;
- `.deb`, tarballs musl, imagem multi-arch, Homebrew e artefatos assinados;
- suíte padrão e feature-gated com mais de 600 testes.

### Marcos por fase

| Fase | Resultado | Estado |
|---|---|---|
| 0 | Workspace, schema, validação e CLI | ✅ entregue |
| 1 | Landing Askama/Tailwind + i18n | ✅ entregue |
| 2 | Persistência e Admin CRUD | ✅ entregue |
| 3 | Proxy HTTP/WS + Docker + lifecycle | ✅ entregue |
| 4 | Dashboard, métricas e logs | ✅ entregue |
| 5 | Segurança, operação e distribuição | ✅ entregue |
| 6 | Visibilidade por usuário/grupo, multi-host e base-path | ✅ entregue |
| 7 | HA ativo-ativo com Postgres e leader lock | ✅ entregue |
| 8 | Provedores externos de identidade | 🚧 opcional / ainda não entregue |

---

## 2. O que ainda não existe

Estas lacunas são deliberadas ou continuam abertas; não devem ser confundidas
com os antigos stubs das fases iniciais:

- **SSO corporativo:** OIDC/SAML/LDAP ainda não existem. Hoje a autenticação
  usa contas locais, RBAC e token break-glass. Ver [#934](https://github.com/StrategicProjects/ruscker/issues/934).
- **Isolamento por origem:** `/app/*` e `/admin/*` ainda podem compartilhar a
  mesma origem. Apps não confiáveis precisam de uma topologia externa
  cuidadosa até existir enforcement nativo. Ver [#878](https://github.com/StrategicProjects/ruscker/issues/878).
- **Backend Kubernetes:** fora do escopo atual; Docker local/multi-host é o
  backend suportado.
- **Webhooks de alerta:** thresholds e Prometheus existem, mas não há entrega
  de notificações por webhook. Ver [#930](https://github.com/StrategicProjects/ruscker/issues/930).
- **E2E contínuo com frameworks reais:** há integração WS e validação de
  produção, mas o CI padrão ainda não sobe Shiny/Streamlit reais. Ver
  [#929](https://github.com/StrategicProjects/ruscker/issues/929).
- **Multi-tenancy comercial:** organizações isoladas, billing e marketplace
  público não fazem parte do produto open-source atual.

---

## 3. Prioridades abertas

O projeto já passou do ponto de “completar o MVP”. Novas mudanças devem
resolver risco operacional, adoção ou dívida claramente demonstrada.

### P0 — segurança e confiabilidade

1. [#878 — separar control plane e app plane por origem](https://github.com/StrategicProjects/ruscker/issues/878).
2. [#944 — agregar contadores de acesso de APIs](https://github.com/StrategicProjects/ruscker/issues/944), eliminando uma task e um write por request.
3. [#929 — E2E real de Shiny/Streamlit](https://github.com/StrategicProjects/ruscker/issues/929).

### P1 — adoção e operação

1. [#934 — fatiar o MVP OIDC/SSO](https://github.com/StrategicProjects/ruscker/issues/934).
2. [#940 — matriz ShinyProxy → Ruscker](https://github.com/StrategicProjects/ruscker/issues/940).
3. [#925 — edição consolidada de usuários](https://github.com/StrategicProjects/ruscker/issues/925).
4. [#930 — webhooks de alerta](https://github.com/StrategicProjects/ruscker/issues/930).

### Decisões que exigem design antes de código

- [#926 — separar configuração do serviço, catálogo DB-first e formato de
  migração ShinyProxy](https://github.com/StrategicProjects/ruscker/issues/926).
- [#389 — estratégia e ownership dos containers demo](https://github.com/StrategicProjects/ruscker/issues/389).

Não implementar épicos grandes diretamente. Primeiro registrar a decisão e
abrir slices independentes com critérios de aceite verificáveis.

---

## 4. Decisões firmes

- **YAML compatível:** nomes existentes do ShinyProxy não são renomeados.
- **DB-first em runtime:** YAML é entrada/saída de migração; o Admin e o banco
  são a fonte operacional quando `--db` está ativo.
- **Sem segredos em YAML ou DB em claro:** usar `${ENV_VAR}` e credenciais
  criptografadas.
- **Zero SPA:** Askama + HTMX + Alpine; não introduzir framework frontend sem
  uma decisão arquitetural explícita.
- **Domínio puro:** `ruscker-config` e `ruscker-core` não recebem I/O assíncrono.
- **Frames WS stateful não são descartados:** backpressure persistente encerra
  a conexão inteira com diagnóstico estruturado.
- **Compatibilidade antes de conveniência:** uma feature ShinyProxy ignorada
  deve gerar warning/erro claro, nunca desaparecer silenciosamente.

As justificativas duráveis ficam nos ADRs, não neste arquivo.

---

## 5. Riscos ativos

| Risco | Backstop atual | Próximo passo |
|---|---|---|
| App comprometido alcança Admin same-origin | CSRF, RBAC e hardening de ações destrutivas | Separação de origem #878 |
| Carga de API amplifica writes de analytics | Write best-effort fora do hot path | Agregador supervisionado #944 |
| Regressão específica de framework | Testes de rewrite/WS + smokes reais | Shiny/Streamlit E2E #929 |
| HA com sessões Admin locais | Sticky upstream documentado | Avaliar store compartilhado só com necessidade real |
| Docker socket amplia blast radius | Operador controla host, validações e RBAC | Manter threat model e least privilege |
| Configuração DB/YAML confunde operadores | Documentação de import/export | Decisão de modelo #926 |

---

## 6. Como trabalhar no próximo slice

1. Confirmar que a issue continua válida contra `main` e que não existe PR
   concorrente.
2. Criar branch `codex/<descrição>` a partir de `origin/main` atualizado.
3. Alterar somente o menor conjunto de arquivos necessário.
4. Preservar compatibilidade YAML e dados existentes.
5. Adicionar regressão que falhe sem a mudança.
6. Executar os gates relevantes:

```bash
DOCKER_REGISTRY_PASSWORD=test cargo test --locked
DOCKER_REGISTRY_PASSWORD=test cargo clippy --locked --all-targets -- -D warnings
./scripts/i18n-check.sh
bash scripts/migration-parity-check.sh
git diff --check
```

Testes que abrem sockets localhost podem precisar rodar fora do sandbox.
Suites Docker/Postgres continuam feature-gated e exigem os serviços descritos
em seus módulos.

**Não executar `cargo fmt` no workspace inteiro.** `main` não é fmt-clean sob
o rustfmt atual; formatar manualmente apenas as linhas tocadas.

---

## 7. Política de versões e releases

- O toolchain de desenvolvimento/release é o fixado em
  `rust-toolchain.toml`; o MSRV é validado separadamente no CI.
- Antes de adicionar ou atualizar dependência, consultar a versão estável
  atual e avaliar bumps major em issue própria.
- Toda tag publica os artefatos suportados e passa pelos gates de release,
  incluindo assinatura e verificações de segurança.
- `book/src/news.md` é o changelog de releases; este arquivo não deve repetir
  notas de cada versão.

---

## 8. Critério de conclusão

Um slice só está concluído quando:

- o comportamento está implementado e documentado na fonte correta;
- testes locais e CI estão verdes;
- não há warnings de compatibilidade introduzidos sem explicação;
- o PR referencia/fecha a issue e pode ser revertido isoladamente;
- `main` volta a ficar sincronizada e pronta para o próximo slice.
