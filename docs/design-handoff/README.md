# Handoff: Ruscker — Portal Público + Admin

## Visão Geral

O Ruscker é uma plataforma de hospedagem de aplicações científicas (Shiny, Jupyter, Streamlit, FastAPI, etc.) desenvolvida na UFPE. Este pacote contém o protótipo UX completo com **alta fidelidade**, cobrindo:

- **Portal público** — catálogo de apps com busca, filtros, destaques e controle de acesso por grupo
- **Admin** — dashboard de monitoramento, gerenciamento de apps, usuários, grupos, disco, credenciais, logs, auditoria e configuração de aparência

## Sobre os arquivos de design

Os arquivos neste bundle são **protótipos de referência criados em HTML + React (Babel inline)**. Eles **não** são código de produção — são mockups funcionais de alta fidelidade que mostram layout, comportamento, interações e estados.

**Sua tarefa** é recriar estas telas no ambiente do projeto real (framework, design system e bibliotecas já existentes). Analise cada tela no HTML e implemente de acordo com os padrões do codebase alvo.

## Fidelidade

**Alta fidelidade (hifi)** — cores, tipografia, espaçamento, ícones e interações estão definidos com precisão. O desenvolvedor deve reproduzir as telas pixel-a-pixel usando as bibliotecas e padrões do projeto real.

---

## Design Tokens

### Cores principais
```
--teal-600:    #0f6e56   (accent primário — verde Ruscker)
--teal-400:    #06d6a0   (dot de status "ready")
--warn:        #f59e0b   (warning)
--lock:        #ef4444   (restrito/vermelho)
--lock-open:   #10b981   (público/verde)
--text:        #111827   (light) / #e6edf3 (dark)
--text-muted:  #6b7280
--text-faint:  #9ca3af
--bg:          #f4f5f0   (light) / #0d1117 (dark)
--surface:     #ffffff   (light) / #161b22 (dark)
--surface-soft:#f8faf8   (light) / #1c2128 (dark)
--border:      rgba(0,0,0,.1) (light) / rgba(255,255,255,.08) (dark)
```

### Cores de grupos de acesso
```
admin:  #ef476f  (rosa/vermelho)
editor: #06b6d4  (ciano)
viewer: #26547c  (azul escuro)
```

### Tipografia
- **Interface:** `'Geist'` (Google Fonts) — fallback: `system-ui, -apple-system, sans-serif`
- **Mono:** `'Geist Mono'` — fallback: `'JetBrains Mono', monospace`
- Tamanhos base: 13–14px para body, 11–12px para labels/meta, 30–36px para métricas

### Ícones
Tabler Icons — `https://unpkg.com/@tabler/icons-webfont@latest/dist/tabler-icons.min.css`
Classe padrão: `<i class="ti ti-{name}"></i>`

### Bordas e sombras
```
border-radius base:  8–10px
border-radius card:  14px
border-radius pill:  999px
shadow-sm: 0 1px 3px rgba(0,0,0,.06)
shadow-lg: 0 8px 30px rgba(0,0,0,.12)
```

---

## Telas / Views

### 1. Portal Público

**Propósito:** Catálogo de aplicações acessível ao público, com controle de visibilidade por grupo de acesso.

**Layout:** single-column, max-width ~1200px centrado.

#### Cabeçalho (`.lhead`)
- Logo (SVG ~28px) + nome do portal + tagline abaixo
- Fundo: gradiente configurável (suave/vibrante/nenhum) baseado no accent color
- Direita: 3 ícones — tema (sol/lua), idioma (PT/EN), entrar (login icon)
- Em mobile: collapse para hamburger

#### Seletor de usuário simulado (protótipo)
- Botão discreto no canto direito acima do catálogo
- Dropdown com lista de usuários para simular acesso por grupo
- Pontinhos coloridos de grupo ao lado do nome

#### Carrossel de destaques (`.feat`)
- Título "Em destaque / Featured" com `ti-star`
- Navegação: chevrons esquerdo/direito, 3 cards por página
- Cards idênticos aos do catálogo
- Só aparece quando há apps favoritados E visíveis ao usuário atual
- Controlado pelo toggle em Admin → Aparência

#### Barra de busca + filtros (`.portal-sticky`)
- Sticky abaixo do carrossel
- Input de busca com ícone de lupa + atalho ⌘K
- Botão de ordenação (Recente/Nome)
- Chips de filtro: Todos · Apps · APIs · Pacotes · Relatórios | público · restrito
- Linha de resultados com contagem e botão "Limpar"

#### Grade de cards (`.cards-grid`)
- CSS grid: `repeat(auto-fill, minmax(280px, 1fr))`
- Cards com: cover area (logo + badges de tipo/subject + cadeado), nome, descrição, data de atualização, status dot
- Estado de loading: skeleton cards animados

#### Filtro de acesso por grupo
- Apps com `accessGroups: []` → visíveis a todos (logados ou não)
- Apps com `accessGroups: ["editor","viewer"]` → só visíveis para usuários desses grupos
- A filtragem ocorre no `useMemo` do `filtered` array — deps incluem `portalUser` e `appAccess`

**3 variações de layout:**
- **A — Refinado:** carrossel horizontal + grid de cards
- **B — Compacto:** nav rail lateral + lista de linhas densas + carrossel no topo
- **C — Seções:** agrupado por tipo (Apps / APIs / Pacotes / Relatórios)

---

### 2. Dashboard de Monitoramento

**Propósito:** Visão operacional do servidor — réplicas rodando, uso de recursos, estado de cada app.

**Layout:** 2 colunas (métricas + lista de réplicas)

#### Métricas globais
- 4 KPI cards: CPU, RAM, Réplicas ativas, Uptime
- Números com animação de "live" (contagem suave)
- Mini sparkline (SVG path) em cada card

#### Lista de réplicas (por app)
- Apps agrupados com collapse/expand
- Por réplica: ID curto, estado (dot colorido), CPU %, RAM, uptime
- Estados: `ready` (verde), `boot` (âmbar), `warn` (laranja), `off` (cinza)
- Botão de restart por réplica

**3 variações:**
- **A — Agrupado:** accordion por app, réplicas em tabela
- **B — Cards:** um card por app com métricas e lista de réplicas colapsável
- **C — Ops:** foco em ações — lista densa com checkboxes para operações em lote

---

### 3. Admin — Apps

**Propósito:** Catálogo de specs de apps cadastrados no banco de dados.

**Layout:** página full-width com tabela sticky-header

#### Barra de ações
- Busca instantânea, chips de filtro por tipo (Shiny/Interativo/API/Externo), ordenação
- Botões: Importar YAML + Novo app

#### Tabela
Colunas: ID · Nome (logo+texto) · Tipo (pill colorida) · Acesso (público ou group-badges) · Estado (pill) · Atualizado · Ações

- **Coluna Acesso:** `[]` → ícone globo + "público"; `[grupos]` → group-badge pills
- **Ações:** estrela SVG (outline/preenchida âmbar), lápis (abre editor), duplicar
- Clicar na linha abre o editor completo

#### Estrela / Favoritar
- SVG personalizado: `<path d="M12 17.27L18.18 21...">` 
- `fill="currentColor"` quando favoritado (âmbar `#d97706` + fundo amarelo)
- `fill="none"` quando não favoritado (stroke apenas)
- Estado compartilhado globalmente — reflete no Portal (carrossel) e Aparência (preview)

---

### 4. Admin — Editor de App

**Propósito:** Criar ou editar spec de um app com preview ao vivo do card.

**Layout:** 2 colunas — formulário (esquerda) + preview fixo (direita)

#### Seções do formulário
1. **Identity** — ID, Nome
2. **Kind** — tipo (App/API/Pacote/Relatório), cobertura do card
3. **Description** — PT + EN
4. **Appearance** — cor accent, ícone mono
5. **Access & scale** — toggle Restrito, grupos de acesso (mutex: Público OU grupos), réplicas
6. **Advanced** — env vars, recursos (CPU/RAM), autoscale, health check, URL externa
7. **Live preview** — card ao vivo reagindo ao formulário

#### Seleção de grupos de acesso (seção Access & scale)
- Botão "Público" (verde teal) — quando ativo, desabilita os 3 botões de grupo (`disabled + opacity: 0.32`)
- Botões de grupo: Administrador / Editor / Visualizador — multi-seleção
- Regra: nunca Público + grupos ao mesmo tempo
- Ao salvar, persiste em estado global `appAccess: Map<appId, string[]>`

---

### 5. Admin — Importar YAML

**Propósito:** Importar specs de apps a partir de um arquivo YAML.

**Layout:** modal/page com textarea + preview de apps detectados

- Textarea para colar YAML
- Lista de apps detectados com checkbox por app (selecionar subconjunto)
- Botão "Importar selecionados"
- Validação inline (ID duplicado, campos obrigatórios)

---

### 6. Admin — Aparência

**Propósito:** Configurar a identidade visual do portal público com preview ao vivo.

**Layout:** 2 colunas — formulário (esquerda) + mini-preview do portal (direita, sticky)

#### Seções do formulário
1. **Identidade** — título, tagline
2. **Cor da marca** — swatches predefinidos + input hex
3. **Logo** — Marca+nome / Só símbolo / URL customizada + sliders de tamanho/margem
4. **Fundo & gradientes** — hero do cabeçalho (nenhum/suave/vibrante) + cover dos cards (tint/gradient)
5. **Tema padrão** — Claro / Escuro / Auto
6. **Layout do catálogo** — Grid / Lista / Seções + Confortável / Compacto
7. **Seções visíveis** — toggles: carrossel destaques, busca, filtros de acesso
8. **SEO** — meta title, meta description, og:image
9. **Analytics** — Nenhum / Google Analytics / Plausible / Matomo + ID + respeitar DNT
10. **CSS customizado** — code editor com syntax highlighting (CSS)
11. **Blocos HTML** — 4 slots injetáveis: abaixo do cabeçalho / após destaques / antes do rodapé / dentro do rodapé

#### Editor de código (CSS + HTML)
- Fundo sempre escuro `#0d1117` (GitHub/VS Code Dark)
- Overlay pattern: `<pre>` com highlight + `<textarea>` transparente por cima
- Paleta de tokens (VS Code Dark+):
  - Comentários: `#6A9955` itálico
  - Strings: `#CE9178`
  - At-rules: `#C586C0`
  - Seletores: `#D7BA7D`
  - Propriedades: `#9CDCFE`
  - Números/unidades: `#B5CEA8`
  - !important: `#F44747`
  - Tags HTML: `#4EC9B0`

#### Preview ao vivo (mini-portal)
- Escala fixa ~350px de largura
- Reflete em tempo real: hero gradient, cor accent, logo, tagline, carrossel de destaques, filtros
- Cabeçalho com: logo + nome + 3 ícones (tema, idioma, login)
- Estado global `portalCfg` compartilhado — mudanças na Aparência refletem instantaneamente no Portal

---

### 7. Admin — Mídia

**Propósito:** Biblioteca de imagens/SVGs disponíveis no sistema.

**Layout:** grid de tiles + zona de upload

- Grid `repeat(auto-fill, minmax(180px, 1fr))`
- Cada tile: preview da imagem + nome (mono) + tamanho + tipo MIME
- Hover: botão de remover (trash, vermelho)
- Zona de drag-and-drop no topo (suporta PNG/JPEG/WebP/SVG, até 10MB)
- Busca instantânea

---

### 8. Admin — Disco

**Propósito:** Uso de armazenamento do servidor com ações de limpeza.

**Layout:** hero de uso + 2 painéis side-by-side

#### Hero de uso
- Número grande: "X.X GB / 50 GB" + percentual
- Barra de progresso segmentada por categoria (imagens, containers, cache, logs, backups)
- Legenda com ícone + label + tamanho por categoria

#### Painel: Imagens de container
- Tabela: Imagem:tag · In use (badge verde/vermelho) · Tamanho · Ação (trash)
- Imagens com `usedBy === 0` ficam levemente opacas (`.is-unused`)
- Badge "dangling" para imagens sem tag
- Botão "Prune unused · X.X GB" — remove todas as não utilizadas

#### Painel: Containers
- Tabela: ID (8 chars hex) · App · Estado (running/stopped/exited + dot) · RAM · Criado · Ação
- Botão "Prune stopped · X MB" — remove parados/exited

---

### 9. Admin — Usuários

**Propósito:** Gerenciar usuários com acesso ao portal e ao admin.

**Layout:** tabela com inline editor por linha

#### Tabela
Colunas: Usuário (avatar+nome+email) · Grupos · Estado · Visto · Ações

- **Avatar:** círculo com iniciais colorido por hash do nome
- **Grupos:** um ou mais group-badge pills por usuário (cor por tipo)
- **Estado:** dot + label (ativo/convidado/suspenso)
- Linha suspensa: `opacity: 0.55`

#### Editor inline de grupos
- Clique em ✎ expande uma linha extra com toggles de grupo
- Cada toggle: checkbox-style (ativo = cor do grupo + check icon)
- Mínimo 1 grupo por usuário (último não pode ser removido)

#### Modelo de dados de usuário
```typescript
interface User {
  id: number;
  name: string;
  email: string;
  groups: ("admin" | "editor" | "viewer")[];  // array, multi-grupo
  status: "active" | "invited" | "suspended";
  seen: { pt: string; en: string };
}
```

---

### 10. Admin — Grupos

**Propósito:** Visualizar papéis de acesso, seus membros e apps vinculados.

**Layout:** 3 cards em grid + seção de apps públicos abaixo

#### Card de grupo
- Barra colorida na esquerda (4px)
- Nome do grupo, descrição, permissões (pills: Gerenciar/Deploy/Visualizar)
- Rodapé: contagem de membros + apps + botão editar
- Expandível via chevron → mostra membros (avatar+nome+email) e apps vinculados

#### Apps públicos
- Seção separada listando apps com `accessGroups: []`
- Ícone de globo ao lado do nome

#### Modelo de dados
```typescript
interface Group {
  id: "admin" | "editor" | "viewer";
  name: Record<"pt"|"en", string>;
  desc: Record<"pt"|"en", string>;
  perms: ("manage" | "deploy" | "view")[];
  color: string;
}

interface App {
  accessGroups: string[];  // [] = público, [...] = restrito
}
```

---

### 11. Admin — Credenciais

**Propósito:** Gerenciar tokens de API e usuários de serviço.

**Layout:** tabela + botão de criação

- Colunas: Nome · Token (mascarado `rk_live_****`) · Escopo · Criado · Ações (eye/copy/ban)
- Criar novo token: abre form com nome + escopos
- Revogar: confirmação antes de remover

---

### 12. Admin — Logs

**Propósito:** Acompanhamento quase em tempo real dos logs do servidor por
polling incremental finito (sem conexão SSE automática persistente).

**Layout:** painel de terminal com controles no topo

- Linhas incrementais com cursor e auto-scroll
- Filtros: nível (INFO/WARN/ERROR/DEBUG), app, texto livre
- Botão Pausar/Retomar; polling suspenso quando a página fica oculta
- Cores por nível: ERROR=vermelho, WARN=âmbar, INFO=texto normal, DEBUG=faint

---

### 13. Admin — Auditoria

**Propósito:** Registro de ações administrativas.

**Layout:** tabela filtrável + export

- Colunas: Quando · Ator (avatar+nome) · Ação (`code` mono) · Alvo · IP
- Categorias: Apps / Usuários / Segurança / Config
- Chips de filtro por categoria
- Busca por ator/ação/alvo
- Botão Export → CSV

---

## Interações & Comportamento

### Velocidade percebida (foco do projeto)
- **Barra de progresso top-of-page:** fina linha teal animada ao trocar de tela
- **Skeletons:** cards/linhas de tabela skeleton aparecem imediatamente; conteúdo real aparece após ~600ms
- **Filtros instantâneos:** busca e chips filtram sem debounce
- **Animações de entrada:** `fade-in` + `stagger` (delay escalonado) nos cards
- **Números animados:** métricas do dashboard "contam" até o valor final

### Estados de loading
- Skeleton: `background: linear-gradient(90deg, var(--surface-soft) 25%, var(--border) 50%, var(--surface-soft) 75%); background-size: 200% 100%; animation: shimmer 1.4s infinite`
- Barra de progresso: opacity 0→1, width aumenta em etapas (10%→58%→86%→100%), depois some

### Temas
- Toggle claro/escuro via atributo `data-theme` no `<html>`
- Persistido em `localStorage`

### i18n (PT/EN)
- Toggle no header
- Todas as strings têm versão PT e EN
- Persistido em `localStorage`

---

## Estado Global Compartilhado

```typescript
// Elevado no componente raiz (App)
const [featured, setFeatured]       // Set<appId> — apps favoritados
const [appAccess, setAppAccess]     // Map<appId, string[]> — grupos de acesso
const [portalUser, setPortalUser]   // User | null — usuário simulado (protótipo)
const [portalCfg, setPortalCfg]     // PortalConfig — configuração da aparência
```

`portalCfg` inclui:
```typescript
interface PortalConfig {
  title: string;
  tagline: string;
  accent: string;         // hex color
  theme: "light"|"dark"|"auto";
  layout: "grid"|"list"|"sections";
  featured: boolean;      // mostrar carrossel
  showSearch: boolean;
  showAccess: boolean;
  density: "comfortable"|"compact";
  logo: "wordmark"|"mark"|"custom";
  logoUrl: string;
  logoSize: number;
  logoGap: number;
  hero: "none"|"soft"|"bold";
  cover: "tint"|"gradient";
  seoTitle: string;
  seoDesc: string;
  ogImage: string;
  analytics: "none"|"ga"|"plausible"|"matomo";
  analyticsId: string;
  customCss: string;
  htmlTop: string;
  htmlAfterFeatured: string;
  htmlBeforeFooter: string;
  htmlFooter: string;
}
```

---

## Assets

| Asset | Origem | Uso |
|---|---|---|
| `ruscker-mark.svg` | Marca Ruscker | Logo em cabeçalhos e rodapé |
| `ruscker-mark-knockout.svg` | Marca Ruscker (knockout) | Fundos escuros |
| Tabler Icons | CDN unpkg | Todos os ícones da interface |
| Geist / Geist Mono | Google Fonts | Tipografia |
| Logos de frameworks | simpleicons.org CDN | Logos de Shiny, Jupyter, RStudio, etc. |

---

## Arquivos de Design

| Arquivo | Conteúdo |
|---|---|
| `Ruscker UX.html` | Ponto de entrada — carrega todos os scripts Babel |
| `src/data.jsx` | Dados mock: apps, réplicas, i18n, logos |
| `src/components.jsx` | Componentes compartilhados: Logo, AppCard, AppRow, Skeletons, StarIcon |
| `src/portal.jsx` | Portal público (3 variações + FeaturedCarousel + filtros) |
| `src/dashboard.jsx` | Dashboard de monitoramento (3 variações) |
| `src/admin.jsx` | Telas Apps + Mídia |
| `src/admin2.jsx` | Telas Disco + Credenciais + Logs |
| `src/admin3.jsx` | Telas Usuários + Grupos + Auditoria |
| `src/appearance.jsx` | Tela Aparência + mini-portal preview + code editor |
| `src/forms.jsx` | Editor de app + Import YAML |
| `src/app.jsx` | Shell do protótipo: navegação, estado global, MockHeader |
| `assets/ruscker.css` | Todos os tokens CSS e estilos de componentes |

---

## Como rodar o protótipo

Abrir `Ruscker UX.html` diretamente no navegador (não requer build — usa Babel in-browser).

**Controles do protótipo (barra cinza no topo):**
- **Tela:** alterna Portal ↔ Admin
- **Variação:** A/B/C para Portal e Dashboard
- **Reload:** replay dos skeletons de loading
- **Tema:** claro/escuro
- **Idioma:** PT/EN

**No Portal:** botão discreto no canto direito para simular usuário logado (controle de acesso por grupo).

---

## Notas de implementação

1. O carrossel de destaques deve re-filtrar quando o usuário muda — inclua `portalUser` e `appAccess` nas deps do `useMemo`
2. O editor CSS/HTML usa o padrão textarea+pre overlay — sincronize scroll entre os dois
3. A estrela de favorito deve propagar para o portal E para o preview de Aparência via estado global
4. O mutex Público/grupos no editor de app deve desabilitar os botões de grupo via `disabled` prop (não apenas opacity CSS)
5. `FeaturedCarousel` deve receber `portalCfg` como prop — sem ele crasha quando `feat.length > 0`
