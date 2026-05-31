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
admin-nav-landing = Portal
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
admin-nav-portal = Volver al portal
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
admin-pw-first-prompt = Estás usando una contraseña asignada por un administrador. ¿Quieres cambiarla ahora?
admin-pw-current-label = Contraseña actual
admin-pw-new-label = Nueva contraseña
admin-pw-confirm-label = Confirmar contraseña
admin-pw-error-current = La contraseña actual es incorrecta.
admin-pw-error-mismatch = Las contraseñas no coinciden.
admin-pw-error-short = La contraseña debe tener al menos 8 caracteres.
admin-pw-submit = Guardar contraseña
admin-pw-keep = Mantener la contraseña actual
# — Gestión de usuarios (admin)
admin-nav-users = Usuarios
admin-users-title = Usuarios
admin-users-subtitle = Crea y gestiona quién accede al panel y con qué nivel.
admin-users-new = Nuevo usuario
admin-users-create = Crear
admin-users-initial-password = Contraseña inicial
admin-users-initial-password-hint = Se le preguntará si desea cambiarla en el primer acceso.
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
admin-users-flash-bad-input = Datos inválidos (usuario vacío o contraseña de menos de 8 caracteres).
admin-users-flash-exists = Ya existe un usuario con ese nombre.

# Admin dashboard
admin-dashboard-title = Panel de monitoreo
admin-dashboard-subtitle = Estado de contenedores y sesiones en tiempo real
admin-dashboard-metric-containers = Contenedores
admin-dashboard-metric-sessions = Sesiones activas
admin-dashboard-metric-specs = Aplicaciones con réplicas
admin-dashboard-metric-tracker = Sesiones rastreadas
admin-dashboard-replicas-heading = Réplicas activas
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
admin-specs-col-actions = Acciones
admin-specs-filter-search = Buscar por id o nombre…
admin-specs-filter-kind-all = Todos los tipos
admin-specs-filter-state-all = Activos e inactivos
admin-specs-edit = Editar
admin-specs-duplicate = Duplicar
admin-specs-config-badge = config
admin-specs-config-defined = Definido en el YAML — solo lectura aquí; edita el archivo
admin-specs-delete = Borrar

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
spec-form-visual = Visual
spec-form-logo = Logo del card
spec-form-logo-help = URL o ruta /assets/img/foo.png. Ver docs/IMAGES.md.
spec-form-logo-pick-help = O elige una imagen ya subida en la biblioteca de medios.
spec-form-state = Estado
spec-form-state-active = Activo
spec-form-state-inactive = Inactivo
spec-form-subject = Asunto
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
spec-form-error-number = Un campo numérico tiene un valor no numérico.
spec-form-error-cpu = El límite de CPU debe ser un número positivo (ej.: 0.5).
spec-form-error-memory = El límite de memoria debe ser un tamaño como 512m o 1.5g.
spec-form-error-replica-range = Réplicas máx. debe ser mayor o igual que réplicas mín.

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
admin-images-empty = Aún no hay imágenes. Suba la primera arriba.
admin-images-delete = Eliminar
admin-images-delete-confirm = ¿Eliminar esta imagen? Las specs que referencian el archivo mostrarán el cover tintado.
admin-images-search = Buscar imágenes…
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
admin-landing-title = Editor del portal
admin-landing-crumb = Ajustes · Landing page
admin-landing-subtitle = Personalice el portal público. Los cambios surten efecto al refrescar.
admin-landing-scope-help = Estas opciones (colores, textos de introducción, SEO, bloques personalizados) se aplican a la portada pública en vivo — guardadas aquí, mostradas en la próxima visita, sin reinicio. Es un conjunto fijo de ajustes, no un editor de CSS arbitrario.
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
admin-landing-seo = SEO y compartir
admin-landing-seo-title = Título de la página (SEO)
admin-landing-seo-title-placeholder = Predeterminado: título del portal
admin-landing-seo-title-help = Define el título de la pestaña y el og:title. Vacío usa el título por defecto del portal.
admin-landing-seo-description = Descripción (meta description)
admin-landing-seo-description-placeholder = Resumen para buscadores y redes sociales
admin-landing-seo-description-help = Se usa en la meta description y el og:description. Vacío usa el texto de introducción.
admin-landing-og-image = Imagen para compartir (og:image)
admin-landing-og-image-help = URL o ruta (p. ej. /assets/img/og.png) que se muestra al compartir en redes sociales.
admin-landing-analytics = Analytics
admin-landing-analytics-html = Snippet de analytics
admin-landing-analytics-html-help = HTML inyectado en el <head> de la landing (p. ej. una etiqueta <script> de Plausible/Matomo/GA). Se renderiza sin escapar — usa solo fuentes de confianza.
admin-landing-analytics-origins = Orígenes permitidos (CSP)
admin-landing-analytics-origins-help = Dominios separados por espacios (p. ej. https://plausible.io) permitidos en la CSP de la landing para que el script cargue y reporte.

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
spec-form-cover-auto = Auto (color del tipo)
spec-form-cover-auto-help = Usa el tono por defecto del tipo. Elija Sólido o Degradado para personalizar.

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
admin-nav-logs = Registros
admin-proclog-title = Registros
admin-proclog-subtitle = Registro reciente del proceso Ruscker (en vivo).
admin-proclog-unavailable = Búfer de registro no disponible (el servidor inició sin la capa de logging).

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
spec-help-registry-credential = Elige una credencial con nombre del almacén (página Credenciales) para descargar imágenes privadas. Cuando se define, tiene prioridad sobre el usuario/contraseña en línea.
spec-form-registry-help = Descarga una imagen privada seleccionando una credencial guardada. Crea y gestiona credenciales en la página Credenciales — la contraseña puede ser literal (cifrada) o una referencia a variable de entorno.
spec-form-registry-inline-note = Esta app tiene credenciales de registry en línea (YAML importado). Se conservan, pero prefiera una credencial guardada arriba.
spec-form-access-section = Acceso
spec-form-access-groups = Grupos permitidos
spec-help-access-groups = Grupos que pueden ver y acceder a la app (separados por comas). Vacío, con usuarios también vacío = abierto a todos.
spec-form-access-users = Usuarios permitidos
spec-help-access-users = Usuarios que pueden ver y acceder a la app (separados por comas).
spec-form-access-help = Ambos vacíos = tarjeta abierta a todos (incluido anónimo). Con algún valor, solo usuarios conectados que coincidan — y los admins siempre.
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

landing-empty = Nada por aquí.

admin-landing-style = Estilo (CSS)
admin-landing-custom-css = CSS personalizado
admin-landing-custom-css-help = CSS inyectado al final del <head> de la landing — sobrescribe los estilos por defecto. Apunta a clases/variables estables (.rcard, .tint-*, --color-link, variables del header). De confianza (admin); cuidado con romper el diseño.
