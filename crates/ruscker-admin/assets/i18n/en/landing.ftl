### Landing page — en-US

landing-title = Ruscker
landing-subtitle = Application and API portal

filter-search-placeholder = Search application…
filter-access-all = All
filter-access-public = Public
filter-access-restricted = Restricted
filter-clear = Clear filters

type-all = All
type-app = Applications
type-package = Packages
type-talk = Presentations
type-report = Reports
type-api = APIs

type-app-abbr = APP
type-talk-abbr = PRS
type-report-abbr = RPT
type-package-abbr = PKG
type-api-abbr = API
type-link-abbr = LNK

# Admin shell
admin-nav-dashboard = Dashboard
admin-nav-apps = Apps
admin-nav-images = Media
admin-nav-credentials = Credentials
admin-nav-landing = Portal
admin-nav-blocks = Blocks
admin-blocks-title = HTML blocks
admin-blocks-subtitle = Custom HTML snippets rendered on the landing (top/bottom slots).
admin-blocks-new = New block
admin-blocks-empty = No blocks yet.
admin-blocks-col-slot = Slot
admin-blocks-col-title = Title
admin-blocks-col-status = Status
admin-blocks-enabled = enabled
admin-blocks-disabled = disabled
admin-blocks-edit = edit
admin-blocks-delete = delete
admin-blocks-delete-confirm = Delete this block?
admin-blocks-move-up = Move up
admin-blocks-move-down = Move down
admin-blocks-form-new = New block
admin-blocks-form-edit = Edit block
admin-blocks-slot = Slot
admin-blocks-slot-help = Where the block appears on the landing.
admin-blocks-slot-top = Top (after the header)
admin-blocks-slot-bottom = Bottom (after the grid)
admin-blocks-title-label = Title (internal)
admin-blocks-title-placeholder = A label to recognize this block
admin-blocks-html = HTML
admin-blocks-html-help = Rendered unescaped on the landing — only use trusted sources.
admin-blocks-origins = Allowed origins (CSP)
admin-blocks-origins-help = Space-separated domains allowed in the landing CSP (e.g. https://example.com).
admin-blocks-enabled-label = Enabled (render on the landing)
admin-blocks-save = Save
admin-blocks-cancel = Cancel
admin-nav-audit = Audit log
admin-nav-portal = Back to portal
admin-nav-logout = Sign out

# Admin dashboard
admin-dashboard-title = Monitoring dashboard
admin-dashboard-subtitle = Live container and session state
admin-dashboard-metric-containers = Containers
admin-dashboard-metric-sessions = Active sessions
admin-dashboard-metric-specs = Apps with replicas
admin-dashboard-metric-tracker = Tracked sessions
admin-dashboard-replicas-heading = Active replicas
admin-dashboard-no-replicas = No replicas running. Replicas show up here when the scaler enforces a minimum or when a request triggers a cold start.
admin-dashboard-col-spec = App
admin-dashboard-col-state = State
admin-dashboard-col-uptime = Uptime
admin-dashboard-col-sessions = Sessions
admin-dashboard-col-container = Container
admin-dashboard-col-cpu = CPU
admin-dashboard-col-memory = Memory
admin-dashboard-metric-memory = Memory used
admin-dashboard-metrics-pending = awaiting first reading
admin-dashboard-state-ready = ready
admin-dashboard-state-starting = starting
admin-dashboard-state-draining = draining
admin-dashboard-state-stopped = stopped
admin-dashboard-state-failed = failed
admin-dashboard-backend-missing = Docker backend is not connected — start the server with `--docker` to see containers here.

# Admin login
admin-login-title = Admin sign in
admin-login-help = Enter the admin token defined in RUSCKER_ADMIN_TOKEN.
admin-login-token-label = Token
admin-login-token-placeholder = Paste the token here
admin-login-submit = Sign in
admin-login-error-wrong = Wrong token. Please try again.
admin-login-back-portal = ← public portal

# Apps list
admin-specs-title = Apps
admin-specs-subtitle = Spec catalog stored in the database
admin-specs-empty = No apps yet. Use { $cmd } to import from a YAML.
admin-specs-add = Add app
admin-specs-col-id = ID
admin-specs-col-name = Name
admin-specs-col-kind = Kind
admin-specs-col-state = State
admin-specs-col-updated = Updated
admin-specs-col-version = Version
admin-specs-col-actions = Actions
admin-specs-filter-search = Search by id or name…
admin-specs-filter-kind-all = All kinds
admin-specs-filter-state-all = Active and inactive
admin-specs-edit = Edit
admin-specs-delete = Delete

# Spec form (new / edit)
spec-form-title-new = New app
spec-form-crumb-new = New
spec-form-crumb-edit = Edit
spec-form-cancel = Cancel
spec-form-save = Save changes
spec-form-kind = Kind
spec-form-kind-app = App container
spec-form-kind-talk = Presentation
spec-form-kind-report = Report
spec-form-kind-package = Package
spec-form-kind-api = API
spec-form-kind-link = External link
spec-form-identity = Identity
spec-form-id = ID
spec-form-id-help-new = Operator-chosen. Appears at /app/<id>/.
spec-form-id-help-edit = ID is immutable once created.
spec-form-name = Display name
spec-form-desc = Description
spec-form-visual = Visual
spec-form-logo = Card logo
spec-form-logo-help = URL or /assets/img/foo.png path. See docs/IMAGES.md.
spec-form-logo-pick-help = Or pick an image already uploaded to the media library.
spec-form-access = Access
spec-form-state = State
spec-form-state-active = Active
spec-form-state-inactive = Inactive
spec-form-subject = Subject
spec-form-container = Container
spec-form-image = Docker image
spec-form-seats = Sessions/container
spec-form-lifetime = Max lifetime (min)
spec-form-lifetime-help = 360 = 6 hours
spec-form-link-section = External link
spec-form-link = Target URL
spec-form-meta = Metadata
spec-form-updated = Updated on
spec-form-updated-help = Leave empty to use today's date.
spec-form-preview = Card preview
spec-form-preview-help = Updates live as you edit.
spec-form-actions = Actions
spec-form-delete = Delete app
spec-form-delete-confirm = Are you sure? This cannot be undone.

spec-form-error-id-required = ID is required.
spec-form-error-id-shape = ID must start with a letter and contain only letters, digits, "_" and "-".
spec-form-error-id-duplicate = An app with that ID already exists.
spec-form-error-name-required = Display name is required.
spec-form-error-number = A numeric field has a non-numeric value.
spec-form-error-cpu = CPU limit must be a positive number (e.g. 0.5).
spec-form-error-memory = Memory limit must be a size like 512m or 1.5g.
spec-form-error-replica-range = Max replicas must be greater than or equal to min replicas.

# Admin image library
admin-images-title = Media library
admin-images-subtitle = PNG, JPEG and WebP are converted to WebP. SVG passes through.
admin-images-drop-here = Click to pick a file
admin-images-formats = PNG · JPEG · WebP · SVG · up to 10 MB
admin-images-upload = Upload
admin-images-uploaded = Image uploaded:
admin-images-empty = No images yet. Upload the first one above.
admin-images-delete = Delete
admin-images-delete-confirm = Delete this image? Specs referencing the filename will fall back to the tinted cover.

# Admin credentials
admin-creds-title = Registry credentials
admin-creds-subtitle = Passwords are encrypted at rest with AES-256-GCM. They never appear in the YAML or in the panel after saving.
admin-creds-form-title = Add / update credential
admin-creds-name = Name
admin-creds-name-help = Unique identifier. Use the same name in your specs.
admin-creds-registry = Registry
admin-creds-username = Username
admin-creds-password = Password / token
admin-creds-password-help = Encrypted on save and never echoed back.
admin-creds-save = Save credential
admin-creds-saved = Credential saved:
admin-creds-empty = No credentials stored yet.
admin-creds-delete = Delete
admin-creds-delete-confirm = Delete this credential?
admin-creds-col-name = Name
admin-creds-col-registry = Registry
admin-creds-col-username = Username
admin-creds-col-created = Created
admin-creds-key-missing-title = RUSCKER_MASTER_KEY is not configured
admin-creds-key-missing-help = The credentials store needs a 32-byte key as hex (64 chars) or base64 (44 chars). Generate one with:

# Admin landing editor
admin-landing-title = Landing editor
admin-landing-crumb = Settings · Landing page
admin-landing-subtitle = Customize the public portal. Changes take effect on the visitor's next refresh.
admin-landing-open-portal = Open portal
admin-landing-save = Save
admin-landing-saved = Settings saved. Reload the public portal to see them.
admin-landing-colors = Header colors
admin-landing-header-bg = Background color
admin-landing-bg-help = Empty = use the theme's default (light/dark).
admin-landing-header-fg = Text color
admin-landing-clear = Clear
admin-landing-intro = Intro text (default)
admin-landing-intro-default = Default (fallback for all languages)
admin-landing-intro-default-placeholder = Welcome to the portal…
admin-landing-intro-help = Rendered between the header and the filters. Empty = no text.
admin-landing-intro-locales = Intro text per language
admin-landing-intro-pt = Portuguese
admin-landing-intro-en = English
admin-landing-intro-es = Spanish
admin-landing-intro-fr = French
admin-landing-preview = Portal preview
admin-landing-preview-help = Approximate look of the header and intro. Cards and filters appear as on the real landing.
admin-landing-preview-empty = (no intro text)
admin-landing-seo = SEO & sharing
admin-landing-seo-title = Page title (SEO)
admin-landing-seo-title-placeholder = Default: portal title
admin-landing-seo-title-help = Sets the browser tab title and og:title. Empty uses the portal's default title.
admin-landing-seo-description = Description (meta description)
admin-landing-seo-description-placeholder = Short summary for search engines and social cards
admin-landing-seo-description-help = Used in the meta description and og:description. Empty falls back to the intro text.
admin-landing-og-image = Share image (og:image)
admin-landing-og-image-help = URL or path (e.g. /assets/img/og.png) shown when the page is shared on social media.
admin-landing-analytics = Analytics
admin-landing-analytics-html = Analytics snippet
admin-landing-analytics-html-help = HTML injected into the landing <head> (e.g. a Plausible/Matomo/GA <script> tag). Rendered unescaped — only use trusted sources.
admin-landing-analytics-origins = Allowed origins (CSP)
admin-landing-analytics-origins-help = Space-separated domains (e.g. https://plausible.io) allowed in the landing CSP so the script can load and report.
admin-landing-future-title = HTML blocks
admin-landing-future-help = Manage custom HTML blocks (banners, notices) in the Blocks section of the menu.

# Admin audit log
admin-audit-title = Audit log
admin-audit-subtitle = Every admin change, newest first. Capped at 100 events per query.
admin-audit-family = Family
admin-audit-family-all = All actions
admin-audit-family-spec = Apps
admin-audit-family-image = Images
admin-audit-family-credential = Credentials
admin-audit-family-landing = Portal
admin-audit-actor = Actor
admin-audit-actor-all = All actors
admin-audit-target-placeholder = Search target (e.g. spec:sales-dashboard)
admin-audit-apply = Apply
admin-audit-empty = No changes yet — or the filter matches nothing.
admin-audit-limit-hint = Showing the 100 most recent matches. Tighten the filter to refine.

card-cta-open = Open
card-cta-link = Visit
card-cta-open-app = Open app
card-cta-open-talk = Open presentation
card-cta-open-report = Open report
card-cta-open-package = Open docs
card-cta-open-api = View docs
card-updated = Updated { $date }
status-new = new { $date }
status-updated = updated { $date }
sort-label = Sort
sort-recent = Recent
sort-name = Name
search-shortcut = ⌘ K

filter-subject-label = Subject
filter-subject-all = All subjects
filter-status-active = Active only
filter-status-all = Active and inactive
filter-status-inactive-only = Inactive only
card-state-active = Available
card-state-inactive = Unavailable
card-access-public = Public access
card-access-restricted = Restricted access

footer-language = Language
footer-theme = Theme
theme-light = Light
theme-dark = Dark
theme-auto = Auto

# Admin logs viewer
admin-logs-title = Container logs
admin-logs-back = Back to dashboard
admin-logs-replica = Replica
admin-logs-empty = No log output for this replica yet.
admin-logs-tail-note = Showing the last lines (newest at the bottom).

# Dashboard replica actions
admin-dashboard-action-stop = Stop
admin-dashboard-action-restart = Restart
admin-dashboard-confirm-stop = Stop this replica? The auto-scaler may recreate it if the configured minimum requires it.
admin-dashboard-confirm-restart = Restart this replica? Any active session will be lost.
admin-logs-follow = Live
admin-logs-follow-stop = Stop

# Admin YAML import
admin-import-button = Import YAML
admin-import-title = Import YAML configuration
admin-import-help = Pick a ShinyProxy or Ruscker application.yml. Import is idempotent and never deletes existing specs.
admin-import-file = .yml / .yaml file
admin-import-submit = Import
admin-import-cancel = Cancel
admin-import-ok = Import complete: { $created } created, { $updated } updated, { $unchanged } unchanged.
admin-import-ok-warnings = { $warnings } validation warning(s) — review embedded credentials and empty names.
admin-import-err = Import failed: { $msg }

# Gradient builder
admin-grad-solid = Solid
admin-grad-gradient = Gradient
admin-grad-linear = Linear
admin-grad-radial = Radial
admin-grad-add-stop = Add stop
admin-grad-remove-stop = Remove stop

# Spec form — card cover
spec-form-cover = Card cover
spec-form-cover-auto = Auto (type tint)
spec-form-cover-auto-help = Uses the card type default tint. Pick Solid or Gradient to customize.

# ── Spec form: advanced section + per-field help (#2) ──────────────
spec-form-advanced = Advanced
spec-form-advanced-hint = All optional — leave blank to keep the default.
spec-form-api-section = API
spec-form-scaling-section = Scaling
spec-form-resources-section = Resources
spec-form-lifecycle-section = Lifecycle
spec-form-api-port = Container port
spec-form-api-rate-limit = Rate limit
spec-form-api-docs-path = Docs path
spec-form-api-health-path = Health path
spec-form-api-cors = Enable permissive CORS
spec-form-min-replicas = Min replicas
spec-form-max-replicas = Max replicas
spec-form-concurrent = Requests per replica
spec-form-cpu-limit = CPU limit
spec-form-memory-limit = Memory limit
spec-form-heartbeat = Heartbeat timeout (ms)
spec-help-kind = What kind of thing this is. Drives routing, the card badge, and whether a container is started.
spec-help-id = Stable identifier used in the URL (/app/<id>). Lowercase letters, digits, "-" and "_"; can't change after creation.
spec-help-name = The title shown on the landing card.
spec-help-desc = Short description on the card. Inline HTML (e.g. a link) is allowed.
spec-help-logo = Card image — a path under /assets/img/ or an external URL. Blank uses a generated tint.
spec-help-cover = Card background: an automatic per-kind tint, a solid color, or a gradient.
spec-help-access = Shows a closed (restricted) or open (public) lock badge. Visual only — the MVP doesn't enforce auth.
spec-help-state = Active cards show on the landing; inactive ones are hidden.
spec-help-subject = Topic/area used by the landing's Subject filter (e.g. "Sales", "Research").
spec-help-image = Docker image to run, as repository:tag (e.g. org/app:latest).
spec-help-seats = How many concurrent sessions one container serves before another is spawned.
spec-help-lifetime = Hard cap, in minutes, on how long a container runs before it is recycled.
spec-help-link = Destination URL for external-link cards — clicking the card navigates here.
spec-help-updated = Display date on the card (DD/MM/YYYY). Blank stamps today's date.
spec-help-api-port = Port the API listens on inside the container. Default 8080.
spec-help-api-rate-limit = Per-client limit at the proxy, as N/unit (e.g. 100/min, 5/s). Over the limit returns 429. Blank = no limit.
spec-help-api-docs-path = Path where the API serves its OpenAPI/Swagger docs. Default /__docs__.
spec-help-api-health-path = Path probed for readiness before a replica joins the pool. Default /__healthz__.
spec-help-api-cors = Add permissive CORS headers and answer preflight requests. Off by default.
spec-help-min-replicas = Containers kept warm at all times — the pool never scales below this. Default 0.
spec-help-max-replicas = Upper bound the auto-scaler may spawn up to. Blank = unlimited.
spec-help-concurrent = Requests one API replica handles before the scaler adds another.
spec-help-cpu-limit = Max CPU as fractional cores (e.g. 0.5 = half a core). Blank = unlimited.
spec-help-memory-limit = Max memory, e.g. 512m or 1.5g. Blank = unlimited.
spec-help-heartbeat = Idle session timeout in milliseconds; -1 never expires. Blank = use the global default.
admin-blocks-slot-empty = No blocks in this slot yet.
admin-blocks-drag-hint = Drag the handle to reorder blocks within a slot.
spec-form-volumes-section = Volumes
spec-form-volumes = Volume mounts
spec-form-volumes-help = One bind per line — /host/path:/container/path (append :ro for read-only). Add as many as you need.
spec-help-volumes = Bind-mount host directories into the container (e.g. a persistent data dir, or a static assets dir the app serves). Admin-only; mounting host paths is root-equivalent.
spec-form-routing-section = Routing
spec-form-inject-base-href = Rewrite app HTML for the sub-path
spec-form-inject-base-href-help = On by default. Ruscker rewrites <base href> and root-relative URLs so an app that assumes it lives at the server root works under its /app/ sub-path. Turn off only if the app reads X-Forwarded-Prefix and builds its own paths.
spec-help-inject-base-href = Ruscker always forwards X-Forwarded-Prefix / X-Script-Name (plus X-Forwarded-Proto/-Host). Frameworks like FastAPI (root_path), Dash, and Streamlit can self-route from these — then this HTML rewriting is redundant.
spec-form-error-volume = Each volume must be /host:/container (optionally :ro).
admin-nav-logs = Logs
admin-proclog-title = Logs
admin-proclog-subtitle = Recent Ruscker process log (live).
admin-proclog-unavailable = Log buffer not wired (the server started without the logging layer).
