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
admin-nav-apps = Applications
admin-nav-images = Images
admin-nav-credentials = Identifiants
admin-nav-landing = Portail
admin-nav-audit = Journal
admin-nav-portal = Retour au portail
admin-nav-logout = Déconnexion

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
