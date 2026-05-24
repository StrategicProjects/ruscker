### Landing page — fr-FR

landing-title = Surveillance Stratégique
landing-subtitle = Portail d'applications et d'APIs

filter-search-placeholder = Rechercher une application…
filter-access-all = Tous
filter-access-public = Publics
filter-access-restricted = Restreints
filter-clear = Effacer les filtres

type-all = Tous
type-app = Applications
type-package = Paquets
type-talk = Présentations
type-report = Rapports
type-api = APIs

type-app-abbr = APP
type-talk-abbr = PRS
type-report-abbr = RAP
type-package-abbr = PKG
type-api-abbr = API
type-link-abbr = LNK

# Admin shell
admin-nav-dashboard = Tableau de bord
admin-nav-apps = Applications
admin-nav-images = Images
admin-nav-credentials = Identifiants
admin-nav-landing = Portail
admin-nav-audit = Journal
admin-nav-portal = Retour au portail
admin-nav-logout = Déconnexion

# Admin dashboard
admin-dashboard-title = Tableau de bord
admin-dashboard-subtitle = État des conteneurs et des sessions en temps réel
admin-dashboard-metric-containers = Conteneurs
admin-dashboard-metric-sessions = Sessions actives
admin-dashboard-metric-specs = Applications avec répliques
admin-dashboard-metric-tracker = Sessions suivies
admin-dashboard-replicas-heading = Répliques actives
admin-dashboard-no-replicas = Aucune réplique en cours. Les répliques apparaissent ici lorsque le scaler garantit le minimum configuré ou lorsqu'une requête déclenche un démarrage à froid.
admin-dashboard-col-spec = Application
admin-dashboard-col-state = État
admin-dashboard-col-uptime = Uptime
admin-dashboard-col-sessions = Sessions
admin-dashboard-col-container = Conteneur
admin-dashboard-col-cpu = CPU
admin-dashboard-col-memory = Mémoire
admin-dashboard-metric-memory = Mémoire utilisée
admin-dashboard-metrics-pending = en attente de la première lecture
admin-dashboard-state-ready = prêt
admin-dashboard-state-starting = démarrage
admin-dashboard-state-draining = drainage
admin-dashboard-state-stopped = arrêté
admin-dashboard-state-failed = échec
admin-dashboard-backend-missing = Le backend Docker n'est pas connecté — démarrez le serveur avec `--docker` pour voir les conteneurs ici.

# Admin login
admin-login-title = Accès admin
admin-login-help = Saisissez le jeton admin défini dans RUSCKER_ADMIN_TOKEN.
admin-login-token-label = Jeton
admin-login-token-placeholder = Collez le jeton ici
admin-login-submit = Se connecter
admin-login-error-wrong = Jeton incorrect. Réessayez.
admin-login-back-portal = ← portail public

# Apps list
admin-specs-title = Applications
admin-specs-subtitle = Catalogue de specs dans la base
admin-specs-empty = Aucune application. Utilisez { $cmd } pour importer un YAML.
admin-specs-add = Ajouter une application
admin-specs-col-id = ID
admin-specs-col-name = Nom
admin-specs-col-kind = Type
admin-specs-col-state = État
admin-specs-col-updated = Mis à jour
admin-specs-col-version = Version
admin-specs-col-actions = Actions
admin-specs-filter-search = Rechercher par id ou nom…
admin-specs-filter-kind-all = Tous les types
admin-specs-filter-state-all = Actifs et inactifs
admin-specs-edit = Modifier
admin-specs-delete = Supprimer

# Spec form (new / edit)
spec-form-title-new = Nouvelle application
spec-form-crumb-new = Nouvelle
spec-form-crumb-edit = Modifier
spec-form-cancel = Annuler
spec-form-save = Enregistrer
spec-form-kind = Type
spec-form-kind-app = App conteneur
spec-form-kind-talk = Présentation
spec-form-kind-report = Rapport
spec-form-kind-package = Paquet
spec-form-kind-api = API
spec-form-kind-link = Lien externe
spec-form-identity = Identité
spec-form-id = ID
spec-form-id-help-new = Choisi par l'opérateur. Apparaît à /app/<id>/.
spec-form-id-help-edit = L'ID est immuable une fois créé.
spec-form-name = Nom d'affichage
spec-form-desc = Description
spec-form-visual = Visuel
spec-form-logo = Logo de la carte
spec-form-logo-help = URL ou chemin /assets/img/foo.png. Voir docs/IMAGES.md.
spec-form-access = Accès
spec-form-state = État
spec-form-state-active = Actif
spec-form-state-inactive = Inactif
spec-form-tema = Thème
spec-form-container = Conteneur
spec-form-image = Image Docker
spec-form-seats = Sessions/conteneur
spec-form-lifetime = Durée max. (min)
spec-form-lifetime-help = 360 = 6 heures
spec-form-link-section = Lien externe
spec-form-link = URL cible
spec-form-meta = Métadonnées
spec-form-updated = Mis à jour le
spec-form-updated-help = Vide pour utiliser la date d'aujourd'hui.
spec-form-preview = Aperçu de la carte
spec-form-preview-help = Mise à jour en direct.
spec-form-actions = Actions
spec-form-delete = Supprimer l'application
spec-form-delete-confirm = Êtes-vous sûr ? Cette action est irréversible.

spec-form-error-id-required = L'ID est obligatoire.
spec-form-error-id-shape = L'ID doit commencer par une lettre et contenir uniquement lettres, chiffres, "_" et "-".
spec-form-error-id-duplicate = Une application avec cet ID existe déjà.
spec-form-error-name-required = Le nom d'affichage est obligatoire.

# Admin image library
admin-images-title = Bibliothèque d'images
admin-images-subtitle = PNG, JPEG et WebP sont convertis en WebP. Le SVG passe tel quel.
admin-images-drop-here = Cliquez pour choisir un fichier
admin-images-formats = PNG · JPEG · WebP · SVG · jusqu'à 10 Mo
admin-images-upload = Envoyer
admin-images-uploaded = Image envoyée :
admin-images-empty = Aucune image. Envoyez la première ci-dessus.
admin-images-delete = Supprimer
admin-images-delete-confirm = Supprimer cette image ? Les specs qui référencent le fichier afficheront le cover teinté.

# Admin credentials
admin-creds-title = Identifiants registry
admin-creds-subtitle = Les mots de passe sont chiffrés au repos avec AES-256-GCM. Ils n'apparaissent jamais dans le YAML ni dans le panneau après sauvegarde.
admin-creds-form-title = Ajouter / mettre à jour
admin-creds-name = Nom
admin-creds-name-help = Identifiant unique. Utilisez le même nom dans vos specs.
admin-creds-registry = Registry
admin-creds-username = Nom d'utilisateur
admin-creds-password = Mot de passe / jeton
admin-creds-password-help = Chiffré à l'enregistrement, jamais réaffiché.
admin-creds-save = Enregistrer
admin-creds-saved = Identifiant enregistré :
admin-creds-empty = Aucun identifiant enregistré.
admin-creds-delete = Supprimer
admin-creds-delete-confirm = Supprimer cet identifiant ?
admin-creds-col-name = Nom
admin-creds-col-registry = Registry
admin-creds-col-username = Utilisateur
admin-creds-col-created = Créé le
admin-creds-key-missing-title = RUSCKER_MASTER_KEY n'est pas configurée
admin-creds-key-missing-help = Le store d'identifiants a besoin d'une clé de 32 octets en hex (64 chars) ou base64 (44 chars). Générez-en une avec :

# Admin landing editor
admin-landing-title = Éditeur du portail
admin-landing-crumb = Paramètres · Page d'accueil
admin-landing-subtitle = Personnalisez le portail public. Les modifications s'appliquent au prochain rafraîchissement du visiteur.
admin-landing-open-portal = Ouvrir le portail
admin-landing-save = Enregistrer
admin-landing-saved = Paramètres enregistrés. Rechargez le portail public pour voir.
admin-landing-colors = Couleurs de l'en-tête
admin-landing-header-bg = Couleur de fond
admin-landing-bg-help = Vide = utilise la couleur par défaut du thème (clair/sombre).
admin-landing-header-fg = Couleur du texte
admin-landing-clear = Effacer
admin-landing-intro = Texte d'introduction (par défaut)
admin-landing-intro-default = Par défaut (fallback pour toutes les langues)
admin-landing-intro-default-placeholder = Bienvenue sur le portail…
admin-landing-intro-help = Affiché entre l'en-tête et les filtres. Vide = pas de texte.
admin-landing-intro-locales = Texte d'introduction par langue
admin-landing-intro-pt = Portugais
admin-landing-intro-en = Anglais
admin-landing-intro-es = Espagnol
admin-landing-intro-fr = Français
admin-landing-preview = Aperçu du portail
admin-landing-preview-help = Approximation visuelle de l'en-tête et de l'intro. Les cartes et filtres ressemblent au portail réel.
admin-landing-preview-empty = (pas de texte d'introduction)
admin-landing-future-title = Bientôt
admin-landing-future-help = Éditeur de logos, réorganisation des sections, blocs HTML personnalisés, SEO/analytics et meta tags. Pour l'instant ces champs suivent le YAML.

# Admin audit log
admin-audit-title = Journal d'audit
admin-audit-subtitle = Tous les changements admin, du plus récent au plus ancien. Limité à 100 événements par requête.
admin-audit-family = Famille
admin-audit-family-all = Toutes les actions
admin-audit-family-spec = Applications
admin-audit-family-image = Images
admin-audit-family-credential = Identifiants
admin-audit-family-landing = Portail
admin-audit-actor = Auteur
admin-audit-actor-all = Tous les auteurs
admin-audit-target-placeholder = Rechercher une cible (ex : spec:auroraprime)
admin-audit-apply = Appliquer
admin-audit-empty = Aucun changement — ou le filtre ne correspond à rien.
admin-audit-limit-hint = Affichage des 100 plus récents. Affinez le filtre pour réduire.

card-cta-open = Ouvrir
card-cta-link = Accéder
card-cta-open-app = Ouvrir l'application
card-cta-open-talk = Ouvrir la présentation
card-cta-open-report = Ouvrir le rapport
card-cta-open-package = Ouvrir la documentation
card-cta-open-api = Voir la documentation
card-updated = Mis à jour le { $date }
status-new = nouveau { $date }
status-updated = mis à jour { $date }
sort-label = Trier
sort-recent = Récents
sort-name = Nom
search-shortcut = ⌘ K

filter-theme-label = Thème
filter-theme-all = Tous les thèmes
filter-status-active = Actifs uniquement
filter-status-all = Actifs et inactifs
filter-status-inactive-only = Inactifs uniquement
card-state-active = Disponible
card-state-inactive = Indisponible
card-access-public = Accès public
card-access-restricted = Accès restreint

footer-language = Langue
footer-theme = Thème
theme-light = Clair
theme-dark = Sombre
theme-auto = Automatique
