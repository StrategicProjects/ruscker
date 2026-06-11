### Landing page — es-ES

landing-title = Ruscker
landing-subtitle = Portal de aplicaciones y APIs
landing-signin = Entrar
landing-panel = Panel
landing-signout = Salir
landing-signed-in-as = { $user }

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
admin-nav-dashboard = Panel
admin-nav-apps = Aplicaciones
admin-nav-images = Multimedia
admin-nav-credentials = Credenciales
admin-nav-landing = Apariencia
admin-nav-blocks = Bloques
admin-blocks-title = Bloques HTML
admin-blocks-subtitle = Fragmentos de HTML personalizado renderizados en la landing (slots arriba/abajo).
admin-blocks-new = Nuevo bloque
admin-blocks-empty = Aún no hay bloques.
admin-blocks-col-slot = Slot
admin-blocks-col-title = Título
admin-blocks-col-status = Estado
admin-blocks-enabled = activo
admin-blocks-disabled = inactivo
admin-blocks-edit = editar
admin-blocks-delete = eliminar
admin-blocks-delete-confirm = ¿Eliminar este bloque?
admin-blocks-move-up = Subir
admin-blocks-move-down = Bajar
admin-blocks-form-new = Nuevo bloque
admin-blocks-form-edit = Editar bloque
admin-blocks-slot = Slot
admin-blocks-slot-help = Dónde aparece el bloque en la landing.
admin-blocks-slot-top = Arriba (tras el encabezado)
admin-blocks-slot-bottom = Abajo (tras la cuadrícula)
admin-blocks-title-label = Título (interno)
admin-blocks-title-placeholder = Una etiqueta para reconocer el bloque
admin-blocks-html = HTML
admin-blocks-html-help = Se renderiza sin escapar en la landing — usa solo fuentes de confianza.
admin-blocks-origins = Orígenes permitidos (CSP)
admin-blocks-origins-help = Dominios separados por espacios permitidos en la CSP de la landing (p. ej. https://example.com).
admin-blocks-enabled-label = Activo (renderizar en la landing)
admin-blocks-save = Guardar
admin-blocks-cancel = Cancelar
admin-nav-audit = Auditoría
admin-nav-portal = Portal
admin-nav-logout = Salir
role-current = Tu nivel de acceso
role-viewer = Visor
role-editor = Editor
role-admin = Administrador

# — Inicio de sesión usuario/contraseña + bootstrap por token (#107)
admin-login-help-user = Inicia sesión con tu usuario y contraseña.
admin-login-username-label = Usuario
admin-login-username-placeholder = tu usuario
admin-login-password-label = Contraseña
admin-login-password-placeholder = tu contraseña
admin-login-error-credentials = Usuario o contraseña inválidos.
admin-login-use-token = Entrar con token de administrador
admin-login-use-password = Volver al inicio con contraseña
# — Configuración del primer admin
admin-setup-title = Crea la cuenta de administrador
admin-setup-help = Es la primera vez. Elige un usuario y una contraseña para el administrador.
admin-setup-error = No se pudo crear la cuenta. Revisa los datos.
admin-setup-password-label = Contraseña
admin-setup-submit = Crear administrador
# — Cambio de contraseña / primer acceso
admin-pw-title = Cambiar contraseña
admin-pw-help = Define una nueva contraseña para tu cuenta.
admin-pw-first-prompt = Estás usando una contraseña asignada por un administrador. Define una nueva contraseña para continuar.
admin-pw-current-label = Contraseña actual
admin-pw-new-label = Nueva contraseña
admin-pw-confirm-label = Confirmar contraseña
admin-pw-error-current = La contraseña actual es incorrecta.
admin-pw-error-mismatch = Las contraseñas no coinciden.
admin-pw-error-short = La contraseña debe tener al menos 8 caracteres.
admin-pw-submit = Guardar contraseña
admin-pw-reveal = Mostrar/ocultar contraseña
# — Gestión de usuarios (admin)
admin-nav-users = Usuarios
admin-users-title = Usuarios
admin-users-subtitle = Crea y gestiona quién accede al panel y con qué nivel.
admin-users-new = Nuevo usuario
admin-users-create = Crear
admin-users-initial-password = Contraseña inicial
admin-users-initial-password-hint = Mínimo 8 caracteres. El usuario deberá cambiarla en el primer acceso.
admin-users-role = Nivel
admin-users-col-user = Usuario
admin-users-col-role = Nivel
admin-users-col-created = Creado
admin-users-col-actions = Acciones
admin-users-you = tú
admin-users-must-change = Aún usa la contraseña inicial
admin-users-save-role = Guardar nivel
admin-users-groups = Grupos
admin-users-groups-placeholder = analistas, gestores
admin-users-groups-hint = Los grupos separados por comas controlan qué apps restringidas ve el usuario.
admin-users-col-groups = Grupos
admin-users-save-groups = Guardar grupos
admin-users-new-password = nueva contraseña
admin-users-reset-password = Restablecer contraseña
admin-users-delete = Eliminar usuario
admin-users-confirm-delete = ¿Eliminar este usuario?
admin-users-flash-created = Usuario creado.
admin-users-flash-saved = Cambios guardados.
admin-users-flash-deleted = Usuario eliminado.
admin-users-flash-last-admin = No se puede eliminar ni degradar al último administrador.
admin-users-flash-bad-input = Datos inválidos: el usuario solo puede tener letras, números y _ . @ - , y la contraseña necesita al menos 8 caracteres.
admin-users-username-rule = Solo letras, números y _ . @ - (sin espacios ni acentos).
admin-users-password-rule = Mínimo 8 caracteres.
admin-users-flash-exists = Ya existe un usuario con ese nombre.

# Admin dashboard
admin-dashboard-title = Panel de monitoreo
admin-dashboard-subtitle = Estado de contenedores y sesiones en tiempo real
admin-dashboard-live = En vivo
admin-dashboard-filter-search = Filtrar app…
admin-dashboard-metric-containers = Contenedores
admin-dashboard-metric-sessions = Sesiones activas
admin-dashboard-metric-specs = Aplicaciones con réplicas
admin-dashboard-metric-tracker = Sesiones rastreadas
admin-dashboard-replicas-heading = Réplicas activas
admin-dashboard-grouped-by = Agrupadas por aplicación
admin-dashboard-expand-all = Expandir todo
admin-dashboard-collapse-all = Contraer todo
admin-dashboard-no-replicas = Ninguna réplica en ejecución. Las réplicas aparecen aquí cuando el scaler garantiza el mínimo o una solicitud dispara un arranque en frío.
admin-dashboard-col-spec = Aplicación
admin-dashboard-col-state = Estado
admin-dashboard-col-uptime = Uptime
admin-dashboard-col-sessions = Sesiones
admin-dashboard-col-host = Host
admin-dashboard-col-container = Contenedor
admin-dashboard-col-cpu = CPU
admin-dashboard-col-memory = Memoria
admin-dashboard-metric-memory = Memoria usada
admin-dashboard-metrics-pending = esperando primera lectura
admin-dashboard-state-ready = listo
admin-dashboard-state-starting = iniciando
admin-dashboard-state-draining = drenando
admin-dashboard-state-stopped = detenido
admin-dashboard-state-failed = falló
admin-dashboard-backend-missing = El backend de Docker no está conectado — inicie el servidor con `--docker` para ver contenedores aquí.

# Admin login
admin-login-title = Iniciar sesión
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
admin-specs-col-access = Accesos
admin-specs-col-access-groups = Acceso
admin-specs-access-public = público
admin-specs-col-access-help = Total de accesos (visitas a apps + clics en tarjetas externas)
admin-specs-col-actions = Acciones
admin-specs-filter-search = Buscar por id o nombre…
admin-specs-kind-interactive = Interactivo
admin-specs-kind-external = Externo
admin-specs-filter-kind-all = Todos los tipos
admin-specs-filter-state-all = Activos e inactivos
admin-specs-edit = Editar
admin-specs-duplicate = Duplicar
admin-specs-config-badge = config
admin-specs-config-defined = Definido en el YAML — solo lectura aquí; edita el archivo
admin-specs-delete = Borrar
admin-specs-archive = Archivar — ocultar la tarjeta del portal
admin-specs-unarchive = Reactivar — volver a mostrar la tarjeta en el portal
admin-specs-delete-confirm = ¿Borrar esta aplicación? Sus contenedores se detendrán y la configuración se eliminará. Esta acción no se puede deshacer.

# Spec form (new / edit)
spec-form-title-new = Nueva aplicación
spec-form-crumb-new = Nueva
spec-form-crumb-edit = Editar
spec-form-cancel = Volver a aplicaciones
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
spec-form-visual = Apariencia
spec-form-logo = Logo del card
spec-form-logo-help = URL o ruta /assets/img/foo.png. Ver docs/IMAGES.md.
spec-form-logo-pick-help = O elige una imagen ya subida en la biblioteca de medios.
spec-form-state = Estado
spec-form-state-active = Activo
spec-form-state-inactive = Inactivo
spec-form-subject = Asunto
spec-form-container = Contenedor
spec-form-image = Imagen Docker
spec-form-image-check = Verificar
spec-form-image-checking = Verificando…
spec-form-image-present = En el servidor
spec-form-image-absent = Ausente — se descarga en el primer acceso
spec-form-image-unresolved = Tiene una variable de entorno — se resuelve al hacer pull
spec-form-image-no-backend = Docker no conectado — no se puede verificar
spec-form-image-error = Error al verificar la imagen
spec-form-image-pull = Descargar
spec-form-image-pulling = Descargando…
spec-form-seats = Sesiones/contenedor
spec-form-lifetime = Vida máx. (min)
spec-form-lifetime-help = 360 = 6 horas
spec-form-link-section = Enlace externo
spec-form-link = URL destino
spec-form-accent = Color de acento
spec-form-accent-help = Tiñe la portada del card (cuando no hay cover).
spec-form-monogram = Monograma
spec-form-monogram-ph = AB
spec-form-monogram-help = Se muestra en la portada cuando no hay logo.
spec-form-meta = Acceso y escala
spec-form-restricted = Acceso restringido
spec-form-restricted-hint = Requiere iniciar sesión para abrir
spec-form-initial-replicas = Réplicas iniciales
spec-form-autoscaling = Autoescalado
spec-form-autoscaling-hint = Escala réplicas según la demanda
spec-form-updated = Actualizado el
spec-form-updated-help = Vacío para usar la fecha de hoy.
spec-form-preview = Vista previa
spec-form-preview-help = Se actualiza en vivo.
spec-form-actions = Acciones
spec-form-delete = Eliminar aplicación
spec-form-delete-confirm = ¿Está seguro? Esto no se puede deshacer.

spec-form-env-key = CLAVE
spec-form-error-id-required = El ID es obligatorio.
spec-form-error-id-shape = El ID debe empezar con una letra y contener solo letras, números, "_" y "-".
spec-form-error-id-duplicate = Ya existe una aplicación con ese ID.
spec-form-error-name-required = El nombre visible es obligatorio.
spec-form-error-number = Un campo numérico tiene un valor no numérico.
spec-form-error-cpu = El límite de CPU debe ser un número positivo (ej.: 0.5).
spec-form-error-memory = El límite de memoria debe ser un tamaño como 512m o 1.5g.
spec-form-error-replica-range = Réplicas máx. debe ser mayor o igual que réplicas mín.
spec-form-error-stale = Otra persona guardó esta app mientras editabas. Revisa los valores actuales abajo y envía de nuevo.

# Admin image library
admin-images-title = Biblioteca multimedia
admin-images-subtitle = PNG, JPEG y WebP se convierten a WebP. SVG pasa directo.
admin-images-drop-here = Haga clic para elegir un archivo
admin-images-formats = PNG · JPEG · WebP · SVG · hasta 10 MB
admin-images-upload = Subir
admin-images-choose = Elegir imagen
admin-images-builtin = Logos integrados
admin-images-builtin-tag = integrado
admin-images-uploaded = Imagen subida:
admin-images-renamed = renombrada porque este nombre ya estaba en uso:
admin-images-rename = Renombrar
admin-images-rename-prompt = Nuevo nombre del archivo (se mantiene la extensión):
admin-images-rename-taken = Ya existe una imagen con ese nombre. Elige otro.
admin-images-rename-invalid = Nombre inválido.
admin-images-empty = Aún no hay imágenes. Suba la primera arriba.
admin-images-delete = Eliminar
admin-images-delete-confirm = ¿Eliminar esta imagen? Las specs que referencian el archivo mostrarán el cover tintado.
admin-images-inuse = En uso
admin-images-inuse-help = Usada en una tarjeta o un logo de la landing
admin-images-delete-confirm-inuse = Esta imagen está EN USO. Al eliminarla, los apps que la usan vuelven al logo predeterminado de Ruscker (la tarjeta no se rompe). ¿Eliminar?
admin-images-search = Buscar imágenes…
admin-images-type-all = Todos los tipos
admin-images-no-match = Ninguna imagen coincide con la búsqueda.

# Admin credentials
admin-creds-title = Credenciales del registry
admin-creds-subtitle = Las contraseñas se cifran en reposo con AES-256-GCM. Nunca aparecen en el YAML ni en el panel después de guardarlas.
admin-creds-form-title = Añadir / actualizar credencial
admin-creds-name = Nombre
admin-creds-name-help = Identificador único. Use el mismo nombre en sus specs.
admin-creds-registry = Registry
admin-creds-username = Usuario
admin-creds-password = Contraseña / token
admin-creds-password-help = Cifrada al guardar; no se muestra de nuevo. O indique una referencia a variable de entorno — resuelta en el pull y nunca almacenada.
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
admin-landing-title = Apariencia del portal
admin-landing-crumb = Ajustes · Landing page
admin-landing-subtitle = Configure cómo se ve el portal público para los visitantes.
admin-landing-scope-help = Estas opciones (colores, textos de introducción, SEO, bloques personalizados) se aplican a la portada pública en vivo — guardadas aquí, mostradas en la próxima visita, sin reinicio. Es un conjunto fijo de ajustes, no un editor de CSS arbitrario.
admin-landing-open-portal = Abrir portal
admin-landing-save = Guardar
admin-landing-reset = Restaurar predeterminado
admin-landing-reset-help = Devuelve la apariencia del portal a los valores originales
admin-landing-reset-confirm = ¿Restaurar la apariencia predeterminada? Colores, tema, estilo de cabecera, portadas y diseño vuelven al original. Títulos, logos, textos, SEO, CSS personalizado y bloques HTML se conservan.
admin-landing-saved = Ajustes guardados. Recargue el portal para ver los cambios.
admin-landing-header-bg = Color de fondo personalizado
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
admin-landing-seo = SEO y compartir
admin-landing-seo-title = Título de la página (SEO)
admin-landing-seo-preview = Vista previa de búsqueda
admin-landing-seo-title-placeholder = Predeterminado: título del portal
admin-landing-seo-title-help = Define el título de la pestaña y el og:title. Vacío usa el título por defecto del portal.
admin-landing-seo-description = Descripción (meta description)
admin-landing-seo-description-placeholder = Resumen para buscadores y redes sociales
admin-landing-seo-description-help = Se usa en la meta description y el og:description. Vacío usa el texto de introducción.
admin-landing-og-image = Imagen para compartir (og:image)
admin-landing-og-image-help = Imagen mostrada al compartir el portal en redes sociales. En blanco, usa el logo del encabezado (izquierda) o la marca Ruscker. Para mejor resultado, sube un PNG/JPG ~1200×630 (algunos sitios no renderizan SVG).
admin-landing-analytics = Analytics
admin-landing-analytics-html = Snippet de analytics
admin-landing-analytics-html-help = HTML inyectado en el <head> de la landing (p. ej. una etiqueta <script> de Plausible/Matomo/GA). Se renderiza sin escapar — usa solo fuentes de confianza.
admin-landing-analytics-origins = Orígenes permitidos (CSP)
admin-landing-analytics-origins-help = Dominios separados por espacios (p. ej. https://plausible.io) permitidos en la CSP de la landing para que el script cargue y reporte.
admin-landing-analytics-provider = Proveedor
admin-landing-provider-none = Ninguno
admin-landing-analytics-key = Clave del sitio
admin-landing-analytics-key-help = ID de medición de GA4 (G-XXXX), dominio de Plausible, o URL|siteId de Matomo.


# Admin audit log
admin-audit-title = Auditoría
admin-audit-subtitle = Todos los cambios del admin, del más reciente al más antiguo. Tope de 100 eventos por consulta.
admin-audit-family = Familia
admin-audit-family-all = Todas las acciones
admin-audit-family-spec = Aplicaciones
admin-audit-family-image = Imágenes
admin-audit-family-credential = Credenciales
admin-audit-family-landing = Portal
admin-audit-actor = Autor
admin-audit-actor-all = Todos los autores
admin-audit-target-placeholder = Buscar destino (ej: spec:sales-dashboard)
admin-audit-apply = Aplicar
admin-audit-col-when = Cuándo
admin-audit-col-action = Acción
admin-audit-col-target = Objetivo
admin-audit-empty = Aún no hay cambios — o el filtro no coincide con nada.
admin-audit-limit-hint = Mostrando los 100 más recientes. Afine el filtro para reducir.

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
status-title-new = Actualizado recientemente
status-title-updated = Actualizado
status-title-none = Sin fecha de actualización
sort-label = Ordenar
sort-recent = Recientes
sort-name = Nombre
search-shortcut = ⌘ K

filter-subject-label = Asunto
filter-subject-all = Todos los asuntos
filter-status-label = Estado
filter-status-active = Solo activos
filter-status-all = Activos e inactivos
filter-status-inactive-only = Solo inactivos
card-state-active = Disponible
card-state-inactive = No disponible
card-access-public = Acceso público
card-access-restricted = Acceso restringido

theme-light = Claro
theme-dark = Oscuro
theme-auto = Automático

# Top-right chrome cluster (#182 + #183)
chrome-cluster-label = Preferencias de la página
chrome-theme-label = Tema
chrome-language-label = Idioma
chrome-account-label = Cuenta
chrome-signin = Iniciar sesión
chrome-signed-in-as-prefix = Conectado como
chrome-panel = Panel
chrome-change-password = Cambiar contraseña
chrome-signout = Cerrar sesión

# Admin logs viewer
admin-logs-title = Logs del contenedor
admin-logs-back = Volver al panel
admin-logs-replica = Réplica
admin-logs-empty = Aún no hay salida de log para esta réplica.
admin-logs-tail-note = Mostrando las últimas líneas (más recientes al final).

# Dashboard replica actions
admin-dashboard-action-stop = Detener
admin-dashboard-action-restart = Reiniciar
admin-dashboard-confirm-stop = ¿Detener esta réplica? El auto-scaler puede recrearla si el mínimo configurado lo exige.
admin-dashboard-confirm-restart = ¿Reiniciar esta réplica? Se perderá la sesión activa.
admin-logs-follow = En vivo
admin-logs-follow-stop = Detener

# Admin YAML import
admin-import-button = Importar YAML
admin-import-title = Importar configuración YAML
admin-import-help = Elija un application.yml de ShinyProxy o Ruscker. El import es idempotente y no elimina specs existentes.
admin-import-file = Archivo .yml / .yaml
admin-import-submit = Importar
admin-import-cancel = Cancelar
admin-import-ok = Import completo: { $created } creados, { $updated } actualizados, { $unchanged } sin cambios.
admin-import-ok-warnings = { $warnings } advertencia(s) de validación — revise credenciales embebidas y nombres vacíos.
admin-import-ok-assets = { $creds } credencial(es) y { $logos } imagen(es) importadas al panel.
admin-import-drop = Arrastra el application.yml aquí o haz clic para seleccionar
admin-table-search = Buscar…
admin-table-no-results = Sin resultados
admin-import-err = Error en el import: { $msg }

# Gradient builder
admin-grad-solid = Sólido
admin-grad-gradient = Degradado
admin-grad-linear = Lineal
admin-grad-radial = Radial
admin-grad-add-stop = Agregar color
admin-grad-remove-stop = Quitar color

# Spec form — card cover
spec-form-cover = Cover de la tarjeta
spec-form-choose-image = Elegir imagen
spec-form-cover-auto = Auto (color del tipo)
spec-form-cover-auto-help = Usa el tono por defecto del tipo. Elija Sólido o Degradado para personalizar.
spec-form-cover-legacy-help = Esta portada usa una imagen (modo descontinuado — el logo ya va encima de la portada). Se mantiene como está; elija Auto, Sólido o Degradado arriba para reemplazarla.

# ── Formulario de spec: sección avanzada + ayuda por campo (#2) ────
spec-form-advanced = Avanzado
spec-form-advanced-hint = Todo opcional — déjalo vacío para mantener el valor por defecto.
spec-form-api-section = API
spec-form-scaling-section = Escalado
spec-form-resources-section = Recursos
spec-form-lifecycle-section = Ciclo de vida
spec-form-api-port = Puerto del contenedor
spec-form-api-rate-limit = Límite de tasa
spec-form-api-docs-path = Ruta de docs
spec-form-api-health-path = Ruta de health
spec-form-api-cors = Habilitar CORS permisivo
spec-form-min-replicas = Réplicas mín.
spec-form-max-replicas = Réplicas máx.
spec-form-concurrent = Solicitudes por réplica
spec-form-cpu-limit = Límite de CPU
spec-form-memory-limit = Límite de memoria
spec-form-heartbeat = Tiempo de heartbeat (ms)
spec-help-kind = Qué tipo de elemento es. Define el enrutamiento, la insignia de la tarjeta y si se inicia un contenedor.
spec-help-id = Identificador estable usado en la URL (/app/<id>). Minúsculas, dígitos, "-" y "_"; no se puede cambiar tras crearlo.
spec-help-name = El título mostrado en la tarjeta.
spec-help-desc = Descripción breve en la tarjeta. Se permite HTML en línea (p. ej. un enlace).
spec-help-logo = Imagen de la tarjeta — una ruta en /assets/img/ o una URL externa. Vacío usa un tono generado.
spec-help-cover = Fondo de la tarjeta: tono automático por tipo, color sólido o degradado.
spec-help-state = Las tarjetas activas se muestran; las inactivas se ocultan.
spec-help-subject = Tema/área usada por el filtro Asunto de la portada (p. ej. "Ventas", "Investigación").
spec-help-image = Imagen Docker a ejecutar, como repositorio:tag (p. ej. org/app:latest).
spec-help-seats = Cuántas sesiones simultáneas atiende un contenedor antes de crear otro.
spec-help-lifetime = Límite rígido, en minutos, de cuánto puede ejecutarse un contenedor antes de reciclarse.
spec-help-link = URL de destino para tarjetas de enlace externo — al hacer clic se navega aquí.
spec-help-updated = Fecha mostrada en la tarjeta (DD/MM/AAAA). Vacío estampa la fecha de hoy.
spec-help-api-port = Puerto en el que la API escucha dentro del contenedor. Por defecto 8080.
spec-help-api-rate-limit = Límite por cliente en el proxy, como N/unidad (p. ej. 100/min, 5/s). Por encima devuelve 429. Vacío = sin límite.
spec-help-api-docs-path = Ruta donde la API sirve la documentación OpenAPI/Swagger. Por defecto /__docs__.
spec-help-api-health-path = Ruta consultada para readiness antes de que la réplica entre al pool. Por defecto /__healthz__.
spec-help-api-cors = Añade cabeceras CORS permisivas y responde al preflight. Desactivado por defecto.
spec-help-min-replicas = Contenedores siempre activos — el pool nunca baja de aquí. Por defecto 0.
spec-help-max-replicas = Tope hasta donde el auto-escalado puede crecer. Vacío = ilimitado.
spec-help-concurrent = Solicitudes que atiende una réplica de API antes de que el escalado añada otra.
spec-help-cpu-limit = CPU máxima en núcleos fraccionarios (p. ej. 0,5 = medio núcleo). Vacío = ilimitado.
spec-help-memory-limit = Memoria máxima, p. ej. 512m o 1.5g. Vacío = ilimitado.
spec-help-heartbeat = Tiempo de sesión inactiva en milisegundos; -1 nunca expira. Vacío = usa el valor global.
admin-blocks-slot-empty = Aún no hay bloques en este slot.
admin-blocks-drag-hint = Arrastra desde el asa para reordenar los bloques dentro del slot.
spec-form-volumes-section = Volúmenes
spec-form-volumes = Montajes de volumen
spec-form-volumes-help = Un bind por línea — /host:/contenedor (usa :ro para solo lectura). Añade tantos como necesites.
spec-help-volumes = Monta directorios del host en el contenedor (p. ej. datos persistentes, o estáticos que sirve la app). Solo admin; montar rutas del host equivale a root.
spec-form-routing-section = Enrutamiento
spec-form-inject-base-href = Reescribir el HTML de la app para el sub-path
spec-form-inject-base-href-help = Activado por defecto. Ruscker reescribe <base href> y las URL relativas a la raíz para que una app que asume estar en la raíz del servidor funcione bajo su sub-path /app/. Desactiva solo si la app lee X-Forwarded-Prefix y construye sus propias rutas.
spec-help-inject-base-href = Ruscker siempre reenvía X-Forwarded-Prefix / X-Script-Name (y X-Forwarded-Proto/-Host). Frameworks como FastAPI (root_path), Dash y Streamlit pueden auto-enrutarse con ellos — entonces esta reescritura de HTML es redundante.
spec-form-error-volume = Cada volumen debe ser /host:/contenedor (opcional :ro).
spec-form-error-env = Cada variable de entorno debe ser NOMBRE=valor, con un NOMBRE válido (letras, números, _; empezando por letra o _). Corrige o elimina la línea inválida.
admin-nav-logs = Registros
admin-proclog-title = Registros
admin-proclog-subtitle = Flujo de eventos del balanceador y las réplicas
admin-proclog-unavailable = Búfer de registro no disponible (el servidor inició sin la capa de logging).
admin-proclog-empty = Aún no se ha capturado ningún registro en este nivel. Los nuevos eventos aparecen aquí a medida que ocurren; ejecuta el servidor con -v para incluir registros de nivel info.

# ── spec-form advanced params (#211/#212) ──────────────────────────
spec-form-runtime-section = Runtime
spec-form-container-port = Puerto del contenedor
spec-help-container-port = Puerto en el que la app escucha dentro del contenedor. Vacío = predeterminado por tipo (3838 para Shiny). Defínelo para Streamlit (8501), Dash (8050) o Jupyter (8888).
spec-form-platform = Plataforma
spec-help-platform = Plataforma Docker (ej.: linux/amd64) para ejecutar una imagen de otra arquitectura por emulación. Vacío = el daemon elige según el manifiesto.
spec-form-container-lifetime = Vida útil del contenedor (min)
spec-help-container-lifetime = Límite suave en minutos antes de reciclar el contenedor. Vacío = sin límite suave.
spec-form-stop-on-logout = Detener al cerrar sesión
spec-help-stop-on-logout = Detiene el contenedor del usuario cuando cierra sesión. Desactivado por defecto.
spec-form-env-section = Entorno + comando
spec-form-container-env = Variables de entorno
spec-form-container-env-help = Una por línea, NOMBRE=valor. Para secretos, referencia una variable de entorno en vez de pegar el valor.
spec-form-env-add = Añadir
spec-form-env-value = valor
spec-form-env-remove = Eliminar
spec-form-env-empty = Sin variables de entorno.
spec-help-container-env = Inyectadas en el contenedor (container-env). Vacío = ninguna. Para secretos, usa interpolación de variable de entorno.
spec-form-container-cmd = Comando (sobrescribir)
spec-form-container-cmd-help = Un argumento por línea. Vacío = usa el CMD de la imagen.
spec-help-container-cmd = Sobrescribe el comando del contenedor (container-cmd), como lista de argumentos.
spec-form-registry-section = Registro (imágenes privadas)
spec-form-registry-domain = Dominio del registro
spec-help-registry-domain = Host del registro para imágenes privadas (ej.: docker.io, ghcr.io). Vacío = Docker Hub.
spec-form-registry-username = Usuario
spec-help-registry-username = Usuario para autenticar la descarga de una imagen privada.
spec-form-registry-password = Contraseña
spec-form-registry-password-keep = En blanco mantiene la contraseña actual
spec-help-registry-password = Usa una variable de entorno — nunca pegues la contraseña en texto. Solo se usa junto con el usuario.
spec-form-registry-credential = Credencial guardada
spec-form-registry-none = No hay credenciales guardadas. Para imágenes privadas, crea una en la página Credenciales (enlace abajo).
spec-form-registry-none-option = (ninguna — imagen pública)
spec-form-registry-missing = eliminada
spec-help-registry-credential = Elige una credencial con nombre del almacén (página Credenciales) para descargar imágenes privadas. Cuando se define, tiene prioridad sobre el usuario/contraseña en línea.
spec-form-registry-help = Descarga una imagen privada seleccionando una credencial guardada. Crea y gestiona credenciales en la página Credenciales — la contraseña puede ser literal (cifrada) o una referencia a variable de entorno.
spec-form-registry-inline-note = Esta app tiene credenciales de registry en línea (YAML importado). Se conservan, pero prefiera una credencial guardada arriba.
spec-form-access-section = Acceso
spec-form-access-groups = Grupos permitidos
spec-help-access-groups = Grupos que pueden ver y acceder a la app (separados por comas). Vacío, con usuarios también vacío = abierto a todos.
spec-form-access-users = Usuarios permitidos
spec-help-access-users = Usuarios que pueden ver y acceder a la app (separados por comas).
spec-form-access-help = Ambos vacíos = tarjeta abierta a todos (incluido anónimo). Con algún valor, solo usuarios conectados que coincidan — y los admins siempre.
spec-form-access-public = Público
spec-form-access-add-group = + añadir grupo
spec-form-access-public-hint = vacío = visible para todos
spec-form-summary-replicas = réplicas
spec-form-summary-sessions = sesiones por réplica
spec-form-cpu-request = Reserva de CPU
spec-help-cpu-request = Reserva suave de CPU en núcleos (container-cpu-request). Vacío = sin reserva.
spec-form-memory-request = Reserva de memoria
spec-help-memory-request = Reserva suave de memoria, ej.: 256m. Vacío = sin reserva.
spec-form-max-body-size = Tamaño máx. del cuerpo
spec-help-max-body-size = Límite por spec del cuerpo de las solicitudes, ej.: 10m. Vacío = usa el límite global.
spec-form-scale-up = Umbral de scale-up
spec-help-scale-up = Fracción de utilización (0–1) que dispara crear una réplica. Vacío = predeterminado del scaler.
spec-form-scale-down = Umbral de scale-down
spec-help-scale-down = Fracción de utilización (0–1) por debajo de la cual se retira una réplica. Vacío = predeterminado del scaler.
spec-form-scale-down-grace = Gracia de scale-down (s)
spec-help-scale-down-grace = Segundos por debajo del umbral antes de retirar la réplica. Vacío = predeterminado.
spec-form-drain-timeout = Tiempo de drenaje (s)
spec-help-drain-timeout = Segundos para drenar las sesiones de una réplica antes de detenerla. Vacío = predeterminado.
spec-form-routing-strategy = Estrategia de enrutamiento
spec-help-routing-strategy = Cómo el balanceador elige la réplica. Vacío = predeterminado por tipo (least-connections para apps, round-robin para API).
spec-form-routing-default = Predeterminado (por tipo)
spec-form-placement = Ubicación (multi-host)
spec-help-placement = Cómo distribuir réplicas entre hosts Docker. Vacío = spread. Solo relevante con proxy.hosts.
spec-form-placement-default = Predeterminado (spread)
spec-form-anti-affinity = Anti-afinidad
spec-help-anti-affinity = Prefiere hosts distintos para las réplicas de este spec (multi-host). Desactivado por defecto.
spec-form-error-port = El puerto debe ser un número entre 1 y 65535.
spec-form-error-threshold = El umbral debe ser un número entre 0 y 1.

# ── spec-form image picker (#213) ──────────────────────────────────
spec-form-logo-upload = Subir imagen
spec-form-gallery-more = Mostrar más
spec-form-logo-clear = Quitar
spec-form-logo-none = Sin imagen — se usa un tono según el tipo.
spec-form-logo-builtin = Logos integrados
spec-form-logo-path-advanced = Avanzado: pegar una ruta o URL
spec-form-cover-image = Imagen
spec-form-cover-image-help = Elige una imagen de la biblioteca (o sube una) como fondo de la tarjeta.
admin-proclog-tail-note = Mostrando las líneas más recientes
admin-proclog-download = Descargar log completo
admin-proclog-filter-level = Filtrar por nivel
admin-proclog-filter-all = Todos los niveles
admin-proclog-search = Buscar en los logs…
admin-proclog-pause = Pausar
admin-proclog-resume = Reanudar
admin-proclog-clear = Limpiar
admin-proclog-lines = líneas
admin-proclog-filter-app-all = Todas las apps

landing-empty = Nada por aquí.

admin-landing-style = Estilo (CSS)
admin-landing-card-content = Contenido
admin-landing-card-meta = SEO y analítica
admin-landing-card-header-desc = Título y subtítulo mostrados en la parte superior del portal.
admin-landing-card-content-desc = El texto introductorio del portal, general y por idioma.
admin-landing-card-meta-desc = Metadatos de búsqueda/compartir y el fragmento de analítica.
admin-landing-card-style-desc = CSS personalizado, inyectado al final (escape hatch).
admin-landing-custom-css = CSS personalizado
admin-landing-custom-css-help = CSS inyectado al final del <head> de la landing — sobrescribe los estilos por defecto. Apunta a clases/variables estables (.rcard, .tint-*, --color-link, variables del header). De confianza (admin); cuidado con romper el diseño.
admin-landing-logos-help = Añade logos a la cabecera o al pie. A la izquierda: sustituye la marca Ruscker (cabecera) o se ubica a la izquierda (pie). A la derecha: tras los botones (cabecera) o junto a la versión (pie). Al centro: barra separada. Varios en la misma alineación quedan lado a lado.
admin-landing-logo-header = Cabecera
admin-landing-logo-footer = Pie
admin-landing-logo-left = Izquierda
admin-landing-logo-center = Centro
admin-landing-logo-right = Derecha
admin-landing-logo-link = Enlace (opcional)
admin-landing-logo-height = Altura (px)
admin-landing-logo-margin = Margen (px)
admin-landing-logos-card = Logotipos
admin-landing-logo-main = Logo principal (cabecera)
admin-landing-logo-main-help = "Marca + nombre" usa el símbolo de Ruscker; "Solo símbolo" oculta el título; "Personalizado" reemplaza el símbolo por tu imagen. El tamaño y el margen de abajo aplican a ambos.
admin-landing-logos-extra = Logos adicionales
admin-landing-logos-extra-help = Logos extra en el centro/derecha de la cabecera o en el pie (socios, instituciones). La izquierda de la cabecera es del logo principal.
admin-landing-header-style-card = Estilo de la cabecera
admin-landing-bgmode-preset = Preajuste
admin-landing-bgmode-help = Preajuste usa los estilos integrados; Sólido y Degradado pintan un fondo personalizado que reemplaza el preajuste.
admin-landing-header-dark-inherit = hereda del claro
admin-landing-header-dark-inherit-help = En blanco, el tema oscuro usa el mismo fondo que el claro.
admin-landing-cards-card = Tarjetas del catálogo
admin-landing-theme-card = Tema y colores
admin-landing-logo-image = Imagen
admin-landing-logo-slot-label = Posición
admin-landing-logo-align-label = Alineación
admin-landing-logo-add = Añadir logo

# — Gestión de disco (admin) #453
admin-nav-disk = Disco
admin-disk-title = Disco
admin-disk-subtitle = Recupera espacio de contenedores detenidos e imágenes sin usar.
admin-disk-backend-missing = El backend de Docker no está conectado — inicia el servidor con `--docker` para gestionar el disco.
admin-disk-containers-heading = Contenedores de Ruscker
admin-disk-prune = Eliminar detenidos
admin-disk-prune-confirm = ¿Eliminar todos los contenedores detenidos de Ruscker?
admin-disk-no-containers = No hay contenedores gestionados por Ruscker.
admin-disk-col-container = Contenedor
admin-disk-col-app = App
admin-disk-col-image = Imagen
admin-disk-col-status = Estado
admin-disk-running = en ejecución
admin-disk-remove = Eliminar
admin-disk-remove-confirm = ¿Eliminar este contenedor?
admin-disk-remove-running-confirm = Este contenedor está en ejecución. ¿Detenerlo y eliminarlo?
admin-disk-images-heading = Imágenes
admin-disk-images-total = Total
admin-disk-used = Usado
admin-disk-free = libres
admin-disk-seg-images = Imágenes Ruscker
admin-disk-seg-other = Otro uso
admin-disk-seg-free = Libre
admin-disk-images-note = El total puede contar capas compartidas más de una vez. Solo se pueden eliminar imágenes sin usar (sin forzar).
admin-disk-no-images = No hay imágenes locales.
admin-disk-col-id = ID
admin-disk-col-size = Tamaño
admin-disk-col-usage = Uso
admin-disk-used-by-spec = usada por una app
admin-disk-used-by-container = usada por un contenedor
admin-disk-unused = sin usar
admin-disk-in-use-hint = En uso — no se puede eliminar.
admin-disk-remove-image-confirm = ¿Eliminar esta imagen?
admin-disk-flash-removed = Eliminado.
admin-disk-flash-pruned = Contenedores detenidos eliminados.
admin-disk-flash-nothing = Nada que eliminar.
admin-disk-flash-error = La operación falló. Revisa los registros.
admin-disk-prune-images = Eliminar sin usar
admin-disk-prune-images-confirm = ¿Eliminar todas las imágenes sin usar?
admin-disk-flash-images-pruned = Imágenes sin usar eliminadas.
admin-disk-cleaning = Limpiando…
admin-disk-word-images = imágenes
admin-disk-word-containers = contenedores
admin-disk-word-stopped = parados
admin-disk-badge-inuse = en uso
admin-dashboard-metric-sessions-help = Sesiones que las réplicas reportan atendiendo ahora.
admin-dashboard-metric-tracker-help = Sesiones fijas (sticky) que el proxy rastrea en el heartbeat.
admin-landing-header = Encabezado
admin-landing-portal-title = Título del portal
admin-landing-portal-title-help = Aparece arriba en la landing. En blanco usa el título del config (proxy.title).
admin-landing-portal-subtitle = Subtítulo
admin-landing-portal-subtitle-help = La línea bajo el título. Déjalo en blanco para ocultarlo.
admin-landing-footer = Pie de página
admin-landing-footer-help = Texto en el pie del portal. En blanco muestra la versión y la marca.
admin-landing-default-theme = Tema predeterminado
admin-landing-default-theme-help = El tema inicial para quien nunca eligió. El visitante aún puede cambiar.
admin-landing-visible-sections = Secciones visibles
admin-landing-show-search = Barra de búsqueda
admin-landing-show-filters = Filtros de acceso (público/restringido)
admin-landing-brand-color = Color de marca
admin-landing-brand-custom = Color personalizado
admin-landing-brand-color-help = Atajo para el acento (claro y oscuro). Ajuste fino abajo.
admin-landing-logo-mode-mark = Marca + nombre
admin-landing-logo-mode-symbol = Solo símbolo
admin-landing-logo-mode-custom = Personalizado
admin-landing-logo-size = Tamaño del logo
admin-landing-header-bg-preset = Estilo de la cabecera
admin-landing-header-bg-preset-help = Un color de fondo personalizado (en Apariencia) anula este ajuste.
admin-landing-preset-flat = Plano
admin-landing-preset-soft = Suave
admin-landing-preset-bold = Intenso
admin-landing-card-cover-default = Portada predeterminada de tarjetas
admin-landing-cover-auto = Auto
admin-landing-cover-auto-sub = color del tipo
admin-landing-cover-own = Propio
admin-landing-cover-inherited = Heredado
admin-landing-cover-inherits-line = Hereda el fondo del tema claro.
admin-landing-card-cover-default-auto-help = Cada tarjeta usa un tono del color de su tipo. Sin configuración: se adapta automáticamente.
admin-landing-catalog-layout = Diseño del catálogo
admin-landing-layout-grid = Cuadrícula
admin-landing-layout-list = Lista
admin-landing-layout-sections = Secciones
admin-landing-density-comfortable = Cómodo
admin-landing-density-compact = Compacto






admin-landing-theme-colors = Colores por tema
admin-landing-theme-colors-help = Recolorea el tema claro y oscuro del portal público. En blanco, mantiene el predeterminado.
admin-landing-theme-light = Tema claro
admin-landing-theme-dark = Tema oscuro
admin-landing-theme-bg = Fondo
admin-landing-theme-text = Texto
admin-landing-theme-accent = Acento

# Featured carousel (#506)
highlights-title = Destacados
spec-form-featured = Destacar esta app
spec-form-featured-help = Muestra la app en el carrusel de Destacados arriba de la landing (si la opción está activada).
admin-landing-show-highlights = Mostrar Destacados
admin-landing-show-highlights-help = Muestra el carrusel de apps destacadas encima de los filtros. Se oculta si no hay ninguna destacada.

# Groups page (#503, read-only)
admin-nav-groups = Grupos
admin-groups-title = Grupos
admin-groups-subtitle = Grupos derivados de los apps (access-groups) y usuarios — solo lectura. Edita en el usuario o el app.
admin-groups-members = Miembros
admin-groups-apps = Apps
admin-groups-public-title = Apps públicas
admin-groups-public-help = Sin grupo — visible para todos
admin-groups-rename = Renombrar grupo
admin-groups-rename-prompt = Nuevo nombre del grupo:
admin-groups-delete = Eliminar grupo
admin-groups-delete-confirm = ¿Eliminar este grupo? Se quitará de todos los usuarios y apps que lo usan.
admin-groups-remove-member = Quitar del grupo
admin-groups-remove-member-confirm = ¿Quitar este miembro del grupo?
admin-groups-add-member = Añadir miembro
admin-groups-pick-user = Elegir usuario…
admin-groups-create = Crear grupo
admin-groups-new-name = Nombre del grupo
admin-groups-new-group-title = Nuevo grupo:
admin-groups-flash-renamed = Grupo renombrado.
admin-groups-flash-deleted = Grupo eliminado.
admin-groups-flash-member-added = Miembro añadido.
admin-groups-flash-member-removed = Miembro quitado.
admin-groups-flash-bad-input = Datos inválidos (nombre vacío o usuario inexistente).
admin-groups-empty = Aún no hay grupos. Aparecen cuando defines access-groups en un app o grupos en un usuario.
admin-groups-no-members = Sin miembros
admin-groups-no-apps = Ningún app restringido a este grupo

highlights-prev = Anteriores
highlights-next = Siguientes

# Featured star toggle in the Apps table (#521)
admin-specs-col-featured = Destacado
admin-specs-featured-on = Destacado — clic para quitar
admin-specs-featured-off = No destacado — clic para destacar
admin-specs-featured-readonly = El destacado se define en el archivo de config

# Importacion selectiva (#557)
admin-import-preview-title = Confirmar importación
admin-import-preview-help = Elige qué apps importar
admin-import-apps-label = apps
admin-import-warnings-label = avisos
admin-import-preview-none = El archivo no contiene ninguna app.
admin-import-select-all = Seleccionar todo
admin-import-col-status = Estado
admin-import-badge-new = Nuevo
admin-import-badge-new-help = Se creará (no está en el panel)
admin-import-badge-update = Actualiza
admin-import-badge-update-help = Sobrescribe una app ya existente en el panel
admin-import-confirm = Importar seleccionados
admin-import-load-file = Cargar archivo
admin-import-editor-placeholder = Pega tu application.yml aquí…
admin-import-editor-empty = La vista previa aparece aquí mientras escribes.
