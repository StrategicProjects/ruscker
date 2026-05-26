### Landing page — fr-FR

landing-title = Ruscker
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
admin-nav-images = Médias
admin-nav-credentials = Identifiants
admin-nav-landing = Portail
admin-nav-blocks = Blocs
admin-blocks-title = Blocs HTML
admin-blocks-subtitle = Extraits HTML personnalisés rendus sur la landing (emplacements haut/bas).
admin-blocks-new = Nouveau bloc
admin-blocks-empty = Aucun bloc pour l'instant.
admin-blocks-col-slot = Emplacement
admin-blocks-col-title = Titre
admin-blocks-col-status = Statut
admin-blocks-enabled = actif
admin-blocks-disabled = inactif
admin-blocks-edit = éditer
admin-blocks-delete = supprimer
admin-blocks-delete-confirm = Supprimer ce bloc ?
admin-blocks-move-up = Monter
admin-blocks-move-down = Descendre
admin-blocks-form-new = Nouveau bloc
admin-blocks-form-edit = Modifier le bloc
admin-blocks-slot = Emplacement
admin-blocks-slot-help = Où le bloc apparaît sur la landing.
admin-blocks-slot-top = Haut (après l'en-tête)
admin-blocks-slot-bottom = Bas (après la grille)
admin-blocks-title-label = Titre (interne)
admin-blocks-title-placeholder = Un libellé pour reconnaître ce bloc
admin-blocks-html = HTML
admin-blocks-html-help = Rendu sans échappement sur la landing — n'utilisez que des sources de confiance.
admin-blocks-origins = Origines autorisées (CSP)
admin-blocks-origins-help = Domaines séparés par des espaces autorisés dans la CSP de la landing (ex. https://example.com).
admin-blocks-enabled-label = Actif (afficher sur la landing)
admin-blocks-save = Enregistrer
admin-blocks-cancel = Annuler
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
spec-form-logo-pick-help = Ou choisissez une image déjà importée dans la médiathèque.
spec-form-access = Accès
spec-form-state = État
spec-form-state-active = Actif
spec-form-state-inactive = Inactif
spec-form-subject = Sujet
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
admin-images-title = Bibliothèque de médias
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
admin-landing-seo = SEO et partage
admin-landing-seo-title = Titre de la page (SEO)
admin-landing-seo-title-placeholder = Par défaut : titre du portail
admin-landing-seo-title-help = Définit le titre de l'onglet et og:title. Vide utilise le titre par défaut du portail.
admin-landing-seo-description = Description (meta description)
admin-landing-seo-description-placeholder = Résumé pour les moteurs de recherche et les réseaux sociaux
admin-landing-seo-description-help = Utilisé dans la meta description et og:description. Vide reprend le texte d'introduction.
admin-landing-og-image = Image de partage (og:image)
admin-landing-og-image-help = URL ou chemin (ex. /assets/img/og.png) affiché lors du partage sur les réseaux sociaux.
admin-landing-analytics = Analytics
admin-landing-analytics-html = Snippet analytics
admin-landing-analytics-html-help = HTML injecté dans le <head> de la landing (ex. une balise <script> Plausible/Matomo/GA). Rendu sans échappement — n'utilisez que des sources de confiance.
admin-landing-analytics-origins = Origines autorisées (CSP)
admin-landing-analytics-origins-help = Domaines séparés par des espaces (ex. https://plausible.io) autorisés dans la CSP de la landing pour que le script se charge et envoie ses données.
admin-landing-future-title = Blocs HTML
admin-landing-future-help = Gérez les blocs HTML personnalisés (bannières, avis) dans la section Blocs du menu.

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
admin-audit-target-placeholder = Rechercher une cible (ex : spec:sales-dashboard)
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

filter-subject-label = Sujet
filter-subject-all = Tous les sujets
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

# Admin logs viewer
admin-logs-title = Logs du conteneur
admin-logs-back = Retour au tableau de bord
admin-logs-replica = Réplique
admin-logs-empty = Pas encore de sortie de log pour cette réplique.
admin-logs-tail-note = Affichage des dernières lignes (les plus récentes en bas).

# Dashboard replica actions
admin-dashboard-action-stop = Arrêter
admin-dashboard-action-restart = Redémarrer
admin-dashboard-confirm-stop = Arrêter cette réplique ? L auto-scaler peut la recréer si le minimum configuré l exige.
admin-dashboard-confirm-restart = Redémarrer cette réplique ? La session active sera perdue.
admin-logs-follow = En direct
admin-logs-follow-stop = Arrêter

# Admin YAML import
admin-import-button = Importer YAML
admin-import-title = Importer une configuration YAML
admin-import-help = Choisissez un application.yml ShinyProxy ou Ruscker. L import est idempotent et ne supprime jamais les specs existants.
admin-import-file = Fichier .yml / .yaml
admin-import-submit = Importer
admin-import-cancel = Annuler
admin-import-ok = Import terminé : { $created } créés, { $updated } mis à jour, { $unchanged } inchangés.
admin-import-ok-warnings = { $warnings } avertissement(s) de validation — vérifiez les identifiants intégrés et les noms vides.
admin-import-err = Échec de l import : { $msg }

# Gradient builder
admin-grad-solid = Uni
admin-grad-gradient = Dégradé
admin-grad-linear = Linéaire
admin-grad-radial = Radial
admin-grad-add-stop = Ajouter une couleur
admin-grad-remove-stop = Retirer la couleur

# Spec form — card cover
spec-form-cover = Couverture de la carte
spec-form-cover-auto = Auto (teinte du type)
spec-form-cover-auto-help = Utilise la teinte par défaut du type. Choisissez Uni ou Dégradé pour personnaliser.

# ── Formulaire de spec : section avancée + aide par champ (#2) ─────
spec-form-advanced = Avancé
spec-form-advanced-hint = Tout est optionnel — laissez vide pour garder la valeur par défaut.
spec-form-api-section = API
spec-form-scaling-section = Mise à l'échelle
spec-form-resources-section = Ressources
spec-form-lifecycle-section = Cycle de vie
spec-form-api-port = Port du conteneur
spec-form-api-rate-limit = Limite de débit
spec-form-api-docs-path = Chemin des docs
spec-form-api-health-path = Chemin de santé
spec-form-api-cors = Activer CORS permissif
spec-form-min-replicas = Réplicas min.
spec-form-max-replicas = Réplicas max.
spec-form-concurrent = Requêtes par réplica
spec-form-cpu-limit = Limite CPU
spec-form-memory-limit = Limite mémoire
spec-form-heartbeat = Délai de heartbeat (ms)
spec-help-kind = Le type d'élément. Détermine le routage, le badge de la carte et si un conteneur est démarré.
spec-help-id = Identifiant stable utilisé dans l'URL (/app/<id>). Minuscules, chiffres, « - » et « _ » ; non modifiable après création.
spec-help-name = Le titre affiché sur la carte.
spec-help-desc = Brève description sur la carte. Le HTML en ligne (ex. un lien) est autorisé.
spec-help-logo = Image de la carte — un chemin sous /assets/img/ ou une URL externe. Vide : une teinte générée.
spec-help-cover = Fond de la carte : teinte automatique par type, couleur unie ou dégradé.
spec-help-access = Affiche un cadenas (restreint) ou ouvert (public). Visuel seulement — le MVP n'impose pas d'authentification.
spec-help-state = Les cartes actives s'affichent ; les inactives sont masquées.
spec-help-subject = Thème/domaine utilisé par le filtre Sujet de la page (ex. « Ventes », « Recherche »).
spec-help-image = Image Docker à exécuter, sous la forme dépôt:tag (ex. org/app:latest).
spec-help-seats = Combien de sessions simultanées un conteneur sert avant d'en démarrer un autre.
spec-help-lifetime = Plafond strict, en minutes, de la durée d'exécution d'un conteneur avant recyclage.
spec-help-link = URL de destination des cartes de lien externe — cliquer navigue ici.
spec-help-updated = Date affichée sur la carte (JJ/MM/AAAA). Vide : la date du jour est apposée.
spec-help-api-port = Port d'écoute de l'API dans le conteneur. Par défaut 8080.
spec-help-api-rate-limit = Limite par client au proxy, sous la forme N/unité (ex. 100/min, 5/s). Au-delà : 429. Vide = sans limite.
spec-help-api-docs-path = Chemin où l'API sert la doc OpenAPI/Swagger. Par défaut /__docs__.
spec-help-api-health-path = Chemin sondé pour la disponibilité avant qu'une réplica rejoigne le pool. Par défaut /__healthz__.
spec-help-api-cors = Ajoute des en-têtes CORS permissifs et répond au préflight. Désactivé par défaut.
spec-help-min-replicas = Conteneurs gardés actifs en permanence — le pool ne descend jamais en dessous. Par défaut 0.
spec-help-max-replicas = Plafond jusqu'auquel l'auto-scaler peut monter. Vide = illimité.
spec-help-concurrent = Requêtes qu'une réplica d'API gère avant que le scaler en ajoute une.
spec-help-cpu-limit = CPU max en cœurs fractionnaires (ex. 0,5 = un demi-cœur). Vide = illimité.
spec-help-memory-limit = Mémoire max, ex. 512m ou 1.5g. Vide = illimité.
spec-help-heartbeat = Délai de session inactive en millisecondes ; -1 = jamais. Vide = valeur globale.
admin-blocks-slot-empty = Aucun bloc dans cet emplacement.
admin-blocks-drag-hint = Glissez par la poignée pour réordonner les blocs dans un emplacement.
