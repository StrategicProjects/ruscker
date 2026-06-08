### Landing page — en-US

landing-title = Ruscker
landing-subtitle = Application and API portal
landing-signin = Sign in
landing-panel = Panel
landing-signout = Sign out
landing-signed-in-as = { $user }

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
admin-nav-landing = Appearance
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
admin-nav-portal = Portal
admin-nav-logout = Sign out
role-current = Your access level
role-viewer = Viewer
role-editor = Editor
role-admin = Admin

# — Username/password login + token bootstrap (#107)
admin-login-help-user = Sign in with your username and password.
admin-login-username-label = Username
admin-login-username-placeholder = your username
admin-login-password-label = Password
admin-login-password-placeholder = your password
admin-login-error-credentials = Invalid username or password.
admin-login-use-token = Sign in with admin token
admin-login-use-password = Back to password sign-in
# — First-admin setup
admin-setup-title = Create the admin account
admin-setup-help = This is the first run. Choose a username and password for the administrator.
admin-setup-error = Could not create the account. Check the details.
admin-setup-password-label = Password
admin-setup-submit = Create admin
# — Password change / first login
admin-pw-title = Change password
admin-pw-help = Set a new password for your account.
admin-pw-first-prompt = You're using a password set by an administrator. Set a new password to continue.
admin-pw-current-label = Current password
admin-pw-new-label = New password
admin-pw-confirm-label = Confirm password
admin-pw-error-current = Current password is incorrect.
admin-pw-error-mismatch = Passwords don't match.
admin-pw-error-short = Password must be at least 8 characters.
admin-pw-submit = Save password
admin-pw-reveal = Show/hide password
# — User management (admin)
admin-nav-users = Users
admin-users-title = Users
admin-users-subtitle = Create and manage who can sign in, and at which level.
admin-users-new = New user
admin-users-create = Create
admin-users-initial-password = Initial password
admin-users-initial-password-hint = At least 8 characters. The user must change it on first login.
admin-users-role = Role
admin-users-col-user = User
admin-users-col-role = Role
admin-users-col-created = Created
admin-users-col-actions = Actions
admin-users-you = you
admin-users-must-change = Still using the initial password
admin-users-save-role = Save role
admin-users-groups = Groups
admin-users-groups-placeholder = analysts, managers
admin-users-groups-hint = Comma-separated groups control which restricted apps the user sees.
admin-users-col-groups = Groups
admin-users-save-groups = Save groups
admin-users-new-password = new password
admin-users-reset-password = Reset password
admin-users-delete = Remove user
admin-users-confirm-delete = Remove this user?
admin-users-flash-created = User created.
admin-users-flash-saved = Changes saved.
admin-users-flash-deleted = User removed.
admin-users-flash-last-admin = Can't remove or demote the last administrator.
admin-users-flash-bad-input = Invalid input: the username may contain only letters, digits and _ . @ - , and the password needs at least 8 characters.
admin-users-username-rule = Letters, digits and _ . @ - only (no spaces or accents).
admin-users-password-rule = At least 8 characters.
admin-users-flash-exists = A user with that name already exists.

# Admin dashboard
admin-dashboard-title = Monitoring dashboard
admin-dashboard-subtitle = Live container and session state
admin-dashboard-live = Live
admin-dashboard-filter-search = Filter app…
admin-dashboard-metric-containers = Containers
admin-dashboard-metric-sessions = Active sessions
admin-dashboard-metric-specs = Apps with replicas
admin-dashboard-metric-tracker = Tracked sessions
admin-dashboard-replicas-heading = Active replicas
admin-dashboard-grouped-by = Grouped by application
admin-dashboard-expand-all = Expand all
admin-dashboard-collapse-all = Collapse all
admin-dashboard-no-replicas = No replicas running. Replicas show up here when the scaler enforces a minimum or when a request triggers a cold start.
admin-dashboard-col-spec = App
admin-dashboard-col-state = State
admin-dashboard-col-uptime = Uptime
admin-dashboard-col-sessions = Sessions
admin-dashboard-col-host = Host
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
admin-login-title = Sign in
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
admin-specs-col-access = Accesses
admin-specs-col-access-groups = Access
admin-specs-access-public = public
admin-specs-col-access-help = Total accesses (app visits + external-card clicks)
admin-specs-col-actions = Actions
admin-specs-filter-search = Search by id or name…
admin-specs-kind-interactive = Interactive
admin-specs-kind-external = External
admin-specs-filter-kind-all = All kinds
admin-specs-filter-state-all = Active and inactive
admin-specs-edit = Edit
admin-specs-duplicate = Duplicate
admin-specs-config-badge = config
admin-specs-config-defined = Defined in the YAML config — read-only here; edit the file
admin-specs-delete = Delete

# Spec form (new / edit)
spec-form-title-new = New app
spec-form-crumb-new = New
spec-form-crumb-edit = Edit
spec-form-cancel = Back to apps
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
spec-form-state = State
spec-form-state-active = Active
spec-form-state-inactive = Inactive
spec-form-subject = Subject
spec-form-container = Container
spec-form-image = Docker image
spec-form-image-check = Check
spec-form-image-checking = Checking…
spec-form-image-present = On the server
spec-form-image-absent = Not on the server — pulled on first launch
spec-form-image-unresolved = Has an env variable — resolved at pull time
spec-form-image-no-backend = Docker not connected — can't check
spec-form-image-error = Image check failed
spec-form-image-pull = Pull
spec-form-image-pulling = Pulling…
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
admin-images-choose = Choose image
admin-images-builtin = Built-in logos
admin-images-builtin-tag = built-in
admin-images-uploaded = Image uploaded:
admin-images-renamed = renamed because this name was already taken:
admin-images-rename = Rename
admin-images-rename-prompt = New file name (the extension is kept):
admin-images-rename-taken = An image with that name already exists. Choose another.
admin-images-rename-invalid = Invalid name.
admin-images-empty = No images yet. Upload the first one above.
admin-images-delete = Delete
admin-images-delete-confirm = Delete this image? Specs referencing the filename will fall back to the tinted cover.
admin-images-inuse = In use
admin-images-inuse-help = Used by a card or a landing logo
admin-images-delete-confirm-inuse = This image is IN USE. Deleting it resets the apps that use it to the default Ruscker logo (the card will not break). Delete?
admin-images-search = Search images…
admin-images-type-all = All types
admin-images-no-match = No images match your search.

# Admin credentials
admin-creds-title = Registry credentials
admin-creds-subtitle = Passwords are encrypted at rest with AES-256-GCM. They never appear in the YAML or in the panel after saving.
admin-creds-form-title = Add / update credential
admin-creds-name = Name
admin-creds-name-help = Unique identifier. Use the same name in your specs.
admin-creds-registry = Registry
admin-creds-username = Username
admin-creds-password = Password / token
admin-creds-password-help = Encrypted on save and never echoed back. Or enter an environment-variable reference — resolved at pull time and never stored.
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
admin-landing-title = Portal appearance
admin-landing-crumb = Settings · Landing page
admin-landing-subtitle = Configure how the public portal looks to visitors.
admin-landing-scope-help = These options (colors, intro texts, SEO, custom blocks) apply to the public landing live — saved here, shown on the next view, no restart. It's a fixed set of settings, not an arbitrary-CSS editor.
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
admin-landing-seo-preview = Search preview
admin-landing-seo-title-placeholder = Default: portal title
admin-landing-seo-title-help = Sets the browser tab title and og:title. Empty uses the portal's default title.
admin-landing-seo-description = Description (meta description)
admin-landing-seo-description-placeholder = Short summary for search engines and social cards
admin-landing-seo-description-help = Used in the meta description and og:description. Empty falls back to the intro text.
admin-landing-og-image = Share image (og:image)
admin-landing-og-image-help = Image shown when the portal is shared on social media. Blank uses the header (left) logo, else the Ruscker mark. For best results upload a ~1200×630 PNG/JPG (some sites don't render SVG).
admin-landing-analytics = Analytics
admin-landing-analytics-html = Analytics snippet
admin-landing-analytics-html-help = HTML injected into the landing <head> (e.g. a Plausible/Matomo/GA <script> tag). Rendered unescaped — only use trusted sources.
admin-landing-analytics-origins = Allowed origins (CSP)
admin-landing-analytics-origins-help = Space-separated domains (e.g. https://plausible.io) allowed in the landing CSP so the script can load and report.

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
admin-audit-col-when = When
admin-audit-col-action = Action
admin-audit-col-target = Target
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
status-title-new = Recently updated
status-title-updated = Updated
status-title-none = No update date
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

theme-light = Light
theme-dark = Dark
theme-auto = Auto

# Top-right chrome cluster (#182 + #183)
chrome-cluster-label = Page settings
chrome-theme-label = Theme
chrome-language-label = Language
chrome-account-label = Account
chrome-signin = Sign in
chrome-signed-in-as-prefix = Signed in as
chrome-panel = Panel
chrome-change-password = Change password
chrome-signout = Sign out

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
admin-import-ok-assets = { $creds } credential(s) and { $logos } image(s) imported into the panel.
admin-import-drop = Drag your application.yml here or click to browse
admin-table-search = Search…
admin-table-no-results = No results
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
spec-form-choose-image = Choose image
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
admin-proclog-empty = No logs captured yet at this level. New events show up here as they happen; run the server with -v to include info-level logs.

# ── spec-form advanced params (#211/#212) ──────────────────────────
spec-form-runtime-section = Runtime
spec-form-container-port = Container port
spec-help-container-port = Port the app listens on inside the container. Blank = per-kind default (3838 for Shiny). Set it for Streamlit (8501), Dash (8050) or Jupyter (8888).
spec-form-platform = Platform
spec-help-platform = Docker platform (e.g. linux/amd64) to run a cross-arch image under emulation. Blank = the daemon picks per the manifest.
spec-form-container-lifetime = Container lifetime (min)
spec-help-container-lifetime = Soft cap in minutes before the container is recycled. Blank = no soft cap.
spec-form-stop-on-logout = Stop on logout
spec-help-stop-on-logout = Stop the user's container when they log out. Off by default.
spec-form-env-section = Environment + command
spec-form-container-env = Environment variables
spec-form-container-env-help = One NAME=value per line. For secrets, reference an environment variable instead of pasting the value.
spec-form-env-add = Add
spec-form-env-value = value
spec-form-env-remove = Remove
spec-form-env-empty = No environment variables.
spec-help-container-env = Injected into the container (container-env). Blank = none. For secrets, use environment-variable interpolation.
spec-form-container-cmd = Command (override)
spec-form-container-cmd-help = One argument per line. Blank = use the image's CMD.
spec-help-container-cmd = Override the container command (container-cmd), as an argument list.
spec-form-registry-section = Registry (private images)
spec-form-registry-domain = Registry domain
spec-help-registry-domain = Registry host for private images (e.g. docker.io, ghcr.io). Blank = Docker Hub.
spec-form-registry-username = Username
spec-help-registry-username = Username to authenticate the pull of a private image.
spec-form-registry-password = Password
spec-form-registry-password-keep = Blank keeps the current password
spec-help-registry-password = Use an environment variable — never paste the password as text. Only used together with the username.
spec-form-registry-credential = Stored credential
spec-form-registry-none = No saved credentials. For private images, create one on the Credentials page (link below).
spec-form-registry-none-option = (none — public image)
spec-form-registry-missing = removed
spec-help-registry-credential = Pick a named credential from the store (Credentials page) to pull private images. When set, it takes precedence over the inline username/password.
spec-form-registry-help = Pull a private image by selecting a saved credential. Create and manage credentials on the Credentials page — the password can be a literal (encrypted) or an environment-variable reference.
spec-form-registry-inline-note = This app carries inline registry credentials from imported YAML. They're preserved, but prefer a saved credential above.
spec-form-access-section = Access
spec-form-access-groups = Allowed groups
spec-help-access-groups = Groups that may see and reach the app (comma-separated). Blank, with users also blank = open to everyone.
spec-form-access-users = Allowed users
spec-help-access-users = Usernames that may see and reach the app (comma-separated).
spec-form-access-help = Both blank = card is open to everyone (including anonymous). With any value, only matching logged-in users — and admins always.
spec-form-access-public = Public
spec-form-access-add-group = + add group
spec-form-access-public-hint = empty = visible to everyone
spec-form-summary-replicas = replicas
spec-form-summary-sessions = sessions per replica
spec-form-cpu-request = CPU reservation
spec-help-cpu-request = Soft CPU reservation in cores (container-cpu-request). Blank = no reservation.
spec-form-memory-request = Memory reservation
spec-help-memory-request = Soft memory reservation, e.g. 256m. Blank = no reservation.
spec-form-max-body-size = Max body size
spec-help-max-body-size = Per-spec cap on proxied request bodies, e.g. 10m. Blank = use the global limit.
spec-form-scale-up = Scale-up threshold
spec-help-scale-up = Utilization fraction (0–1) that triggers spawning a replica. Blank = scaler default.
spec-form-scale-down = Scale-down threshold
spec-help-scale-down = Utilization fraction (0–1) below which a replica is reaped. Blank = scaler default.
spec-form-scale-down-grace = Scale-down grace (s)
spec-help-scale-down-grace = Seconds below the threshold before reaping the replica. Blank = default.
spec-form-drain-timeout = Drain timeout (s)
spec-help-drain-timeout = Seconds to drain a replica's sessions before stopping it. Blank = default.
spec-form-routing-strategy = Routing strategy
spec-help-routing-strategy = How the balancer picks a replica. Blank = per-kind default (least-connections for apps, round-robin for APIs).
spec-form-routing-default = Default (by kind)
spec-form-placement = Placement (multi-host)
spec-help-placement = How to spread replicas across Docker hosts. Blank = spread. Only relevant with proxy.hosts.
spec-form-placement-default = Default (spread)
spec-form-anti-affinity = Anti-affinity
spec-help-anti-affinity = Prefer distinct hosts for this spec's replicas (multi-host). Off by default.
spec-form-error-port = Port must be a number between 1 and 65535.
spec-form-error-threshold = Threshold must be a number between 0 and 1.

# ── spec-form image picker (#213) ──────────────────────────────────
spec-form-logo-upload = Upload image
spec-form-gallery-more = Show more
spec-form-logo-clear = Remove
spec-form-logo-none = No image — a kind-based tint is used.
spec-form-logo-builtin = Built-in logos
spec-form-logo-path-advanced = Advanced: paste a path or URL
spec-form-cover-image = Image
spec-form-cover-image-help = Pick a library image (or upload one) as the card background.
admin-proclog-tail-note = Showing the most recent lines
admin-proclog-download = Download full log
admin-proclog-filter-level = Filter by level
admin-proclog-filter-all = All levels
admin-proclog-search = Search logs…
admin-proclog-pause = Pause
admin-proclog-resume = Resume

landing-empty = Nothing here yet.

admin-landing-style = Style (CSS)
admin-landing-card-appearance = Appearance
admin-landing-card-content = Content
admin-landing-card-meta = SEO & analytics
admin-landing-card-header-desc = Title and subtitle shown at the top of the portal.
admin-landing-card-appearance-desc = Header colours and each theme's palette (light/dark).
admin-landing-card-content-desc = The portal's intro text, general and per language.
admin-landing-card-meta-desc = Search/share metadata and the analytics snippet.
admin-landing-card-style-desc = Custom CSS, injected last (escape hatch).
admin-landing-custom-css = Custom CSS
admin-landing-custom-css-help = CSS injected at the end of the landing <head> — overrides the built-in styles. Target stable classes/variables (.rcard, .tint-*, --color-link, header vars). Admin-trusted; take care not to break the layout.
admin-landing-logos = Header / footer logos
admin-landing-logos-help = Add logos to the header or footer. Left: replaces the Ruscker mark (header) or sits on the far left (footer). Right: after the buttons (header) or next to the version (footer). Center: a separate bar. Several at the same alignment sit side by side.
admin-landing-logo-header = Header
admin-landing-logo-footer = Footer
admin-landing-logo-left = Left
admin-landing-logo-center = Center
admin-landing-logo-right = Right
admin-landing-logo-link = Link (optional)
admin-landing-logo-height = Height (px)
admin-landing-logo-margin = Margin (px)
admin-landing-logo-image = Image
admin-landing-logo-slot-label = Position
admin-landing-logo-align-label = Alignment
admin-landing-logo-add = Add logo

# — Disk management (admin) #453
admin-nav-disk = Disk
admin-disk-title = Disk
admin-disk-subtitle = Reclaim space from stopped containers and unused images.
admin-disk-backend-missing = The Docker backend isn't connected — start the server with `--docker` to manage disk.
admin-disk-containers-heading = Ruscker containers
admin-disk-prune = Remove stopped
admin-disk-prune-confirm = Remove all stopped Ruscker containers?
admin-disk-no-containers = No Ruscker-managed containers.
admin-disk-col-container = Container
admin-disk-col-app = App
admin-disk-col-image = Image
admin-disk-col-status = Status
admin-disk-running = running
admin-disk-remove = Remove
admin-disk-remove-confirm = Remove this container?
admin-disk-remove-running-confirm = This container is running. Stop and remove it?
admin-disk-images-heading = Images
admin-disk-images-total = Total
admin-disk-used = Used
admin-disk-free = free
admin-disk-seg-images = Ruscker images
admin-disk-seg-other = Other used
admin-disk-seg-free = Free
admin-disk-images-note = The total may count shared layers more than once. Only unused images can be removed (no force).
admin-disk-no-images = No local images.
admin-disk-col-id = ID
admin-disk-col-size = Size
admin-disk-col-usage = Usage
admin-disk-used-by-spec = used by an app
admin-disk-used-by-container = used by a container
admin-disk-unused = unused
admin-disk-in-use-hint = In use — can't be removed.
admin-disk-remove-image-confirm = Remove this image?
admin-disk-flash-removed = Removed.
admin-disk-flash-pruned = Stopped containers removed.
admin-disk-flash-nothing = Nothing to remove.
admin-disk-flash-error = The operation failed. Check the logs.
admin-disk-prune-images = Remove unused
admin-disk-prune-images-confirm = Remove all unused images?
admin-disk-flash-images-pruned = Unused images removed.
admin-disk-cleaning = Cleaning…
admin-disk-word-images = images
admin-disk-word-containers = containers
admin-disk-word-stopped = stopped
admin-disk-badge-inuse = in use
admin-dashboard-metric-sessions-help = Sessions the replicas report currently serving.
admin-dashboard-metric-tracker-help = Sticky sessions the proxy is tracking in the heartbeat.
admin-landing-header = Header
admin-landing-portal-title = Portal title
admin-landing-portal-title-help = Shown at the top of the landing. Blank uses the config title (proxy.title).
admin-landing-portal-subtitle = Subtitle
admin-landing-portal-subtitle-help = The line under the title. Leave blank to hide it.
admin-landing-footer = Footer
admin-landing-footer-help = Text in the portal footer. Blank shows the version and wordmark.
admin-landing-default-theme = Default theme
admin-landing-default-theme-help = The initial theme for a first-time visitor. They can still switch.
admin-landing-visible-sections = Visible sections
admin-landing-show-search = Search bar
admin-landing-show-filters = Access filters (public/restricted)



admin-landing-theme-colors = Theme colors
admin-landing-theme-colors-help = Recolor the public portal's light and dark themes. Blank keeps the default.
admin-landing-theme-light = Light theme
admin-landing-theme-dark = Dark theme
admin-landing-theme-bg = Background
admin-landing-theme-text = Text
admin-landing-theme-accent = Accent

# Featured carousel (#506)
highlights-title = Featured
spec-form-featured = Feature this app
spec-form-featured-help = Shows the app in the Featured carousel at the top of the landing (when the toggle is on).
admin-landing-show-highlights = Show Featured carousel
admin-landing-show-highlights-help = Shows the carousel of featured apps above the filters. Hidden when nothing is featured.

# Groups page (#503, read-only)
admin-nav-groups = Groups
admin-groups-title = Groups
admin-groups-subtitle = Groups derived from apps (access-groups) and users — read-only. Edit on the user or the app.
admin-groups-members = Members
admin-groups-apps = Apps
admin-groups-public-title = Public apps
admin-groups-public-help = No group — visible to everyone
admin-groups-rename = Rename group
admin-groups-rename-prompt = New group name:
admin-groups-delete = Delete group
admin-groups-delete-confirm = Delete this group? It will be removed from every user and app that uses it.
admin-groups-remove-member = Remove from group
admin-groups-remove-member-confirm = Remove this member from the group?
admin-groups-add-member = Add member
admin-groups-pick-user = Choose a user…
admin-groups-create = Create group
admin-groups-new-name = Group name
admin-groups-new-group-title = New group:
admin-groups-flash-renamed = Group renamed.
admin-groups-flash-deleted = Group deleted.
admin-groups-flash-member-added = Member added.
admin-groups-flash-member-removed = Member removed.
admin-groups-flash-bad-input = Invalid input (empty name or unknown user).
admin-groups-empty = No groups yet. Groups appear when you set access-groups on an app or groups on a user.
admin-groups-no-members = No members
admin-groups-no-apps = No app restricted to this group

highlights-prev = Previous
highlights-next = Next

# Featured star toggle in the Apps table (#521)
admin-specs-col-featured = Featured
admin-specs-featured-on = Featured — click to remove
admin-specs-featured-off = Not featured — click to feature
admin-specs-featured-readonly = Featured is set in the config file

# Selective import (#557)
admin-import-preview-title = Confirm import
admin-import-preview-help = Pick which apps to import
admin-import-apps-label = apps
admin-import-warnings-label = warnings
admin-import-preview-none = The file contains no apps.
admin-import-select-all = Select all
admin-import-col-status = Status
admin-import-badge-new = New
admin-import-badge-new-help = Will be created (not in the panel)
admin-import-badge-update = Updates
admin-import-badge-update-help = Overwrites an app already in the panel
admin-import-confirm = Import selected
admin-import-load-file = Load file
admin-import-editor-placeholder = Paste your application.yml here…
admin-import-editor-empty = The preview appears here as you type.
