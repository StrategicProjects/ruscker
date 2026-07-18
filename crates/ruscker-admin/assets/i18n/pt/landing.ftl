### Landing page — pt-BR
### Português brasileiro. Idioma de referência (fallback dos demais).

landing-title = Ruscker
landing-subtitle = Portal de aplicações e APIs
landing-signin = Entrar
landing-panel = Painel
landing-signout = Sair
landing-signed-in-as = { $user }

filter-search-placeholder = Buscar aplicação…
filter-clear = Limpar filtros

type-all = Todos
type-app = Aplicações
type-package = Pacotes
type-link = Links
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
admin-nav-dashboard = Containers
admin-nav-apps = Aplicações
admin-nav-images = Mídia
admin-nav-credentials = Credenciais
admin-nav-landing = Aparência
admin-nav-blocks = Blocos
admin-blocks-title = Blocos HTML
admin-blocks-subtitle = Trechos HTML renderizados na landing (topo / base).
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
admin-blocks-slot-bottom = Base (após a grade)
admin-blocks-title-label = Título (interno)
admin-blocks-title-placeholder = Rótulo para você identificar o bloco
admin-blocks-html = HTML
admin-blocks-html-help = Renderizado sem escape na landing — use só fontes confiáveis.
admin-blocks-origins = Origens permitidas (CSP)
admin-blocks-origins-help = Domínios separados por espaço liberados na CSP da landing (ex.: https://example.com).
admin-blocks-enabled-label = Ativo (renderizar na landing)
admin-blocks-save = Salvar
admin-blocks-cancel = Cancelar
admin-blocks-position = Posição
admin-blocks-pos-top = Topo
admin-blocks-pos-bottom = Base
admin-blocks-done = Concluir
admin-blocks-delete-block = Excluir bloco
admin-nav-audit = Atividades
admin-nav-portal = Portal
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
admin-pw-first-prompt = Você está usando uma senha definida pelo administrador. Defina uma nova senha para continuar.
admin-pw-current-label = Senha atual
admin-pw-new-label = Nova senha
admin-pw-confirm-label = Confirme a senha
admin-pw-error-current = Senha atual incorreta.
admin-pw-error-mismatch = As senhas não coincidem.
admin-pw-error-short = A senha não atende à política: mínimo de 8 caracteres, com 1 maiúscula, 1 minúscula, 1 número e 1 caractere especial.
admin-pw-submit = Salvar senha
admin-pw-reveal = Mostrar/ocultar senha
# — Gestão de usuários (admin)
admin-nav-users = Usuários
admin-users-title = Gestão de Usuários
admin-users-subtitle = Criação e edição de usuários.
admin-users-edit = Editar usuário
admin-users-edit-title = Editar usuário
admin-users-edit-subtitle = Atualize acesso e perfil em um único salvamento.
admin-users-account = Dados da conta
admin-users-save = Salvar alterações
admin-users-cancel = Cancelar
admin-users-password-section = Redefinição de senha
admin-users-password-reset-hint = Define uma senha temporária e exige que o usuário a altere no próximo acesso.
admin-users-new = Novo usuário
admin-users-create = Criar
admin-users-initial-password = Senha inicial
admin-users-initial-password-hint = Mínimo de 8 caracteres. O usuário será obrigado a trocá-la no primeiro acesso.
admin-users-role = Nível
admin-users-col-user = Usuário
admin-users-col-role = Nível
admin-users-col-created = Criado em
admin-users-col-actions = Ações
admin-users-you = você
# Busca + paginação server-side na tabela de usuários (#999)
admin-users-search = Buscar
admin-users-search-clear = Limpar busca
admin-users-search-none = Nenhum usuário corresponde à busca.
admin-users-pager-status = Página { $page } de { $pages } · { $total } { $total ->
        [one] usuário
       *[other] usuários
    }
admin-users-prev = Anterior
admin-users-next = Próxima
admin-users-must-change = Ainda usa a senha inicial
admin-users-save-role = Salvar nível
admin-users-groups = Grupos
admin-users-setor = Setor
admin-users-setor-placeholder = ex.: GAPE
admin-users-email = E-mail
admin-users-celular = Celular
admin-users-col-profile = Perfil
admin-users-save-profile = Salvar perfil
admin-users-import-review = Revisar import
admin-users-import-change = Trocar arquivo
admin-users-import-choose = Escolher arquivo CSV
admin-users-import-help = Colunas: username, role, password, groups, setor, email, celular. A primeira linha é o cabeçalho. Separador: vírgula (,); no Windows, salve o CSV no padrão Unix com encoding UTF-8. Papéis (role) em inglês: viewer, editor, admin.
admin-users-import-title = Importar usuários
admin-users-import-preview-title = Prévia da importação
admin-users-import-col-status = Status
admin-users-import-status-ok = vai importar
admin-users-import-status-exists = já existe — ignorado
admin-users-import-status-bad-username = usuário inválido
admin-users-import-status-bad-password = senha fora da política (mín. 8, com maiúscula, minúscula, número e especial)
admin-users-import-status-bad-role = nível inválido
admin-users-import-confirm = Importar usuários
admin-users-import-cancel = Cancelar
admin-users-import-done-prefix = Importados:
admin-users-import-skipped-prefix = ignorados:
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
admin-users-flash-bad-input = Dados inválidos: o usuário só pode ter letras, números e _ . @ - , e a senha precisa de ao menos 8 caracteres.
admin-users-username-rule = Apenas letras, números e _ . @ - (sem espaços nem acentos).
admin-users-password-rule = Mínimo de 8 caracteres, com ao menos 1 maiúscula, 1 minúscula, 1 número e 1 caractere especial.
admin-users-flash-weak-password = Senha fraca — a política exige mínimo de 8 caracteres, com 1 maiúscula, 1 minúscula, 1 número e 1 caractere especial.
admin-users-generate-password = Gerar senha aleatória
admin-users-flash-exists = Já existe um usuário com esse nome.

# Admin dashboard
admin-dashboard-title = Gestão dos Containers
admin-dashboard-subtitle = Estado dos containers e sessões em tempo real
admin-dashboard-live = Ao vivo
admin-dashboard-filter-search = Filtrar app…
admin-dashboard-metric-containers = Containers
admin-dashboard-metric-sessions = Sessões ativas
admin-dashboard-metric-specs = Aplicações com réplica
admin-dashboard-metric-tracker = Sessões rastreadas
admin-dashboard-replicas-heading = Réplicas ativas
admin-dashboard-grouped-by = Agrupadas por aplicação
admin-dashboard-expand-all = Expandir tudo
admin-dashboard-collapse-all = Recolher tudo
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
admin-specs-title = Gestão das Aplicações
admin-specs-refresh = Recarregar
admin-specs-subtitle = Informações e especificações das imagens das aplicações
admin-specs-empty = Nenhuma aplicação ainda. Use { $cmd } para importar de um YAML.
admin-specs-add = Adicionar aplicação
admin-specs-col-id = ID
admin-specs-col-name = Nome
admin-specs-col-kind = Tipo
admin-specs-col-state = Estado
admin-specs-col-updated = Atualizado
admin-specs-col-version = Versão
admin-specs-col-access = Acessos
admin-specs-col-access-groups = Acesso
admin-specs-access-public = público
admin-specs-col-access-help = Total de acessos (visitas a apps + cliques em cards externos)
admin-specs-col-actions = Ações
admin-specs-filter-search = Buscar por id ou nome…
admin-specs-kind-interactive = Interativo
admin-specs-kind-external = Externo
admin-specs-filter-kind-all = Todos os tipos
admin-specs-filter-state-all = Ativos e inativos
admin-specs-edit = Editar
admin-specs-duplicate = Duplicar
admin-specs-update-image = Atualizar imagem (re-pull)
admin-specs-update-image-running = Atualizando imagem…
admin-specs-update-image-ok = Imagem atualizada
admin-specs-update-image-fail = Falha ao atualizar a imagem
admin-specs-config-badge = config
admin-specs-config-defined = Definido no YAML — somente leitura aqui; edite o arquivo
admin-specs-delete = Apagar
admin-specs-archive = Arquivar — ocultar o card do portal
admin-specs-unarchive = Reativar — voltar a exibir o card no portal
admin-specs-delete-confirm = Excluir esta aplicação? Os contêineres serão parados e a configuração removida. Esta ação não pode ser desfeita.

# Spec form (new / edit)
spec-form-title-new = Nova aplicação
spec-form-crumb-new = Nova
spec-form-crumb-edit = Editar
spec-form-cancel = Voltar para aplicações
spec-form-save = Salvar alterações
spec-form-created-title = Aplicação criada
spec-form-created-body = A aplicação foi criada com sucesso. O que deseja fazer agora?
spec-form-created-stay = Voltar ao formulário
spec-form-created-list = Ir para a lista de aplicações
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
spec-form-visual = Aparência
spec-form-logo = Logo do card
spec-form-logo-help = URL ou caminho /assets/img/foo.png. Veja docs/IMAGES.md.
spec-form-logo-pick-help = Ou escolha uma imagem já enviada na biblioteca de mídia.
spec-form-state = Estado
spec-form-state-active = Ativo
spec-form-state-inactive = Inativo
spec-form-subject = Assunto
spec-form-container = Container
spec-form-image = Imagem Docker
spec-form-image-check = Verificar
spec-form-image-checking = Verificando…
spec-form-image-present = No servidor
spec-form-image-absent = Ausente — será puxada no primeiro acesso
spec-form-image-unresolved = Contém variável de ambiente — resolvida no pull
spec-form-image-no-backend = Docker não conectado — não dá para verificar
spec-form-image-error = Falha ao verificar a imagem
spec-form-image-pull = Puxar
spec-form-image-repull = Atualizar imagem
spec-form-image-repull-help = Baixa novamente do registry — use após republicar a mesma tag (bytes ou arquitetura diferentes).
spec-form-image-pulling = Puxando…
spec-form-seats = Sessões/container
spec-form-lifetime = Vida máx. (min)
spec-form-lifetime-help = 360 = 6 horas
spec-form-link-section = Link externo
spec-form-link = URL de destino
spec-form-accent = Cor de destaque
spec-form-accent-help = Tinge a capa do card (quando não há cover definido).
spec-form-monogram = Monograma
spec-form-monogram-ph = AB
spec-form-monogram-help = Mostrado na capa quando não há logo.
spec-form-meta = Acesso & escala
spec-form-restricted = Acesso restrito
spec-form-restricted-hint = Exige login para abrir
spec-form-initial-replicas = Réplicas iniciais
spec-form-autoscaling = Autoescalonamento
spec-form-autoscaling-hint = Escala réplicas conforme a demanda
spec-form-updated = Atualizado em
spec-form-updated-help = Vazio para usar a data de hoje.
spec-form-preview = Prévia do card
spec-form-preview-help = Atualiza ao vivo conforme você edita.
spec-form-actions = Ações
spec-form-delete = Excluir aplicação
spec-form-delete-confirm = Tem certeza? Esta ação não pode ser desfeita.

spec-form-env-key = CHAVE
spec-form-error-id-required = ID é obrigatório.
spec-form-error-id-shape = ID deve começar com letra e conter apenas letras, números, "_" e "-".
spec-form-error-id-duplicate = Já existe uma aplicação com esse ID.
spec-form-error-name-required = Nome de exibição é obrigatório.
spec-form-error-number = Um campo numérico tem valor não numérico.
spec-form-error-mfa-days = “Solicitar novamente após N dias” deve ser um inteiro entre 0 e 30.
spec-form-error-max-replicas-zero = Máx. de containers deve ser ao menos 1 (0 faz o app nunca iniciar).
spec-form-error-cpu = O limite de CPU deve ser um número positivo (ex.: 0.5).
spec-form-error-memory = O limite de memória deve ser um tamanho como 512m ou 1.5g.
spec-form-error-replica-range = Réplicas máx. deve ser maior ou igual a réplicas mín.
spec-form-error-stale = Outra pessoa salvou este app enquanto você editava. Revise os valores atuais abaixo e envie novamente.

# Admin image library
admin-images-title = Biblioteca de mídia
admin-images-subtitle = Imagens utilizadas pelo portal
admin-images-formats-help = PNG, JPEG e WebP são convertidos para WebP. SVG passa direto.
admin-images-drop-here = Clique para escolher um arquivo
admin-images-formats = PNG · JPEG · WebP · SVG · até 10 MB
admin-images-upload = Enviar
admin-images-choose = Escolher imagem
admin-images-builtin = Logos integrados
admin-images-builtin-tag = integrado
admin-images-uploaded = Imagem enviada:
admin-images-renamed = renomeada porque este nome já estava em uso:
admin-images-rename = Renomear
admin-images-rename-prompt = Novo nome do arquivo (a extensão é mantida):
admin-images-rename-taken = Já existe uma imagem com esse nome. Escolha outro.
admin-images-rename-invalid = Nome inválido.
admin-images-empty = Nenhuma imagem ainda. Envie a primeira acima.
admin-images-delete = Excluir
admin-images-delete-confirm = Excluir essa imagem? Specs que referenciam o arquivo passarão a mostrar o cover tintado.
admin-images-inuse = Em uso
admin-images-inuse-help = Usada num card ou nos logos da landing
admin-images-delete-confirm-inuse = Esta imagem está EM USO. Se deletar, os apps que a usam voltam ao logo padrão do Ruscker (o card não quebra). Deletar?
admin-images-search = Buscar imagens…
admin-images-type-all = Todos os tipos
admin-images-no-match = Nenhuma imagem corresponde à busca.

# Admin credentials
admin-creds-title = Gestão de Credenciais
admin-creds-subtitle = Criação de credenciais. As credenciais são criptografadas em repouso com AES-256-GCM. Nunca aparecem no YAML nem no painel depois de salvas.
admin-creds-form-title = Adicionar / atualizar credencial
admin-creds-name = Nome
admin-creds-name-help = Identificador único. Use o mesmo nome nas specs.
admin-creds-registry = Registry
admin-creds-username = Usuário
admin-creds-password = Senha / token
admin-creds-password-help = Será criptografada e nunca exibida novamente. Ou informe uma referência a variável de ambiente — resolvida no pull e nunca armazenada.
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
admin-landing-title = Aparência do Portal
admin-landing-crumb = Configurações · Landing page
admin-landing-subtitle = Configurações de logo, barras, rodapés etc.
admin-landing-scope-help = Estas opções (cores, textos de introdução, SEO, blocos customizados) se aplicam à landing pública ao vivo — salvas aqui, exibidas na próxima visita, sem restart. É um conjunto fixo de configurações, não um editor de CSS arbitrário.
admin-landing-open-portal = Abrir portal
admin-landing-save = Salvar
admin-landing-reset = Restaurar padrão
admin-landing-reset-help = Volta a aparência do portal ao padrão original
admin-landing-reset-confirm = Restaurar a aparência padrão? Cores, tema, estilo do cabeçalho, capas e layout voltam ao original. Títulos, logos, textos, SEO, CSS personalizado e blocos HTML são mantidos.
admin-landing-saved = Configurações salvas. Recarregue o portal público para ver.
admin-landing-header-bg = Cor de fundo personalizada
admin-landing-bg-help = Vazio = usa a cor padrão do tema (claro/escuro).
admin-landing-header-fg = Cor do texto
admin-landing-clear = Limpar
admin-landing-intro = Texto introdutório (padrão)
admin-landing-intro-default = Padrão (fallback para todos os idiomas)
admin-landing-intro-default-placeholder = Bem-vindo ao portal…
admin-landing-intro-help = Mostrado acima do catálogo. Aceita **negrito**, *itálico* e [links](https://…) — sem HTML.
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
admin-landing-seo-preview = Prévia de busca
admin-landing-seo-title-placeholder = Padrão: título do portal
admin-landing-seo-title-help = Define o título da aba e o og:title. Vazio usa o título padrão do portal.
admin-landing-seo-description = Descrição (meta description)
admin-landing-seo-description-placeholder = Resumo do portal para buscadores e redes sociais
admin-landing-seo-description-help = Usada na meta description e no og:description. Vazio usa o texto de introdução.
admin-landing-og-image = Imagem de compartilhamento (og:image)
admin-landing-og-image-help = Imagem mostrada ao compartilhar o portal em redes sociais. Em branco, usa o logo do cabeçalho (à esquerda) ou a marca Ruscker. Para melhor resultado, suba um PNG/JPG ~1200×630 (alguns sites não renderizam SVG).
admin-landing-analytics = Analytics
admin-landing-analytics-html = Snippet de analytics
admin-landing-analytics-html-help = HTML inserido no <head> da landing (ex.: tag <script> do Plausible/Matomo/GA). Renderizado sem escape — use só fontes confiáveis.
admin-landing-analytics-origins = Origens permitidas (CSP)
admin-landing-analytics-origins-help = Domínios separados por espaço (ex.: https://plausible.io) liberados na CSP da landing para o script carregar e reportar.
admin-landing-analytics-provider = Provedor
admin-landing-provider-none = Nenhum
admin-landing-analytics-key = Chave do site
admin-landing-analytics-key-help = ID de medição do GA4 (G-XXXX), domínio do Plausible, ou URL|siteId do Matomo.


# Admin audit log
admin-audit-title = Histórico de Atividades Administrativas
admin-audit-subtitle = Visualização de atividades administrativas, do mais recente para o mais antigo. Limite de 100 eventos por consulta.
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
admin-audit-col-when = Quando
admin-audit-col-action = Ação
admin-audit-col-target = Alvo
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
status-title-new = Atualizado recentemente
status-title-updated = Atualizado
status-title-none = Sem data de atualização
sort-label = Ordenar
sort-recent = Recentes
sort-name = Nome
search-shortcut = ⌘ K

filter-subject-label = Assunto
filter-subject-all = Todos os assuntos
filter-status-label = Situação
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
admin-import-ok-assets = { $creds } credencial(is) e { $logos } imagem(ns) importadas para o painel.
admin-import-drop = Arraste o application.yml aqui ou clique para selecionar
admin-table-search = Buscar…
admin-table-no-results = Nenhum resultado
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
spec-form-choose-image = Escolher imagem
spec-form-cover-auto = Auto (cor do tipo)
spec-form-cover-auto-help = Usa o tom padrão do tipo do card. Escolha Sólida ou Gradiente para personalizar.
spec-form-cover-legacy-help = Esta capa usa uma imagem (modo descontinuado — o logo já fica por cima da capa). Ela é mantida como está; escolha Auto, Sólida ou Gradiente acima para substituí-la.

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
spec-form-max-replicas = Máx. de containers
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
spec-help-max-replicas = Teto rígido — o máximo de containers que o Ruscker roda para este app (o auto-scaler sobe até ele). Vazio = o padrão (5, ou as réplicas iniciais se for maior).
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
spec-form-error-network = Nome de rede Docker inválido (precisa começar com letra ou número, depois letras/números/_/./-).
spec-form-error-env = Cada variável de ambiente deve ser NOME=valor, com NOME válido (letras, números, _; começando por letra ou _). Corrija ou remova a linha inválida.
admin-nav-logs = Logs
admin-proclog-title = Auditoria de Logs
admin-proclog-subtitle = Visualização dos logs de eventos do balanceador e das réplicas.
admin-proclog-unavailable = Buffer de log não disponível (o servidor iniciou sem a camada de logging).
admin-proclog-empty = Nenhum log capturado ainda neste nível. Novos eventos aparecem aqui conforme acontecem; rode o servidor com -v para incluir logs de nível info.

# ── spec-form advanced params (#211/#212) ──────────────────────────
spec-form-runtime-section = Runtime
spec-form-container-port = Porta do container
spec-help-container-port = Porta em que o app escuta dentro do container. Em branco = padrão por tipo (3838 para Shiny). Defina para Streamlit (8501), Dash (8050) ou Jupyter (8888).
spec-form-platform = Plataforma
spec-help-platform = Plataforma Docker (ex.: linux/amd64) para rodar imagens de outra arquitetura via emulação. Em branco = o daemon escolhe pelo manifesto.
spec-form-container-network = Rede Docker
spec-help-container-network = Rede Docker à qual anexar o container (criada se não existir). Em branco = a bridge padrão do daemon. Use para isolar os containers deste app na própria rede.
spec-form-container-lifetime = Vida útil do container (min)
spec-help-container-lifetime = Limite suave em minutos antes de reciclar o container. Em branco = sem limite suave.
spec-form-stop-on-logout = Parar ao sair (logout)
spec-help-stop-on-logout = Para o container do usuário quando ele faz logout. Desligado por padrão.
spec-form-env-section = Ambiente + comando
spec-form-container-env = Variáveis de ambiente
spec-form-container-env-help = Uma por linha, NOME=valor. Para segredos, referencie uma variável de ambiente em vez de colar o valor.
spec-form-env-add = Adicionar
spec-form-env-value = valor
spec-form-env-remove = Remover
spec-form-env-empty = Nenhuma variável de ambiente.
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
spec-form-registry-password-keep = Em branco mantém a senha atual
spec-help-registry-password = Use uma variável de ambiente — nunca cole a senha em texto. Só usada junto com o usuário.
spec-form-registry-credential = Credencial salva
spec-form-registry-none = Nenhuma credencial salva. Para imagens privadas, crie uma na página Credenciais (link abaixo).
spec-form-registry-none-option = (nenhuma — imagem pública)
spec-form-registry-missing = removida
spec-help-registry-credential = Escolha uma credencial nomeada do cofre (página Credenciais) para puxar imagens privadas. Quando definida, tem precedência sobre o usuário/senha inline.
spec-form-registry-help = Puxe uma imagem privada selecionando uma credencial salva. Crie e gerencie credenciais na página Credenciais — a senha pode ser literal (criptografada) ou uma referência a variável de ambiente.
spec-form-registry-inline-note = Este app tem credenciais de registry inline (YAML importado). Elas são preservadas, mas prefira uma credencial salva acima.
spec-form-access-section = Acesso
spec-form-access-groups = Grupos permitidos
spec-help-access-groups = Grupos que podem ver e acessar o app (separados por vírgula). Em branco, com usuários também em branco = aberto a todos.
spec-form-access-users = Usuários permitidos
spec-help-access-users = Usuários que podem ver e acessar o app (separados por vírgula).
spec-form-access-help = Ambos em branco = card aberto a todos (inclusive anônimos). Com algum valor, só usuários logados que combinam — e admins sempre.
spec-form-require-mfa = Exigir 2FA
spec-form-require-mfa-hint = Usuários sem um fator TOTP configurado serão orientados a cadastrá-lo no primeiro acesso a um app protegido.
spec-form-mfa-validity = Solicitar novamente após N dias
spec-form-mfa-validity-hint = Em branco = 7 dias. Use 0 para exigir nova prova em cada sessão de login, sem dispositivo lembrado.
spec-form-mfa-staged-note = A exigência de 2FA chega em uma próxima versão; por enquanto, este app ainda não está protegido.
spec-form-identity-headers = Enviar cabeçalhos de identidade ao app
spec-form-identity-headers-hint = Adiciona X-SP-UserId e X-SP-UserGroups para usuários autenticados. Desativado por padrão; ative apenas para apps que precisam e confiam nessa identidade.
spec-form-identity-claims = Dados adicionais de identidade
spec-form-identity-claims-hint = Envia somente os dados de perfil selecionados a este app. Estes dados são independentes dos cabeçalhos de identidade X-SP.
spec-form-identity-claim-email = E-mail
spec-form-identity-claim-setor = Setor / unidade
spec-form-access-public = Público
spec-form-access-add-group = + add. grupo
spec-form-access-public-hint = vazio = visível a todos
spec-form-summary-replicas = réplicas
spec-form-summary-sessions = sessões por réplica
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
spec-form-scale-down-cooldown = Cooldown pós-scale-down (s)
spec-help-scale-down-cooldown = Segundos sem scale-up por saturação após recolher uma réplica. Em branco = 60; 0 desativa.
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

# ── spec-form image picker (#213) ──────────────────────────────────
spec-form-logo-upload = Enviar imagem
spec-form-gallery-more = Mostrar mais
spec-form-logo-clear = Remover
spec-form-logo-none = Sem imagem — usa um tom gerado pelo tipo.
spec-form-logo-builtin = Logos integrados
spec-form-logo-path-advanced = Avançado: colar um caminho ou URL
spec-form-cover-image = Imagem
spec-form-cover-image-help = Escolha uma imagem da biblioteca (ou envie uma) como fundo do card.
admin-proclog-tail-note = Mostrando as linhas mais recentes
admin-proclog-download = Baixar log completo
admin-proclog-filter-level = Filtrar por nível
admin-proclog-filter-all = Todos os níveis
admin-proclog-search = Buscar nos logs…
admin-proclog-pause = Pausar
admin-proclog-resume = Retomar
admin-proclog-clear = Limpar
admin-proclog-lines = linhas
admin-proclog-filter-app-all = Todos os apps

landing-empty = Nada por aqui.

admin-landing-style = CSS customizado
admin-landing-card-content = Conteúdo
admin-landing-card-meta = SEO e Analytics
admin-landing-card-header-desc = Título e subtítulo exibidos no topo do portal.
admin-landing-card-content-desc = O texto introdutório do portal, geral e por idioma.
admin-landing-card-meta-desc = Metadados de busca/compartilhamento e o snippet de analytics.
admin-landing-card-style-desc = CSS customizado, injetado por último (escape hatch).
admin-landing-custom-css = CSS personalizado
admin-landing-custom-css-help = Injetado no final do portal público. Use com cuidado.
admin-landing-logos-help = Adicione logos ao cabeçalho ou rodapé. À esquerda: substitui a marca Ruscker (cabeçalho) ou fica no canto esquerdo (rodapé). À direita: depois dos botões (cabeçalho) ou ao lado da versão (rodapé). Ao centro: barra separada. Vários no mesmo alinhamento ficam lado a lado.
admin-landing-logo-header = Cabeçalho
admin-landing-logo-footer = Rodapé
admin-landing-logo-left = Esquerda
admin-landing-logo-center = Centro
admin-landing-logo-right = Direita
admin-landing-logo-link = Link (opcional)
admin-landing-logo-height = Altura (px)
admin-landing-logo-margin = Margem (px)
admin-landing-logos-card = Logotipos
admin-landing-logo-main = Logo principal (cabeçalho)
admin-landing-logo-main-help = "Marca + nome" usa o símbolo do Ruscker; "Só símbolo" oculta o título; "Personalizado" troca o símbolo pela sua imagem. Tamanho e margem abaixo valem para os dois casos.
admin-landing-logos-extra = Logos adicionais
admin-landing-logos-extra-help = Logos extras no centro/direita do cabeçalho ou no rodapé (parceiros, instituições). A esquerda do cabeçalho é do logo principal.
admin-landing-header-style-card = Estilo do cabeçalho
admin-landing-bgmode-preset = Predefinição
admin-landing-bgmode-help = Predefinição usa os estilos prontos; Sólida e Gradiente pintam um fundo personalizado que substitui a predefinição.
admin-landing-header-dark-inherit = herda do claro
admin-landing-header-dark-inherit-help = Em branco, o tema escuro usa o mesmo fundo do claro.
admin-landing-cards-card = Cards do catálogo
admin-landing-theme-card = Tema e cores
admin-landing-logo-image = Imagem
admin-landing-logo-slot-label = Posição
admin-landing-logo-align-label = Alinhamento
admin-landing-logo-add = Adicionar logo

# — Gestão de disco (admin) #453
admin-nav-disk = Disco
admin-disk-title = Gestão do Disco
admin-disk-subtitle = Monitoramento do disco e recuperação de espaço ocioso de containers parados e imagens não usadas.
admin-disk-backend-missing = O backend Docker não está conectado — inicie o servidor com `--docker` para gerenciar disco.
admin-disk-containers-heading = Containers do Ruscker
admin-disk-prune = Remover parados
admin-disk-prune-confirm = Remover todos os containers parados do Ruscker?
admin-disk-no-containers = Nenhum container gerenciado pelo Ruscker.
admin-disk-containers-unavailable = Não foi possível carregar o inventário de containers do Docker. Esta é uma visão parcial; tente novamente quando o Docker responder.
admin-disk-col-container = Container
admin-disk-col-app = App
admin-disk-col-image = Imagem
admin-disk-col-status = Status
admin-disk-running = em execução
admin-disk-remove = Remover
admin-disk-remove-confirm = Remover este container?
admin-disk-remove-running-confirm = Este container está em execução. Pará-lo e removê-lo?
admin-disk-images-heading = Imagens
admin-disk-images-total = Total
admin-disk-used = Usado
admin-disk-free = livres
admin-disk-seg-images = Imagens Ruscker
admin-disk-seg-other = Outro uso
admin-disk-seg-free = Livre
admin-disk-images-note = O total pode contar camadas compartilhadas mais de uma vez. Só imagens não usadas podem ser removidas (sem forçar).
admin-disk-no-images = Nenhuma imagem local.
admin-disk-images-unavailable = Não foi possível carregar o inventário de imagens do Docker. Esta é uma visão parcial; tente novamente quando o Docker responder.
admin-disk-col-id = ID
admin-disk-col-size = Tamanho
admin-disk-col-usage = Uso
admin-disk-used-by-spec = usada por um app
admin-disk-used-by-container = usada por um container
admin-disk-unused = não usada
admin-disk-usage-unknown = Não foi possível consultar os containers em execução no Docker — toda imagem aparece como em uso e a remoção fica desabilitada para evitar apagar uma imagem ainda em uso. Tente de novo quando o Docker responder.
admin-disk-in-use-hint = Em uso — não pode ser removida.
admin-disk-foreign = não gerida
admin-disk-foreign-hint = Não é imagem do Ruscker (ex. outro app neste host) — o Ruscker não vai removê-la.
admin-disk-remove-image-confirm = Remover esta imagem?
admin-disk-flash-removed = Removido.
admin-disk-flash-pruned = Containers parados removidos.
admin-disk-flash-nothing = Nada a remover.
admin-disk-flash-error = A operação falhou. Veja os logs.
admin-disk-prune-images = Remover não usadas
admin-disk-prune-images-confirm = Remover todas as imagens não usadas?
admin-disk-flash-images-pruned = Imagens não usadas removidas.
admin-disk-cleaning = Limpando…
admin-disk-word-images = imagens
admin-disk-word-containers = containers
admin-disk-word-stopped = parados
admin-disk-badge-inuse = em uso
admin-disk-volumes-title = Volumes
admin-disk-volumes-hint = Volumes Docker nomeados neste host. Só volumes criados pelo Ruscker e sem nenhuma referência podem ser removidos.
admin-disk-volumes-create = Criar
admin-disk-volumes-name-placeholder = nome-do-volume
admin-disk-volumes-empty = Nenhum volume nomeado.
admin-disk-volumes-unavailable = O inventário de volumes não pôde ser carregado (ou este backend não gerencia volumes). Esta é uma visão parcial; tente de novo quando o Docker responder.
admin-disk-volumes-locked = Em uso, referenciado por um app ou não criado pelo Ruscker — não será removido por aqui.
admin-disk-volumes-badge-ruscker = Ruscker
admin-disk-volumes-badge-external = externo
admin-disk-volumes-confirm-remove = Remover este volume? Os DADOS dele serão apagados definitivamente — não dá para desfazer.
admin-disk-volumes-col-name = Volume
admin-disk-volumes-col-driver = Driver
admin-disk-volumes-col-created = Criado
admin-disk-volumes-col-refs = Referências
admin-disk-volumes-col-origin = Origem
admin-disk-flash-volume-created = Volume criado.
admin-disk-flash-volume-removed = Volume removido.
admin-disk-flash-volume-bad-name = Nome de volume inválido — use letras, dígitos, "_", "." ou "-", começando com letra ou dígito.
admin-dashboard-metric-sessions-help = Sessões que as réplicas reportam atendendo agora.
admin-dashboard-metric-tracker-help = Sessões fixas (sticky) rastreadas pelo proxy no heartbeat.
admin-landing-header = Cabeçalho
admin-landing-portal-title = Título do portal
admin-landing-portal-title-help = Aparece no topo da landing. Em branco, usa o título do config (proxy.title).
admin-landing-portal-subtitle = Subtítulo
admin-landing-portal-subtitle-help = A linha abaixo do título. Em branco, o subtítulo fica oculto.
admin-landing-footer = Rodapé
admin-landing-footer-help = Texto no rodapé do portal. Em branco, mostra a versão e a marca.
admin-landing-default-theme = Tema padrão
admin-landing-default-theme-help = O tema inicial para quem nunca escolheu. O visitante ainda pode trocar.
admin-landing-visible-sections = Seções visíveis
admin-landing-show-search = Barra de busca
admin-landing-brand-color = Cor da marca
admin-landing-brand-custom = Cor personalizada
admin-landing-brand-color-help = Atalho para o acento (claro e escuro). Ajuste fino abaixo.
admin-landing-logo-mode-mark = Marca + nome
admin-landing-logo-mode-symbol = Só símbolo
admin-landing-logo-mode-custom = Personalizado
admin-landing-logo-size = Tamanho do logo
admin-landing-header-bg-preset = Estilo do cabeçalho
admin-landing-header-bg-preset-help = Uma cor de fundo personalizada (em Aparência) sobrescreve esta predefinição.
admin-landing-preset-flat = Plano
admin-landing-preset-soft = Suave
admin-landing-preset-bold = Forte
admin-landing-card-cover-default = Capa padrão dos cards
admin-landing-cover-auto = Auto
admin-landing-cover-auto-sub = cor do tipo
admin-landing-cover-own = Próprio
admin-landing-cover-inherited = Herdado
admin-landing-cover-inherits-line = Herda o fundo do tema claro.
admin-landing-card-cover-default-auto-help = Cada card usa um tom da cor do seu tipo. Sem configuração — adapta-se automaticamente.
admin-landing-catalog-layout = Layout do catálogo
admin-landing-layout-grid = Grade
admin-landing-layout-list = Lista
admin-landing-layout-sections = Seções
admin-landing-density-comfortable = Confortável
admin-landing-density-compact = Compacto






admin-landing-theme-colors = Cores por tema
admin-landing-theme-colors-help = Recolore o tema claro e escuro do portal público. Em branco, mantém o padrão.
admin-landing-theme-light = Tema claro
admin-landing-theme-dark = Tema escuro
admin-landing-theme-bg = Fundo
admin-landing-theme-text = Texto
admin-landing-theme-accent = Acento

# Featured carousel (#506)
highlights-title = Destaques
card-favorite = Favoritar
spec-form-featured = Destacar este app
spec-form-featured-help = Mostra o app no carrossel de Destaques no topo da landing (quando a opção estiver ligada).
admin-landing-show-highlights = Mostrar Destaques
admin-landing-show-highlights-help = Exibe o carrossel de apps em destaque acima dos filtros. Some se nada estiver em destaque.

# Groups page (#503, read-only)
admin-nav-groups = Grupos
admin-groups-title = Gestão de Grupos
admin-groups-subtitle = Criação e edição dos grupos derivados dos apps e usuários.
admin-groups-members = Membros
admin-groups-apps = Apps
admin-groups-public-title = Apps públicos
admin-groups-public-help = Sem grupo — visível a todos
admin-groups-rename = Renomear grupo
admin-groups-rename-prompt = Novo nome do grupo:
admin-groups-delete = Excluir grupo
admin-groups-delete-confirm = Excluir este grupo? Ele será removido de todos os usuários e apps que o usam.
admin-groups-remove-member = Remover do grupo
admin-groups-remove-member-confirm = Remover este membro do grupo?
admin-groups-add-member = Adicionar membro
admin-groups-pick-user = Escolher usuário…
admin-groups-create = Criar grupo
admin-groups-new-name = Nome do grupo
admin-groups-new-group-title = Novo grupo:
admin-groups-flash-renamed = Grupo renomeado.
admin-groups-flash-deleted = Grupo excluído.
admin-groups-flash-member-added = Membro adicionado.
admin-groups-flash-member-removed = Membro removido.
admin-groups-flash-bad-input = Entrada inválida (nome vazio ou usuário inexistente).
admin-groups-empty = Nenhum grupo ainda. Grupos aparecem quando você define access-groups num app ou grupos num usuário.
admin-groups-no-members = Sem membros
admin-groups-no-apps = Nenhum app restrito a este grupo

highlights-prev = Anteriores
highlights-next = Próximos

# Featured star toggle in the Apps table (#521)
admin-specs-col-featured = Destaque
admin-specs-featured-on = Destacado — clique para remover
admin-specs-featured-off = Não destacado — clique para destacar
admin-specs-featured-readonly = Destaque definido no arquivo de config

# Importacao seletiva (#557)
admin-import-preview-title = Confirmar importação
admin-import-preview-help = Marque quais apps importar
admin-import-apps-label = apps
admin-import-warnings-label = avisos
admin-import-preview-none = O arquivo não contém nenhum app.
admin-import-select-all = Selecionar todos
admin-import-col-status = Situação
admin-import-badge-new = Novo
admin-import-badge-new-help = Será criado (não existe no painel)
admin-import-badge-update = Atualiza
admin-import-badge-update-help = Sobrescreve um app já existente no painel
admin-import-confirm = Importar selecionados
admin-import-load-file = Carregar arquivo
admin-import-editor-placeholder = Cole seu application.yml aqui…
admin-import-editor-empty = O preview aparece aqui conforme você digita.

# Aba Sistema do admin (#766)
admin-nav-system = Sistema
admin-system-title = Sistema
admin-system-subtitle = Diagnóstico read-only do servidor em execução.
admin-system-version = Versão do Ruscker
admin-system-base-path = Caminho base
admin-system-bind = Endereço de escuta
admin-system-docker = Docker
admin-system-db = Banco de dados
admin-system-specs = Apps no catálogo
admin-system-replicas = Réplicas em execução
admin-system-forward-headers = Confiar em headers encaminhados
admin-system-metrics = Endpoint de métricas
admin-system-leader = Líder HA
admin-system-draining = Drenando
admin-system-yes = sim
admin-system-no = não
admin-system-restart-title = Reiniciar o serviço
admin-system-restart-hint = O Ruscker não se reinicia com segurança sozinho — rode isto no host (as requisições em voo drenam no SIGTERM).
admin-system-alerts-title = Webhook de alertas
admin-system-alerts-hint = Eventos importantes (falha ao iniciar app, réplica caiu, app saturado no limite) são enviados como POST JSON para esta URL. Vazio = desligado.
admin-system-alerts-url = URL do webhook
admin-system-alerts-save = Salvar
admin-system-alerts-test = Enviar alerta de teste
admin-system-alerts-flash-saved = Webhook de alertas salvo.
admin-system-alerts-flash-bad-url = URL inválida — use http:// ou https:// (ou deixe vazio para desligar).
admin-system-alerts-flash-test = Alerta de teste enfileirado — confira o destino (a entrega tenta 3 vezes).
admin-system-alerts-flash-no-url = Configure e salve a URL do webhook antes de enviar um teste.

# Recuperar espaço no disco (#766)
admin-disk-reclaim = Recuperar espaço
admin-disk-reclaim-hint = Limpa imagens dangling + cache de build (seguro — nunca remove imagem nomeada nem container).
admin-disk-reclaim-confirm = Recuperar espaço? Limpa imagens dangling e o cache de build (nenhuma imagem nomeada ou container é removido).
admin-disk-flash-reclaimed = Espaço recuperado (imagens dangling + cache de build).

# Agendamentos — cron jobs (#986 fatia C)
admin-nav-schedules = Agendamentos
admin-schedules-title = Agendamentos
admin-schedules-subtitle = Executa a imagem de um app até o fim em um horário cron (ETL, relatórios).
admin-schedules-create = Novo agendamento
admin-schedules-spec = App
admin-schedules-cron = Cron
admin-schedules-cron-help = Cron padrão de 5 campos, em UTC. Exemplos: "0 3 * * *" = todo dia às 03:00; "*/15 * * * *" = a cada 15 minutos.
admin-schedules-cmd = Comando
admin-schedules-cmd-help = Um argumento por linha (argv). Vazio = o comando do próprio app (o container-cmd, senão o CMD da imagem).
admin-schedules-timeout = Timeout (minutos)
admin-schedules-timeout-help = Limite de duração de uma execução. Vazio = 1 hora.
admin-schedules-next-run = Próxima execução
admin-schedules-last-run = Última execução
admin-schedules-enabled = Ativo
admin-schedules-disabled = Inativo
admin-schedules-toggle = Ativar/desativar
admin-schedules-delete = Excluir
admin-schedules-confirm-delete = Excluir este agendamento? O histórico de execuções vai junto.
admin-schedules-empty = Nenhum agendamento ainda — crie um acima.
admin-schedules-runs-title = Últimas execuções
admin-schedules-runs-empty = Nenhuma execução ainda.
admin-schedules-runs-started = Início
admin-schedules-runs-status = Status
admin-schedules-runs-exit = Código de saída
admin-schedules-runs-duration = Duração
admin-schedules-log = Log
admin-schedules-flash-created = Agendamento criado. Dispara na próxima ocorrência do cron (não executa ao criar).
admin-schedules-flash-deleted = Agendamento excluído.
admin-schedules-flash-toggled = Agendamento atualizado.
admin-schedules-flash-bad-cron = Expressão cron inválida — use a forma de 5 campos, ex.: "0 3 * * *".
admin-schedules-flash-bad-spec = App desconhecido, ou o app não tem imagem de container para executar.
admin-schedules-flash-error = A operação falhou — verifique os logs do servidor.

# — TOTP / autenticação em dois fatores (#1005)
chrome-mfa = Autenticação em dois fatores
admin-mfa-title = Autenticação em dois fatores
admin-mfa-help = Proteja sua conta com códigos temporários de um aplicativo autenticador.
admin-mfa-error-password = A senha atual está incorreta.
admin-mfa-error-key = RUSCKER_MASTER_KEY não está configurada. Configure-a e reinicie o Ruscker antes de cadastrar o 2FA.
admin-mfa-break-glass = Sessões de emergência por token não têm uma conta nem senha e não podem configurar 2FA. Entre com usuário e senha.
admin-mfa-already = O 2FA já está configurado. Um administrador precisa redefini-lo antes de um novo cadastro.
admin-mfa-enrolled = 2FA configurado
admin-mfa-enrolled-since = Configurado desde
admin-mfa-reenroll-note = Para trocar de aplicativo autenticador, peça a um administrador para redefinir seu 2FA e cadastre-o novamente.
admin-mfa-not-enrolled = 2FA não configurado
admin-mfa-pending-note = Há um cadastro incompleto. Informe sua senha para começar novamente com uma nova chave.
admin-mfa-current-password = Senha atual
admin-mfa-start = Configurar 2FA
admin-mfa-setup-title = Vincule seu aplicativo autenticador
admin-mfa-setup-help = Leia o QR code no aplicativo e informe abaixo o código de 6 dígitos gerado por ele.
admin-mfa-error-rate = Muitas tentativas incorretas. Aguarde um minuto e tente novamente.
admin-mfa-error-code = Código incorreto ou expirado. Confira o relógio do dispositivo e tente novamente.
admin-mfa-manual-title = Chave para cadastro manual
admin-mfa-manual-help = Se não puder ler o QR code, digite esta chave no aplicativo autenticador.
admin-mfa-profile = Perfil: SHA-1, 6 dígitos, período de 30 segundos.
admin-mfa-code-label = Código de 6 dígitos
admin-mfa-confirm = Confirmar e ativar
admin-mfa-recovery-title = Salve seus códigos de recuperação
admin-mfa-recovery-warning = Estes códigos aparecem uma única vez. Copie ou guarde-os agora em um local seguro.
admin-mfa-recovery-help = Cada código pode ser usado somente uma vez caso você perca acesso ao aplicativo autenticador.
admin-mfa-continue = Continuar
admin-mfa-challenge-title = Verificar autenticação em dois fatores
admin-mfa-challenge-help = Informe um código para confiar neste navegador nos aplicativos protegidos.
admin-mfa-challenge-break-glass = Sessões de emergência por token não têm um fator do usuário. O acesso excepcional a aplicativos protegidos é tratado separadamente pela política.
admin-mfa-challenge-method = Método de verificação
admin-mfa-challenge-totp = Código do autenticador
admin-mfa-challenge-recovery = Código de recuperação
admin-mfa-challenge-code = Código
admin-mfa-challenge-submit = Verificar e continuar
admin-mfa-challenge-error = Código incorreto ou já utilizado. Tente novamente.
admin-mfa-challenge-replayed = Este código do autenticador já foi utilizado. Aguarde o próximo código.
admin-mfa-forget-device = Esquecer este dispositivo
admin-mfa-forget-confirm = Esquecer a comprovação de MFA armazenada neste navegador?
admin-mfa-revoke-all = Esquecer todos os dispositivos
admin-mfa-revoke-all-confirm = Esquecer todos os dispositivos confiáveis? Cada navegador precisará provar o 2FA de novo no próximo acesso a um app protegido. As sessões de login continuam ativas.
admin-users-mfa-section = Autenticação em dois fatores
admin-users-mfa-configured = 2FA configurado desde
admin-users-mfa-reset-hint = A redefinição apaga a chave e todos os códigos de recuperação. O usuário precisará cadastrar o 2FA novamente.
admin-users-mfa-reset-confirm = Redefinir o 2FA deste usuário? A chave e TODOS os códigos de recuperação serão apagados imediatamente.
admin-users-mfa-reset = Redefinir 2FA
admin-users-mfa-not-configured = 2FA não configurado
admin-users-mfa-reset-ok = O 2FA e os códigos de recuperação foram redefinidos.
