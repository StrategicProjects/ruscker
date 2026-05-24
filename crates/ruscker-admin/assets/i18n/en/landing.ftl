### Landing page — en-US

landing-title = Strategic Monitoring
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
spec-form-access = Access
spec-form-state = State
spec-form-state-active = Active
spec-form-state-inactive = Inactive
spec-form-tema = Theme
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
admin-landing-future-title = Coming soon
admin-landing-future-help = Logo editor, section reordering, custom HTML blocks, SEO/analytics and meta tags. For now those fields follow the YAML.

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
admin-audit-target-placeholder = Search target (e.g. spec:auroraprime)
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

filter-theme-label = Theme
filter-theme-all = All themes
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
