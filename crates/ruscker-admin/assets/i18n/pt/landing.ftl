### Landing page — pt-BR
### Português brasileiro. Idioma de referência (fallback dos demais).

landing-title = Ruscker
landing-subtitle = Portal de aplicações e APIs
landing-signin = Entrar
landing-panel = Painel
landing-signout = Sair
landing-signed-in-as = { $user }

filter-search-placeholder = Buscar aplicação…
filter-access-all = Todos
filter-access-public = Públicos
filter-access-restricted = Restritos
filter-clear = Limpar filtros

type-all = Todos
type-app = Aplicações
type-package = Pacotes
type-talk = Apresentações
type-report = Relatórios
type-api = APIs

# 3-letter badge labels rendered inside each card. Operators see
# these in tight contexts (badges, dense tables) — keep them short
# enough to fit a 32px-wide pill.
type-app-abbr = APP
type-talk-abbr = APR
type-report-abbr = RLT
type-package-abbr = PCT
type-api-abbr = API
type-link-abbr = LNK

# Admin shell
admin-nav-dashboard = Painel
admin-nav-apps = Aplicações
admin-nav-images = Mídia
admin-nav-credentials = Credenciais
admin-nav-landing = Portal
admin-nav-blocks = Blocos
admin-blocks-title = Blocos HTML
admin-blocks-subtitle = Trechos de HTML customizado renderizados na landing (slots topo/rodapé).
admin-blocks-new = Novo bloco
admin-blocks-empty = Nenhum bloco ainda.
admin-blocks-col-slot = Slot
admin-blocks-col-title = Título
admin-blocks-col-status = Status
admin-blocks-enabled = ativo
admin-blocks-disabled = inativo
admin-blocks-edit = editar
admin-blocks-delete = excluir
admin-blocks-delete-confirm = Excluir este bloco?
admin-blocks-move-up = Mover para cima
admin-blocks-move-down = Mover para baixo
admin-blocks-form-new = Novo bloco
admin-blocks-form-edit = Editar bloco
admin-blocks-slot = Slot
admin-blocks-slot-help = Onde o bloco aparece na landing.
admin-blocks-slot-top = Topo (após o cabeçalho)
admin-blocks-slot-bottom = Rodapé (após o grid)
admin-blocks-title-label = Título (interno)
admin-blocks-title-placeholder = Rótulo para você identificar o bloco
admin-blocks-html = HTML
admin-blocks-html-help = Renderizado sem escape na landing — use só fontes confiáveis.
admin-blocks-origins = Origens permitidas (CSP)
admin-blocks-origins-help = Domínios separados por espaço liberados na CSP da landing (ex.: https://example.com).
admin-blocks-enabled-label = Ativo (renderizar na landing)
admin-blocks-save = Salvar
admin-blocks-cancel = Cancelar
admin-nav-audit = Auditoria
admin-nav-portal = Voltar ao portal
admin-nav-logout = Sair
role-current = Seu nível de acesso
role-viewer = Visualizador
role-editor = Editor
role-admin = Administrador

# — Login por usuário/senha + bootstrap por token (#107)
admin-login-help-user = Entre com seu usuário e senha.
admin-login-username-label = Usuário
admin-login-username-placeholder = seu usuário
admin-login-password-label = Senha
admin-login-password-placeholder = sua senha
admin-login-error-credentials = Usuário ou senha inválidos.
admin-login-use-token = Entrar com token de administrador
admin-login-use-password = Voltar ao login por senha
# — Configuração do primeiro admin
admin-setup-title = Crie a conta de administrador
admin-setup-help = Esta é a primeira vez. Escolha um usuário e uma senha para o administrador.
admin-setup-error = Não foi possível criar a conta. Verifique os dados.
admin-setup-password-label = Senha
admin-setup-submit = Criar administrador
# — Troca de senha / primeiro acesso
admin-pw-title = Alterar senha
admin-pw-help = Defina uma nova senha para a sua conta.
admin-pw-first-prompt = Você está usando uma senha definida pelo administrador. Deseja trocá-la agora?
admin-pw-current-label = Senha atual
admin-pw-new-label = Nova senha
admin-pw-confirm-label = Confirme a senha
admin-pw-error-current = Senha atual incorreta.
admin-pw-error-mismatch = As senhas não coincidem.
admin-pw-error-short = A senha deve ter ao menos 8 caracteres.
admin-pw-submit = Salvar senha
admin-pw-keep = Manter a senha atual
# — Gestão de usuários (admin)
admin-nav-users = Usuários
admin-users-title = Usuários
admin-users-subtitle = Crie e gerencie quem acessa o painel e com qual nível.
admin-users-new = Novo usuário
admin-users-create = Criar
admin-users-initial-password = Senha inicial
admin-users-initial-password-hint = O usuário será consultado se quer trocá-la no primeiro acesso.
admin-users-role = Nível
admin-users-col-user = Usuário
admin-users-col-role = Nível
admin-users-col-created = Criado em
admin-users-col-actions = Ações
admin-users-you = você
admin-users-must-change = Ainda usa a senha inicial
admin-users-save-role = Salvar nível
admin-users-groups = Grupos
admin-users-groups-placeholder = analistas, gestores
admin-users-groups-hint = Grupos separados por vírgula controlam quais apps restritos o usuário vê.
admin-users-col-groups = Grupos
admin-users-save-groups = Salvar grupos
admin-users-new-password = nova senha
admin-users-reset-password = Redefinir senha
admin-users-delete = Remover usuário
admin-users-confirm-delete = Remover este usuário?
admin-users-flash-created = Usuário criado.
admin-users-flash-saved = Alterações salvas.
admin-users-flash-deleted = Usuário removido.
admin-users-flash-last-admin = Não é possível remover ou rebaixar o último administrador.
admin-users-flash-bad-input = Dados inválidos (usuário vazio ou senha com menos de 8 caracteres).
admin-users-flash-exists = Já existe um usuário com esse nome.

# Admin dashboard
admin-dashboard-title = Painel de monitoramento
admin-dashboard-subtitle = Estado dos containers e sessões em tempo real
admin-dashboard-metric-containers = Containers
admin-dashboard-metric-sessions = Sessões ativas
admin-dashboard-metric-specs = Aplicações com réplica
admin-dashboard-metric-tracker = Sessões rastreadas
admin-dashboard-replicas-heading = Réplicas ativas
admin-dashboard-no-replicas = Nenhuma réplica em execução. As réplicas aparecem aqui quando o scaler garante o mínimo configurado ou quando uma requisição dispara o cold-start.
admin-dashboard-col-spec = Aplicação
admin-dashboard-col-state = Estado
admin-dashboard-col-uptime = Uptime
admin-dashboard-col-sessions = Sessões
admin-dashboard-col-host = Host
admin-dashboard-col-container = Container
admin-dashboard-col-cpu = CPU
admin-dashboard-col-memory = Memória
admin-dashboard-metric-memory = Memória usada
admin-dashboard-metrics-pending = aguardando primeira leitura
admin-dashboard-state-ready = pronto
admin-dashboard-state-starting = iniciando
admin-dashboard-state-draining = drenando
admin-dashboard-state-stopped = parado
admin-dashboard-state-failed = falhou
admin-dashboard-backend-missing = O backend Docker não está conectado — inicie o servidor com `--docker` para ver containers aqui.

# Admin login
admin-login-title = Entrar
admin-login-help = Digite o token de admin definido em RUSCKER_ADMIN_TOKEN.
admin-login-token-label = Token
admin-login-token-placeholder = Cole o token aqui
admin-login-submit = Entrar
admin-login-error-wrong = Token incorreto. Tente de novo.
admin-login-back-portal = ← portal público

# Apps list
admin-specs-title = Aplicações
admin-specs-subtitle = Catálogo de specs no banco
admin-specs-empty = Nenhuma aplicação ainda. Use { $cmd } para importar de um YAML.
admin-specs-add = Adicionar aplicação
admin-specs-col-id = ID
admin-specs-col-name = Nome
admin-specs-col-kind = Tipo
admin-specs-col-state = Estado
admin-specs-col-updated = Atualizado
admin-specs-col-version = Versão
admin-specs-col-actions = Ações
admin-specs-filter-search = Buscar por id ou nome…
admin-specs-filter-kind-all = Todos os tipos
admin-specs-filter-state-all = Ativos e inativos
admin-specs-edit = Editar
admin-specs-delete = Apagar

# Spec form (new / edit)
spec-form-title-new = Nova aplicação
spec-form-crumb-new = Nova
spec-form-crumb-edit = Editar
spec-form-cancel = Cancelar
spec-form-save = Salvar alterações
spec-form-kind = Tipo
spec-form-kind-app = App container
spec-form-kind-talk = Apresentação
spec-form-kind-report = Relatório
spec-form-kind-package = Pacote
spec-form-kind-api = API
spec-form-kind-link = Link externo
spec-form-identity = Identidade
spec-form-id = ID
spec-form-id-help-new = Gerado pelo operador. Aparece em /app/<id>/.
spec-form-id-help-edit = ID é imutável depois de criado.
spec-form-name = Nome de exibição
spec-form-desc = Descrição
spec-form-visual = Visual
spec-form-logo = Logo do card
spec-form-logo-help = URL ou caminho /assets/img/foo.png. Veja docs/IMAGES.md.
spec-form-logo-pick-help = Ou escolha uma imagem já enviada na biblioteca de mídia.
spec-form-access = Acesso
spec-form-state = Estado
spec-form-state-active = Ativo
spec-form-state-inactive = Inativo
spec-form-subject = Assunto
spec-form-container = Container
spec-form-image = Imagem Docker
spec-form-seats = Sessões/container
spec-form-lifetime = Vida máx. (min)
spec-form-lifetime-help = 360 = 6 horas
spec-form-link-section = Link externo
spec-form-link = URL de destino
spec-form-meta = Metadados
spec-form-updated = Atualizado em
spec-form-updated-help = Vazio para usar a data de hoje.
spec-form-preview = Prévia do card
spec-form-preview-help = Atualiza ao vivo conforme você edita.
spec-form-actions = Ações
spec-form-delete = Excluir aplicação
spec-form-delete-confirm = Tem certeza? Esta ação não pode ser desfeita.

spec-form-error-id-required = ID é obrigatório.
spec-form-error-id-shape = ID deve começar com letra e conter apenas letras, números, "_" e "-".
spec-form-error-id-duplicate = Já existe uma aplicação com esse ID.
spec-form-error-name-required = Nome de exibição é obrigatório.
spec-form-error-number = Um campo numérico tem valor não numérico.
spec-form-error-cpu = O limite de CPU deve ser um número positivo (ex.: 0.5).
spec-form-error-memory = O limite de memória deve ser um tamanho como 512m ou 1.5g.
spec-form-error-replica-range = Réplicas máx. deve ser maior ou igual a réplicas mín.

# Admin image library
admin-images-title = Biblioteca de mídia
admin-images-subtitle = PNG, JPEG e WebP são convertidos para WebP. SVG passa direto.
admin-images-drop-here = Clique para escolher um arquivo
admin-images-formats = PNG · JPEG · WebP · SVG · até 10 MB
admin-images-upload = Enviar
admin-images-uploaded = Imagem enviada:
admin-images-empty = Nenhuma imagem ainda. Envie a primeira acima.
admin-images-delete = Excluir
admin-images-delete-confirm = Excluir essa imagem? Specs que referenciam o arquivo passarão a mostrar o cover tintado.

# Admin credentials
admin-creds-title = Credenciais do registry
admin-creds-subtitle = Senhas criptografadas em repouso com AES-256-GCM. Nunca aparecem no YAML nem no painel depois de salvas.
admin-creds-form-title = Adicionar / atualizar credencial
admin-creds-name = Nome
admin-creds-name-help = Identificador único. Use o mesmo nome nas specs.
admin-creds-registry = Registry
admin-creds-username = Usuário
admin-creds-password = Senha / token
admin-creds-password-help = Será criptografada e nunca será exibida novamente.
admin-creds-save = Salvar credencial
admin-creds-saved = Credencial salva:
admin-creds-empty = Nenhuma credencial cadastrada.
admin-creds-delete = Excluir
admin-creds-delete-confirm = Apagar essa credencial?
admin-creds-col-name = Nome
admin-creds-col-registry = Registry
admin-creds-col-username = Usuário
admin-creds-col-created = Criada em
admin-creds-key-missing-title = RUSCKER_MASTER_KEY não está configurada
admin-creds-key-missing-help = O store de credenciais precisa de uma chave de 32 bytes em hex (64 chars) ou base64 (44 chars). Gere uma assim:

# Admin landing editor
admin-landing-title = Editor da landing
admin-landing-crumb = Configurações · Landing page
admin-landing-subtitle = Personalize o portal público. Mudanças entram em vigor no próximo refresh do visitante.
admin-landing-open-portal = Abrir portal
admin-landing-save = Salvar
admin-landing-saved = Configurações salvas. Recarregue o portal público para ver.
admin-landing-colors = Cores do cabeçalho
admin-landing-header-bg = Cor de fundo
admin-landing-bg-help = Vazio = usa a cor padrão do tema (claro/escuro).
admin-landing-header-fg = Cor do texto
admin-landing-clear = Limpar
admin-landing-intro = Texto introdutório (padrão)
admin-landing-intro-default = Padrão (fallback para todos os idiomas)
admin-landing-intro-default-placeholder = Bem-vindo ao portal…
admin-landing-intro-help = Texto exibido entre o cabeçalho e os filtros. Vazio = sem texto.
admin-landing-intro-locales = Texto introdutório por idioma
admin-landing-intro-pt = Português
admin-landing-intro-en = Inglês
admin-landing-intro-es = Espanhol
admin-landing-intro-fr = Francês
admin-landing-preview = Prévia do portal
admin-landing-preview-help = Aproximação visual do cabeçalho e texto. Cards e filtros ficam como na landing real.
admin-landing-preview-empty = (sem texto introdutório)
admin-landing-seo = SEO e compartilhamento
admin-landing-seo-title = Título da página (SEO)
admin-landing-seo-title-placeholder = Padrão: título do portal
admin-landing-seo-title-help = Define o título da aba e o og:title. Vazio usa o título padrão do portal.
admin-landing-seo-description = Descrição (meta description)
admin-landing-seo-description-placeholder = Resumo do portal para buscadores e redes sociais
admin-landing-seo-description-help = Usada na meta description e no og:description. Vazio usa o texto de introdução.
admin-landing-og-image = Imagem de compartilhamento (og:image)
admin-landing-og-image-help = URL ou caminho (ex.: /assets/img/og.png) mostrado ao compartilhar em redes sociais.
admin-landing-analytics = Analytics
admin-landing-analytics-html = Snippet de analytics
admin-landing-analytics-html-help = HTML inserido no <head> da landing (ex.: tag <script> do Plausible/Matomo/GA). Renderizado sem escape — use só fontes confiáveis.
admin-landing-analytics-origins = Origens permitidas (CSP)
admin-landing-analytics-origins-help = Domínios separados por espaço (ex.: https://plausible.io) liberados na CSP da landing para o script carregar e reportar.
admin-landing-future-title = Blocos HTML
admin-landing-future-help = Gerencie blocos HTML customizados (banners, avisos) na seção Blocos do menu.

# Admin audit log
admin-audit-title = Auditoria
admin-audit-subtitle = Todas as alterações administrativas, do mais recente para o mais antigo. Limite de 100 eventos por consulta.
admin-audit-family = Família
admin-audit-family-all = Todas as ações
admin-audit-family-spec = Aplicações
admin-audit-family-image = Imagens
admin-audit-family-credential = Credenciais
admin-audit-family-landing = Portal
admin-audit-actor = Autor
admin-audit-actor-all = Todos os autores
admin-audit-target-placeholder = Buscar por alvo (ex: spec:sales-dashboard)
admin-audit-apply = Aplicar
admin-audit-empty = Nenhuma alteração ainda — ou o filtro não bate em nada.
admin-audit-limit-hint = Mostrando os 100 mais recentes que batem com o filtro. Ajuste os filtros para refinar.

card-cta-open = Abrir
card-cta-link = Acessar
card-cta-open-app = Abrir aplicativo
card-cta-open-talk = Abrir apresentação
card-cta-open-report = Abrir relatório
card-cta-open-package = Abrir documentação
card-cta-open-api = Ver documentação
card-updated = Atualizado em { $date }
status-new = novo { $date }
status-updated = atualizado { $date }
sort-label = Ordenar
sort-recent = Recentes
sort-name = Nome
search-shortcut = ⌘ K

filter-subject-label = Assunto
filter-subject-all = Todos os assuntos
filter-status-active = Apenas ativos
filter-status-all = Ativos e inativos
filter-status-inactive-only = Apenas inativos
card-state-active = Disponível
card-state-inactive = Indisponível
card-access-public = Acesso público
card-access-restricted = Acesso restrito

theme-light = Claro
theme-dark = Escuro
theme-auto = Automático

# Top-right chrome cluster (#182 + #183)
chrome-cluster-label = Preferências da página
chrome-theme-label = Tema
chrome-language-label = Idioma
chrome-account-label = Conta
chrome-signin = Entrar
chrome-signed-in-as-prefix = Conectado como
chrome-panel = Painel
chrome-change-password = Mudar senha
chrome-signout = Sair

# Admin logs viewer
admin-logs-title = Logs do container
admin-logs-back = Voltar ao painel
admin-logs-replica = Réplica
admin-logs-empty = Sem saída de log para esta réplica ainda.
admin-logs-tail-note = Mostrando as últimas linhas (mais recentes ao final).

# Dashboard replica actions
admin-dashboard-action-stop = Parar
admin-dashboard-action-restart = Reiniciar
admin-dashboard-confirm-stop = Parar esta réplica? O auto-scaler pode recriá-la se o mínimo configurado exigir.
admin-dashboard-confirm-restart = Reiniciar esta réplica? A sessão ativa será perdida.
admin-logs-follow = Ao vivo
admin-logs-follow-stop = Parar

# Admin YAML import
admin-import-button = Importar YAML
admin-import-title = Importar configuração YAML
admin-import-help = Cole ou selecione um application.yml do ShinyProxy ou Ruscker. O import é idempotente e não remove specs existentes.
admin-import-file = Arquivo .yml / .yaml
admin-import-submit = Importar
admin-import-cancel = Cancelar
admin-import-ok = Import concluído: { $created } criados, { $updated } atualizados, { $unchanged } inalterados.
admin-import-ok-warnings = { $warnings } aviso(s) de validação — revise as credenciais embutidas e nomes vazios.
admin-import-err = Falha no import: { $msg }

# Gradient builder
admin-grad-solid = Sólida
admin-grad-gradient = Gradiente
admin-grad-linear = Linear
admin-grad-radial = Radial
admin-grad-add-stop = Adicionar cor
admin-grad-remove-stop = Remover cor

# Spec form — card cover
spec-form-cover = Cover do card
spec-form-cover-auto = Auto (cor do tipo)
spec-form-cover-auto-help = Usa o tom padrão do tipo do card. Escolha Sólida ou Gradiente para personalizar.

# ── Formulário de spec: seção avançada + ajuda por campo (#2) ──────
spec-form-advanced = Avançado
spec-form-advanced-hint = Tudo opcional — deixe em branco para manter o padrão.
spec-form-api-section = API
spec-form-scaling-section = Escala
spec-form-resources-section = Recursos
spec-form-lifecycle-section = Ciclo de vida
spec-form-api-port = Porta do container
spec-form-api-rate-limit = Limite de taxa
spec-form-api-docs-path = Caminho dos docs
spec-form-api-health-path = Caminho de health
spec-form-api-cors = Habilitar CORS permissivo
spec-form-min-replicas = Réplicas mín.
spec-form-max-replicas = Réplicas máx.
spec-form-concurrent = Requisições por réplica
spec-form-cpu-limit = Limite de CPU
spec-form-memory-limit = Limite de memória
spec-form-heartbeat = Timeout de heartbeat (ms)
spec-help-kind = Que tipo de coisa é. Define o roteamento, o selo do card e se um container é iniciado.
spec-help-id = Identificador estável usado na URL (/app/<id>). Minúsculas, dígitos, "-" e "_"; não pode mudar após criado.
spec-help-name = O título exibido no card da landing.
spec-help-desc = Descrição curta no card. HTML inline (ex.: um link) é permitido.
spec-help-logo = Imagem do card — um caminho em /assets/img/ ou uma URL externa. Em branco, usa um tom gerado.
spec-help-cover = Fundo do card: tom automático por tipo, cor sólida ou gradiente.
spec-help-access = Mostra um cadeado (restrito) ou aberto (público). Apenas visual — o MVP não impõe autenticação.
spec-help-state = Cards ativos aparecem na landing; inativos ficam ocultos.
spec-help-subject = Assunto/área usado pelo filtro Assunto da landing (ex.: "Vendas", "Pesquisa").
spec-help-image = Imagem Docker a executar, como repositório:tag (ex.: org/app:latest).
spec-help-seats = Quantas sessões simultâneas um container atende antes de subir outro.
spec-help-lifetime = Limite rígido, em minutos, de quanto tempo um container roda antes de ser reciclado.
spec-help-link = URL de destino para cards de link externo — clicar no card navega para cá.
spec-help-updated = Data exibida no card (DD/MM/AAAA). Em branco, carimba a data de hoje.
spec-help-api-port = Porta em que a API escuta dentro do container. Padrão 8080.
spec-help-api-rate-limit = Limite por cliente no proxy, como N/unidade (ex.: 100/min, 5/s). Acima do limite retorna 429. Vazio = sem limite.
spec-help-api-docs-path = Caminho onde a API serve a documentação OpenAPI/Swagger. Padrão /__docs__.
spec-help-api-health-path = Caminho consultado para readiness antes da réplica entrar no pool. Padrão /__healthz__.
spec-help-api-cors = Adiciona cabeçalhos CORS permissivos e responde ao preflight. Desligado por padrão.
spec-help-min-replicas = Containers mantidos sempre quentes — o pool nunca desce abaixo disso. Padrão 0.
spec-help-max-replicas = Teto até onde o auto-scaler pode subir. Vazio = ilimitado.
spec-help-concurrent = Requisições que uma réplica de API atende antes do scaler adicionar outra.
spec-help-cpu-limit = CPU máxima em núcleos fracionários (ex.: 0,5 = meio núcleo). Vazio = ilimitado.
spec-help-memory-limit = Memória máxima, ex.: 512m ou 1.5g. Vazio = ilimitado.
spec-help-heartbeat = Timeout de sessão ociosa em milissegundos; -1 nunca expira. Vazio = usa o padrão global.
admin-blocks-slot-empty = Nenhum bloco neste slot ainda.
admin-blocks-drag-hint = Arraste pela alça para reordenar os blocos dentro do slot.
spec-form-volumes-section = Volumes
spec-form-volumes = Montagens de volume
spec-form-volumes-help = Um bind por linha — /host:/container (use :ro para somente leitura). Adicione quantos precisar.
spec-help-volumes = Monta diretórios do host no container (ex.: dados persistentes, ou estáticos que o app serve). Só admin; montar caminhos do host equivale a root.
spec-form-routing-section = Roteamento
spec-form-inject-base-href = Reescrever o HTML do app para o sub-caminho
spec-form-inject-base-href-help = Ligado por padrão. O Ruscker reescreve <base href> e URLs relativas à raiz para que um app que assume estar na raiz do servidor funcione sob o sub-caminho /app/. Desligue só se o app lê X-Forwarded-Prefix e monta os próprios caminhos.
spec-help-inject-base-href = O Ruscker sempre encaminha X-Forwarded-Prefix / X-Script-Name (e X-Forwarded-Proto/-Host). Frameworks como FastAPI (root_path), Dash e Streamlit se auto-roteiam por eles — aí esta reescrita de HTML fica redundante.
spec-form-error-volume = Cada volume deve ser /host:/container (opcional :ro).
admin-nav-logs = Logs
admin-proclog-title = Logs
admin-proclog-subtitle = Log recente do processo Ruscker (ao vivo).
admin-proclog-unavailable = Buffer de log não disponível (o servidor iniciou sem a camada de logging).

# ── spec-form advanced params (#211/#212) ──────────────────────────
spec-form-runtime-section = Runtime
spec-form-container-port = Porta do container
spec-help-container-port = Porta em que o app escuta dentro do container. Em branco = padrão por tipo (3838 para Shiny). Defina para Streamlit (8501), Dash (8050) ou Jupyter (8888).
spec-form-platform = Plataforma
spec-help-platform = Plataforma Docker (ex.: linux/amd64) para rodar imagens de outra arquitetura via emulação. Em branco = o daemon escolhe pelo manifesto.
spec-form-container-lifetime = Vida útil do container (min)
spec-help-container-lifetime = Limite suave em minutos antes de reciclar o container. Em branco = sem limite suave.
spec-form-stop-on-logout = Parar ao sair (logout)
spec-help-stop-on-logout = Para o container do usuário quando ele faz logout. Desligado por padrão.
spec-form-env-section = Ambiente + comando
spec-form-container-env = Variáveis de ambiente
spec-form-container-env-help = Uma por linha, NOME=valor. Para segredos, referencie uma variável de ambiente em vez de colar o valor.
spec-help-container-env = Injetadas no container (container-env). Em branco = nenhuma. Para segredos, use interpolação de variável de ambiente.
spec-form-container-cmd = Comando (sobrescrever)
spec-form-container-cmd-help = Um argumento por linha. Em branco = usa o CMD da imagem.
spec-help-container-cmd = Sobrescreve o comando do container (container-cmd), como lista de argumentos.
spec-form-registry-section = Registro (imagens privadas)
spec-form-registry-domain = Domínio do registro
spec-help-registry-domain = Host do registro para imagens privadas (ex.: docker.io, ghcr.io). Em branco = Docker Hub.
spec-form-registry-username = Usuário
spec-help-registry-username = Usuário para autenticar no pull de uma imagem privada.
spec-form-registry-password = Senha
spec-help-registry-password = Use uma variável de ambiente — nunca cole a senha em texto. Só usada junto com o usuário.
spec-form-registry-credential = Credencial salva
spec-help-registry-credential = Nome de uma credencial do cofre, em vez de usuário/senha inline.
spec-form-registry-help = Para imagens privadas. Mantenha segredos em variáveis de ambiente; o validador alerta sobre senhas em texto.
spec-form-access-section = Acesso
spec-form-access-groups = Grupos permitidos
spec-help-access-groups = Grupos que podem ver e acessar o app (separados por vírgula). Em branco, com usuários também em branco = aberto a todos.
spec-form-access-users = Usuários permitidos
spec-help-access-users = Usuários que podem ver e acessar o app (separados por vírgula).
spec-form-access-help = Ambos em branco = card aberto a todos (inclusive anônimos). Com algum valor, só usuários logados que combinam — e admins sempre.
spec-form-cpu-request = Reserva de CPU
spec-help-cpu-request = Reserva suave de CPU em núcleos (container-cpu-request). Em branco = sem reserva.
spec-form-memory-request = Reserva de memória
spec-help-memory-request = Reserva suave de memória, ex.: 256m. Em branco = sem reserva.
spec-form-max-body-size = Tamanho máx. do corpo
spec-help-max-body-size = Limite por spec do corpo das requisições, ex.: 10m. Em branco = usa o limite global.
spec-form-scale-up = Limiar de scale-up
spec-help-scale-up = Fração de utilização (0–1) que dispara subir uma réplica. Em branco = padrão do scaler.
spec-form-scale-down = Limiar de scale-down
spec-help-scale-down = Fração de utilização (0–1) abaixo da qual uma réplica é recolhida. Em branco = padrão do scaler.
spec-form-scale-down-grace = Carência de scale-down (s)
spec-help-scale-down-grace = Segundos abaixo do limiar antes de recolher a réplica. Em branco = padrão.
spec-form-drain-timeout = Timeout de drenagem (s)
spec-help-drain-timeout = Segundos para drenar as sessões de uma réplica antes de pará-la. Em branco = padrão.
spec-form-routing-strategy = Estratégia de roteamento
spec-help-routing-strategy = Como o balanceador escolhe a réplica. Em branco = padrão por tipo (least-connections para apps, round-robin para API).
spec-form-routing-default = Padrão (por tipo)
spec-form-placement = Posicionamento (multi-host)
spec-help-placement = Como distribuir réplicas entre hosts Docker. Em branco = spread. Só relevante com proxy.hosts.
spec-form-placement-default = Padrão (spread)
spec-form-anti-affinity = Anti-afinidade
spec-help-anti-affinity = Prefere hosts distintos para as réplicas deste spec (multi-host). Desligado por padrão.
spec-form-error-port = A porta deve ser um número entre 1 e 65535.
spec-form-error-threshold = O limiar deve ser um número entre 0 e 1.
