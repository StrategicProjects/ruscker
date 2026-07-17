### Landing page — es-ES

landing-title = Ruscker
landing-subtitle = Portal de aplicaciones y APIs
landing-signin = Entrar
landing-panel = Panel
landing-signout = Salir
landing-signed-in-as = { $user }

filter-search-placeholder = Buscar aplicación…
filter-clear = Limpiar filtros

type-all = Todos
type-app = Aplicaciones
type-package = Paquetes
type-link = Enlaces
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
admin-nav-dashboard = Contenedores
admin-nav-apps = Aplicaciones
admin-nav-images = Multimedia
admin-nav-credentials = Credenciales
admin-nav-landing = Apariencia
admin-nav-blocks = Bloques
admin-blocks-title = Bloques HTML
admin-blocks-subtitle = Fragmentos HTML renderizados en la landing (superior / base).
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
admin-blocks-slot-top = Superior (después de la cabecera)
admin-blocks-slot-bottom = Base (después de la cuadrícula)
admin-blocks-title-label = Título (interno)
admin-blocks-title-placeholder = Una etiqueta para reconocer el bloque
admin-blocks-html = HTML
admin-blocks-html-help = Se renderiza sin escapar en la landing — usa solo fuentes de confianza.
admin-blocks-origins = Orígenes permitidos (CSP)
admin-blocks-origins-help = Dominios separados por espacios permitidos en la CSP de la landing (p. ej. https://example.com).
admin-blocks-enabled-label = Activo (renderizar en la landing)
admin-blocks-save = Guardar
admin-blocks-cancel = Cancelar
admin-blocks-position = Posición
admin-blocks-pos-top = Superior
admin-blocks-pos-bottom = Base
admin-blocks-done = Listo
admin-blocks-delete-block = Eliminar bloque
admin-nav-audit = Actividades
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
admin-pw-error-short = La contraseña no cumple la política: mínimo 8 caracteres, con 1 mayúscula, 1 minúscula, 1 número y 1 carácter especial.
admin-pw-submit = Guardar contraseña
admin-pw-reveal = Mostrar/ocultar contraseña
# — Gestión de usuarios (admin)
admin-nav-users = Usuarios
admin-users-title = Gestión de Usuarios
admin-users-subtitle = Creación y edición de usuarios.
admin-users-edit = Editar usuario
admin-users-edit-title = Editar usuario
admin-users-edit-subtitle = Actualiza el acceso y el perfil con un único guardado.
admin-users-account = Datos de la cuenta
admin-users-save = Guardar cambios
admin-users-cancel = Cancelar
admin-users-password-section = Restablecimiento de contraseña
admin-users-password-reset-hint = Establece una contraseña temporal y exige cambiarla en el próximo acceso.
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
# Búsqueda + paginación en el servidor en la tabla de usuarios (#999)
admin-users-search = Buscar
admin-users-search-clear = Limpiar búsqueda
admin-users-search-none = Ningún usuario coincide con la búsqueda.
admin-users-pager-status = Página { $page } de { $pages } · { $total } { $total ->
        [one] usuario
       *[other] usuarios
    }
admin-users-prev = Anterior
admin-users-next = Siguiente
admin-users-must-change = Aún usa la contraseña inicial
admin-users-save-role = Guardar nivel
admin-users-groups = Grupos
admin-users-setor = Sector
admin-users-setor-placeholder = ej.: GAPE
admin-users-email = Correo
admin-users-celular = Móvil
admin-users-col-profile = Perfil
admin-users-save-profile = Guardar perfil
admin-users-import-review = Revisar importación
admin-users-import-change = Cambiar archivo
admin-users-import-choose = Elegir archivo CSV
admin-users-import-help = Columnas: username, role, password, groups, setor, email, celular. La primera fila es el encabezado. Separador: coma (,); en Windows, guarda el CSV en formato Unix con encoding UTF-8. Los roles van en inglés: viewer, editor, admin.
admin-users-import-title = Importar usuarios
admin-users-import-preview-title = Vista previa de importación
admin-users-import-col-status = Estado
admin-users-import-status-ok = se importará
admin-users-import-status-exists = ya existe — omitido
admin-users-import-status-bad-username = usuario inválido
admin-users-import-status-bad-password = contraseña fuera de la política (mín. 8, con mayúscula, minúscula, número y especial)
admin-users-import-status-bad-role = nivel inválido
admin-users-import-confirm = Importar usuarios
admin-users-import-cancel = Cancelar
admin-users-import-done-prefix = Importados:
admin-users-import-skipped-prefix = omitidos:
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
admin-users-password-rule = Mínimo 8 caracteres, con al menos 1 mayúscula, 1 minúscula, 1 número y 1 carácter especial.
admin-users-flash-weak-password = Contraseña débil — la política exige mínimo 8 caracteres, con 1 mayúscula, 1 minúscula, 1 número y 1 carácter especial.
admin-users-generate-password = Generar contraseña aleatoria
admin-users-flash-exists = Ya existe un usuario con ese nombre.

# Admin dashboard
admin-dashboard-title = Gestión de Contenedores
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
admin-specs-title = Gestión de Aplicaciones
admin-specs-refresh = Recargar
admin-specs-subtitle = Información y especificaciones de las imágenes de las aplicaciones
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
admin-specs-update-image = Actualizar imagen (re-pull)
admin-specs-update-image-running = Actualizando imagen…
admin-specs-update-image-ok = Imagen actualizada
admin-specs-update-image-fail = Error al actualizar la imagen
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
spec-form-created-title = Aplicación creada
spec-form-created-body = La aplicación se creó correctamente. ¿Qué deseas hacer ahora?
spec-form-created-stay = Volver al formulario
spec-form-created-list = Ir a la lista de aplicaciones
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
spec-form-image-repull = Actualizar imagen
spec-form-image-repull-help = Vuelve a descargar del registry — úselo tras republicar la misma etiqueta (bytes o arquitectura distintos).
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
spec-form-error-mfa-days = “Solicitar de nuevo después de N días” debe ser un entero entre 0 y 30.
spec-form-error-max-replicas-zero = Máx. de contenedores debe ser al menos 1 (0 hace que la app nunca inicie).
spec-form-error-cpu = El límite de CPU debe ser un número positivo (ej.: 0.5).
spec-form-error-memory = El límite de memoria debe ser un tamaño como 512m o 1.5g.
spec-form-error-replica-range = Réplicas máx. debe ser mayor o igual que réplicas mín.
spec-form-error-stale = Otra persona guardó esta app mientras editabas. Revisa los valores actuales abajo y envía de nuevo.

# Admin image library
admin-images-title = Biblioteca multimedia
admin-images-subtitle = Imágenes utilizadas por el portal
admin-images-formats-help = PNG, JPEG y WebP se convierten a WebP. SVG pasa directo.
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
admin-creds-title = Gestión de Credenciales
admin-creds-subtitle = Creación de credenciales. Se cifran en reposo con AES-256-GCM y nunca aparecen en el YAML ni en el panel después de guardarlas.
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
admin-landing-title = Apariencia del Portal
admin-landing-crumb = Ajustes · Landing page
admin-landing-subtitle = Configuración de logo, barras, pies de página, etc.
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
admin-landing-intro-help = Se muestra sobre el catálogo. Acepta **negrita**, *cursiva* y [enlaces](https://…) — sin HTML.
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
admin-audit-title = Historial de Actividades Administrativas
admin-audit-subtitle = Visualización de actividades administrativas, de la más reciente a la más antigua. Tope de 100 eventos por consulta.
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
spec-form-max-replicas = Máx. de contenedores
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
spec-help-max-replicas = Tope rígido — el máximo de contenedores que Ruscker ejecuta para esta app (el auto-escalado crece hasta él). Vacío = el valor por defecto (5, o las réplicas iniciales si es mayor).
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
spec-form-error-network = Nombre de red Docker inválido (debe empezar con letra o número, luego letras/números/_/./-).
spec-form-error-env = Cada variable de entorno debe ser NOMBRE=valor, con un NOMBRE válido (letras, números, _; empezando por letra o _). Corrige o elimina la línea inválida.
admin-nav-logs = Registros
admin-proclog-title = Auditoría de Logs
admin-proclog-subtitle = Visualización de los logs de eventos del balanceador y las réplicas.
admin-proclog-unavailable = Búfer de registro no disponible (el servidor inició sin la capa de logging).
admin-proclog-empty = Aún no se ha capturado ningún registro en este nivel. Los nuevos eventos aparecen aquí a medida que ocurren; ejecuta el servidor con -v para incluir registros de nivel info.

# ── spec-form advanced params (#211/#212) ──────────────────────────
spec-form-runtime-section = Runtime
spec-form-container-port = Puerto del contenedor
spec-help-container-port = Puerto en el que la app escucha dentro del contenedor. Vacío = predeterminado por tipo (3838 para Shiny). Defínelo para Streamlit (8501), Dash (8050) o Jupyter (8888).
spec-form-platform = Plataforma
spec-help-platform = Plataforma Docker (ej.: linux/amd64) para ejecutar una imagen de otra arquitectura por emulación. Vacío = el daemon elige según el manifiesto.
spec-form-container-network = Red Docker
spec-help-container-network = Red Docker a la que conectar el contenedor (se crea si no existe). Vacío = la bridge por defecto del daemon. Úsala para aislar los contenedores de esta app en su propia red.
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
spec-form-require-mfa = Exigir 2FA
spec-form-require-mfa-hint = Los usuarios sin un factor TOTP configurado recibirán instrucciones para registrarlo en el primer acceso a una app protegida.
spec-form-mfa-validity = Volver a solicitar después de N días
spec-form-mfa-validity-hint = Vacío = 7 días. Usa 0 para exigir una nueva prueba en cada sesión de inicio de sesión, sin dispositivo recordado.
spec-form-mfa-staged-note = La aplicación de 2FA llegará en una próxima versión; por ahora, esta app aún no está protegida.
spec-form-identity-headers = Enviar cabeceras de identidad a la app
spec-form-identity-headers-hint = Añade X-SP-UserId y X-SP-UserGroups para usuarios autenticados. Desactivado por defecto; actívalo solo para apps que necesiten y confíen en esta identidad.
spec-form-identity-claims = Datos de identidad adicionales
spec-form-identity-claims-hint = Envía solo los datos de perfil seleccionados a esta app. Estos datos son independientes de las cabeceras de identidad X-SP.
spec-form-identity-claim-email = Correo electrónico
spec-form-identity-claim-setor = Sector / unidad
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
spec-form-scale-down-cooldown = Cooldown tras scale-down (s)
spec-help-scale-down-cooldown = Segundos sin scale-up por saturación tras retirar una réplica. Vacío = 60; 0 desactiva.
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

admin-landing-style = CSS personalizado
admin-landing-card-content = Contenido
admin-landing-card-meta = SEO y analítica
admin-landing-card-header-desc = Título y subtítulo mostrados en la parte superior del portal.
admin-landing-card-content-desc = El texto introductorio del portal, general y por idioma.
admin-landing-card-meta-desc = Metadatos de búsqueda/compartir y el fragmento de analítica.
admin-landing-card-style-desc = CSS personalizado, inyectado al final (escape hatch).
admin-landing-custom-css = CSS personalizado
admin-landing-custom-css-help = Inyectado al final del portal público. Úselo con cuidado.
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
admin-disk-title = Gestión del Disco
admin-disk-subtitle = Monitoreo del disco y recuperación de espacio ocioso de contenedores detenidos e imágenes sin usar.
admin-disk-backend-missing = El backend de Docker no está conectado — inicia el servidor con `--docker` para gestionar el disco.
admin-disk-containers-heading = Contenedores de Ruscker
admin-disk-prune = Eliminar detenidos
admin-disk-prune-confirm = ¿Eliminar todos los contenedores detenidos de Ruscker?
admin-disk-no-containers = No hay contenedores gestionados por Ruscker.
admin-disk-containers-unavailable = No se pudo cargar el inventario de contenedores de Docker. Esta es una vista parcial; reintenta cuando Docker responda.
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
admin-disk-images-unavailable = No se pudo cargar el inventario de imágenes de Docker. Esta es una vista parcial; reintenta cuando Docker responda.
admin-disk-col-id = ID
admin-disk-col-size = Tamaño
admin-disk-col-usage = Uso
admin-disk-used-by-spec = usada por una app
admin-disk-used-by-container = usada por un contenedor
admin-disk-unused = sin usar
admin-disk-usage-unknown = No se pudo consultar los contenedores en ejecución en Docker — cada imagen se muestra como en uso y la eliminación queda deshabilitada para no borrar una imagen aún en uso. Reintenta cuando Docker responda.
admin-disk-in-use-hint = En uso — no se puede eliminar.
admin-disk-foreign = no gestionada
admin-disk-foreign-hint = No es una imagen de Ruscker (p. ej. otra app en este host) — Ruscker no la eliminará.
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
admin-disk-volumes-title = Volúmenes
admin-disk-volumes-hint = Volúmenes Docker con nombre en este host. Solo los volúmenes creados por Ruscker y sin ninguna referencia pueden eliminarse.
admin-disk-volumes-create = Crear
admin-disk-volumes-name-placeholder = nombre-del-volumen
admin-disk-volumes-empty = No hay volúmenes con nombre.
admin-disk-volumes-unavailable = No se pudo cargar el inventario de volúmenes (o este backend no gestiona volúmenes). Es una vista parcial; reintenta cuando Docker responda.
admin-disk-volumes-locked = En uso, referenciado por una app o no creado por Ruscker — no se eliminará desde aquí.
admin-disk-volumes-badge-ruscker = Ruscker
admin-disk-volumes-badge-external = externo
admin-disk-volumes-confirm-remove = ¿Eliminar este volumen? Sus DATOS se borran definitivamente — no se puede deshacer.
admin-disk-volumes-col-name = Volumen
admin-disk-volumes-col-driver = Driver
admin-disk-volumes-col-created = Creado
admin-disk-volumes-col-refs = Referencias
admin-disk-volumes-col-origin = Origen
admin-disk-flash-volume-created = Volumen creado.
admin-disk-flash-volume-removed = Volumen eliminado.
admin-disk-flash-volume-bad-name = Nombre de volumen no válido — usa letras, dígitos, "_", "." o "-", empezando por letra o dígito.
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
card-favorite = Favorito
spec-form-featured = Destacar esta app
spec-form-featured-help = Muestra la app en el carrusel de Destacados arriba de la landing (si la opción está activada).
admin-landing-show-highlights = Mostrar Destacados
admin-landing-show-highlights-help = Muestra el carrusel de apps destacadas encima de los filtros. Se oculta si no hay ninguna destacada.

# Groups page (#503, read-only)
admin-nav-groups = Grupos
admin-groups-title = Gestión de Grupos
admin-groups-subtitle = Creación y edición de los grupos derivados de los apps y usuarios.
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

# Pestaña Sistema del admin (#766)
admin-nav-system = Sistema
admin-system-title = Sistema
admin-system-subtitle = Diagnóstico de solo lectura del servidor en ejecución.
admin-system-version = Versión de Ruscker
admin-system-base-path = Ruta base
admin-system-bind = Dirección de escucha
admin-system-docker = Docker
admin-system-db = Base de datos
admin-system-specs = Apps en el catálogo
admin-system-replicas = Réplicas en ejecución
admin-system-forward-headers = Confiar en cabeceras reenviadas
admin-system-metrics = Endpoint de métricas
admin-system-leader = Líder HA
admin-system-draining = Drenando
admin-system-yes = sí
admin-system-no = no
admin-system-restart-title = Reiniciar el servicio
admin-system-restart-hint = Ruscker no puede reiniciarse de forma segura por sí mismo — ejecuta esto en el host (las solicitudes en curso se drenan con SIGTERM).
admin-system-alerts-title = Webhook de alertas
admin-system-alerts-hint = Los eventos importantes (fallo al iniciar una app, réplica caída, app saturada en su límite) se envían como POST JSON a esta URL. Vacío = desactivado.
admin-system-alerts-url = URL del webhook
admin-system-alerts-save = Guardar
admin-system-alerts-test = Enviar alerta de prueba
admin-system-alerts-flash-saved = Webhook de alertas guardado.
admin-system-alerts-flash-bad-url = URL no válida — usa http:// o https:// (o déjala vacía para desactivar).
admin-system-alerts-flash-test = Alerta de prueba en cola — comprueba el destino (la entrega reintenta 3 veces).
admin-system-alerts-flash-no-url = Configura y guarda la URL del webhook antes de enviar una prueba.

# Recuperar espacio en disco (#766)
admin-disk-reclaim = Recuperar espacio
admin-disk-reclaim-hint = Limpia imágenes dangling + caché de compilación (seguro — nunca una imagen con tag ni un contenedor).
admin-disk-reclaim-confirm = ¿Recuperar espacio? Limpia imágenes dangling y la caché de compilación (no se elimina ninguna imagen con tag ni contenedor).
admin-disk-flash-reclaimed = Espacio recuperado (imágenes dangling + caché de compilación).

# Programaciones — cron jobs (#986 parte C)
admin-nav-schedules = Programaciones
admin-schedules-title = Programaciones
admin-schedules-subtitle = Ejecuta la imagen de una app hasta terminar según un horario cron (ETL, informes).
admin-schedules-create = Nueva programación
admin-schedules-spec = App
admin-schedules-cron = Cron
admin-schedules-cron-help = Cron estándar de 5 campos, en UTC. Ejemplos: "0 3 * * *" = cada día a las 03:00; "*/15 * * * *" = cada 15 minutos.
admin-schedules-cmd = Comando
admin-schedules-cmd-help = Un argumento por línea (argv). Vacío = el comando de la propia app (su container-cmd, si no el CMD de la imagen).
admin-schedules-timeout = Timeout (minutos)
admin-schedules-timeout-help = Límite de duración de una ejecución. Vacío = 1 hora.
admin-schedules-next-run = Próxima ejecución
admin-schedules-last-run = Última ejecución
admin-schedules-enabled = Activa
admin-schedules-disabled = Inactiva
admin-schedules-toggle = Activar/desactivar
admin-schedules-delete = Eliminar
admin-schedules-confirm-delete = ¿Eliminar esta programación? Su historial de ejecuciones se va con ella.
admin-schedules-empty = Aún no hay programaciones — crea una arriba.
admin-schedules-runs-title = Últimas ejecuciones
admin-schedules-runs-empty = Aún no hay ejecuciones.
admin-schedules-runs-started = Inicio
admin-schedules-runs-status = Estado
admin-schedules-runs-exit = Código de salida
admin-schedules-runs-duration = Duración
admin-schedules-log = Log
admin-schedules-flash-created = Programación creada. Se dispara en la próxima ocurrencia del cron (no se ejecuta al crearla).
admin-schedules-flash-deleted = Programación eliminada.
admin-schedules-flash-toggled = Programación actualizada.
admin-schedules-flash-bad-cron = Expresión cron no válida — usa la forma de 5 campos, p. ej. "0 3 * * *".
admin-schedules-flash-bad-spec = App desconocida, o la app no tiene imagen de contenedor que ejecutar.
admin-schedules-flash-error = La operación falló — revisa los logs del servidor.

# — TOTP / autenticación de dos factores (#1005)
chrome-mfa = Autenticación de dos factores
admin-mfa-title = Autenticación de dos factores
admin-mfa-help = Protege tu cuenta con códigos temporales de una aplicación de autenticación.
admin-mfa-error-password = La contraseña actual es incorrecta.
admin-mfa-error-key = RUSCKER_MASTER_KEY no está configurada. Configúrala y reinicia Ruscker antes de registrar el 2FA.
admin-mfa-break-glass = Las sesiones de emergencia por token no tienen cuenta ni contraseña y no pueden configurar 2FA. Inicia sesión con usuario y contraseña.
admin-mfa-already = El 2FA ya está configurado. Un administrador debe restablecerlo antes de volver a configurarlo.
admin-mfa-enrolled = 2FA configurado
admin-mfa-enrolled-since = Configurado desde
admin-mfa-reenroll-note = Para cambiar de aplicación, pide a un administrador que restablezca tu 2FA y vuelve a configurarlo.
admin-mfa-not-enrolled = 2FA no configurado
admin-mfa-pending-note = Hay una configuración incompleta. Introduce tu contraseña para comenzar de nuevo con una clave nueva.
admin-mfa-current-password = Contraseña actual
admin-mfa-start = Configurar 2FA
admin-mfa-setup-title = Vincula tu aplicación de autenticación
admin-mfa-setup-help = Escanea el código QR en la aplicación e introduce el código de 6 dígitos que genera.
admin-mfa-error-rate = Demasiados intentos incorrectos. Espera un minuto e inténtalo de nuevo.
admin-mfa-error-code = Código incorrecto o caducado. Comprueba el reloj del dispositivo e inténtalo de nuevo.
admin-mfa-manual-title = Clave de configuración manual
admin-mfa-manual-help = Si no puedes escanear el QR, introduce esta clave en la aplicación de autenticación.
admin-mfa-profile = Perfil: SHA-1, 6 dígitos, periodo de 30 segundos.
admin-mfa-code-label = Código de 6 dígitos
admin-mfa-confirm = Confirmar y activar
admin-mfa-recovery-title = Guarda tus códigos de recuperación
admin-mfa-recovery-warning = Estos códigos se muestran una sola vez. Cópialos o guárdalos ahora en un lugar seguro.
admin-mfa-recovery-help = Cada código puede usarse una sola vez si pierdes acceso a la aplicación de autenticación.
admin-mfa-continue = Continuar
admin-users-mfa-section = Autenticación de dos factores
admin-users-mfa-configured = 2FA configurado desde
admin-users-mfa-reset-hint = El restablecimiento elimina la clave y todos los códigos de recuperación. El usuario deberá configurar 2FA de nuevo.
admin-users-mfa-reset-confirm = ¿Restablecer el 2FA de este usuario? La clave y TODOS los códigos de recuperación se eliminarán inmediatamente.
admin-users-mfa-reset = Restablecer 2FA
admin-users-mfa-not-configured = 2FA no configurado
admin-users-mfa-reset-ok = Se restablecieron el 2FA y sus códigos de recuperación.
