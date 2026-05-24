### Landing page — pt-BR
### Português brasileiro. Idioma de referência (fallback dos demais).

landing-title = Monitoramento Estratégico
landing-subtitle = Portal de aplicações e APIs

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
admin-nav-apps = Aplicações
admin-nav-images = Imagens
admin-nav-credentials = Credenciais
admin-nav-landing = Portal
admin-nav-audit = Auditoria
admin-nav-portal = Voltar ao portal
admin-nav-logout = Sair

# Admin login
admin-login-title = Acesso ao admin
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
spec-form-access = Acesso
spec-form-state = Estado
spec-form-state-active = Ativo
spec-form-state-inactive = Inativo
spec-form-tema = Tema
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

# Admin image library
admin-images-title = Biblioteca de imagens
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
admin-landing-future-title = Em breve
admin-landing-future-help = Editor de logos, reordenação de seções, blocos HTML customizados, SEO/analytics e meta tags. Por enquanto, esses campos seguem o YAML.

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
admin-audit-target-placeholder = Buscar por alvo (ex: spec:auroraprime)
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

filter-theme-label = Tema
filter-theme-all = Todos os temas
filter-status-active = Apenas ativos
filter-status-all = Ativos e inativos
filter-status-inactive-only = Apenas inativos
card-state-active = Disponível
card-state-inactive = Indisponível
card-access-public = Acesso público
card-access-restricted = Acesso restrito

footer-language = Idioma
footer-theme = Tema
theme-light = Claro
theme-dark = Escuro
theme-auto = Automático
