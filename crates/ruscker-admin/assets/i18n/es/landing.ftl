### Landing page — es-ES

landing-title = Monitoreo Estratégico
landing-subtitle = Portal de aplicaciones y APIs

filter-search-placeholder = Buscar aplicación…
filter-access-all = Todos
filter-access-public = Públicos
filter-access-restricted = Restringidos
filter-clear = Limpiar filtros

type-all = Todos
type-app = Aplicaciones
type-package = Paquetes
type-talk = Presentaciones
type-report = Informes
type-api = APIs

type-app-abbr = APP
type-talk-abbr = PRS
type-report-abbr = INF
type-package-abbr = PKG
type-api-abbr = API
type-link-abbr = LNK

# Admin shell
admin-nav-apps = Aplicaciones
admin-nav-images = Imágenes
admin-nav-credentials = Credenciales
admin-nav-landing = Portal
admin-nav-audit = Auditoría
admin-nav-portal = Volver al portal
admin-nav-logout = Salir

# Admin login
admin-login-title = Acceso admin
admin-login-help = Ingrese el token de admin definido en RUSCKER_ADMIN_TOKEN.
admin-login-token-label = Token
admin-login-token-placeholder = Pegue el token aquí
admin-login-submit = Entrar
admin-login-error-wrong = Token incorrecto. Intente de nuevo.
admin-login-back-portal = ← portal público

# Apps list
admin-specs-title = Aplicaciones
admin-specs-subtitle = Catálogo de specs en la base de datos
admin-specs-empty = Aún no hay aplicaciones. Use { $cmd } para importar desde YAML.
admin-specs-add = Añadir aplicación
admin-specs-col-id = ID
admin-specs-col-name = Nombre
admin-specs-col-kind = Tipo
admin-specs-col-state = Estado
admin-specs-col-updated = Actualizado
admin-specs-col-version = Versión
admin-specs-col-actions = Acciones
admin-specs-filter-search = Buscar por id o nombre…
admin-specs-filter-kind-all = Todos los tipos
admin-specs-filter-state-all = Activos e inactivos
admin-specs-edit = Editar
admin-specs-delete = Borrar

# Spec form (new / edit)
spec-form-title-new = Nueva aplicación
spec-form-crumb-new = Nueva
spec-form-crumb-edit = Editar
spec-form-cancel = Cancelar
spec-form-save = Guardar cambios
spec-form-kind = Tipo
spec-form-kind-app = App contenedor
spec-form-kind-talk = Presentación
spec-form-kind-report = Informe
spec-form-kind-package = Paquete
spec-form-kind-api = API
spec-form-kind-link = Enlace externo
spec-form-identity = Identidad
spec-form-id = ID
spec-form-id-help-new = Elegido por el operador. Aparece en /app/<id>/.
spec-form-id-help-edit = El ID es inmutable después de creado.
spec-form-name = Nombre visible
spec-form-desc = Descripción
spec-form-visual = Visual
spec-form-logo = Logo del card
spec-form-logo-help = URL o ruta /assets/img/foo.png. Ver docs/IMAGES.md.
spec-form-access = Acceso
spec-form-state = Estado
spec-form-state-active = Activo
spec-form-state-inactive = Inactivo
spec-form-tema = Tema
spec-form-container = Contenedor
spec-form-image = Imagen Docker
spec-form-seats = Sesiones/contenedor
spec-form-lifetime = Vida máx. (min)
spec-form-lifetime-help = 360 = 6 horas
spec-form-link-section = Enlace externo
spec-form-link = URL destino
spec-form-meta = Metadatos
spec-form-updated = Actualizado el
spec-form-updated-help = Vacío para usar la fecha de hoy.
spec-form-preview = Vista previa
spec-form-preview-help = Se actualiza en vivo.
spec-form-actions = Acciones
spec-form-delete = Eliminar aplicación
spec-form-delete-confirm = ¿Está seguro? Esto no se puede deshacer.

spec-form-error-id-required = El ID es obligatorio.
spec-form-error-id-shape = El ID debe empezar con una letra y contener solo letras, números, "_" y "-".
spec-form-error-id-duplicate = Ya existe una aplicación con ese ID.
spec-form-error-name-required = El nombre visible es obligatorio.

# Admin image library
admin-images-title = Biblioteca de imágenes
admin-images-subtitle = PNG, JPEG y WebP se convierten a WebP. SVG pasa directo.
admin-images-drop-here = Haga clic para elegir un archivo
admin-images-formats = PNG · JPEG · WebP · SVG · hasta 10 MB
admin-images-upload = Subir
admin-images-uploaded = Imagen subida:
admin-images-empty = Aún no hay imágenes. Suba la primera arriba.
admin-images-delete = Eliminar
admin-images-delete-confirm = ¿Eliminar esta imagen? Las specs que referencian el archivo mostrarán el cover tintado.

# Admin credentials
admin-creds-title = Credenciales del registry
admin-creds-subtitle = Las contraseñas se cifran en reposo con AES-256-GCM. Nunca aparecen en el YAML ni en el panel después de guardarlas.
admin-creds-form-title = Añadir / actualizar credencial
admin-creds-name = Nombre
admin-creds-name-help = Identificador único. Use el mismo nombre en sus specs.
admin-creds-registry = Registry
admin-creds-username = Usuario
admin-creds-password = Contraseña / token
admin-creds-password-help = Cifrada al guardar; no se muestra de nuevo.
admin-creds-save = Guardar credencial
admin-creds-saved = Credencial guardada:
admin-creds-empty = No hay credenciales aún.
admin-creds-delete = Borrar
admin-creds-delete-confirm = ¿Borrar esta credencial?
admin-creds-col-name = Nombre
admin-creds-col-registry = Registry
admin-creds-col-username = Usuario
admin-creds-col-created = Creada
admin-creds-key-missing-title = RUSCKER_MASTER_KEY no está configurada
admin-creds-key-missing-help = El store de credenciales necesita una clave de 32 bytes en hex (64 chars) o base64 (44 chars). Genere una con:

# Admin landing editor
admin-landing-title = Editor del portal
admin-landing-crumb = Ajustes · Landing page
admin-landing-subtitle = Personalice el portal público. Los cambios surten efecto al refrescar.
admin-landing-open-portal = Abrir portal
admin-landing-save = Guardar
admin-landing-saved = Ajustes guardados. Recargue el portal para ver los cambios.
admin-landing-colors = Colores del encabezado
admin-landing-header-bg = Color de fondo
admin-landing-bg-help = Vacío = usa el color predeterminado del tema (claro/oscuro).
admin-landing-header-fg = Color del texto
admin-landing-clear = Limpiar
admin-landing-intro = Texto de introducción (predeterminado)
admin-landing-intro-default = Predeterminado (fallback para todos los idiomas)
admin-landing-intro-default-placeholder = Bienvenido al portal…
admin-landing-intro-help = Se muestra entre el encabezado y los filtros. Vacío = sin texto.
admin-landing-intro-locales = Texto de introducción por idioma
admin-landing-intro-pt = Portugués
admin-landing-intro-en = Inglés
admin-landing-intro-es = Español
admin-landing-intro-fr = Francés
admin-landing-preview = Vista previa
admin-landing-preview-help = Aproximación visual del encabezado y la introducción. Cards y filtros como en la landing real.
admin-landing-preview-empty = (sin texto de introducción)
admin-landing-future-title = Próximamente
admin-landing-future-help = Editor de logos, reordenación de secciones, bloques HTML custom, SEO/analytics y meta tags. Por ahora siguen el YAML.

card-cta-open = Abrir
card-cta-link = Visitar
card-cta-open-app = Abrir aplicación
card-cta-open-talk = Abrir presentación
card-cta-open-report = Abrir informe
card-cta-open-package = Abrir documentación
card-cta-open-api = Ver documentación
card-updated = Actualizado el { $date }
status-new = nuevo { $date }
status-updated = actualizado { $date }
sort-label = Ordenar
sort-recent = Recientes
sort-name = Nombre
search-shortcut = ⌘ K

filter-theme-label = Tema
filter-theme-all = Todos los temas
filter-status-active = Solo activos
filter-status-all = Activos e inactivos
filter-status-inactive-only = Solo inactivos
card-state-active = Disponible
card-state-inactive = No disponible
card-access-public = Acceso público
card-access-restricted = Acceso restringido

footer-language = Idioma
footer-theme = Tema
theme-light = Claro
theme-dark = Oscuro
theme-auto = Automático
