### Landing page — fr-FR

landing-title = Ruscker
landing-subtitle = Portail d'applications et d'APIs
landing-signin = Connexion
landing-panel = Panneau
landing-signout = Déconnexion
landing-signed-in-as = { $user }

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
role-current = Votre niveau d'accès
role-viewer = Lecteur
role-editor = Éditeur
role-admin = Administrateur

# — Connexion identifiant/mot de passe + bootstrap par token (#107)
admin-login-help-user = Connectez-vous avec votre identifiant et votre mot de passe.
admin-login-username-label = Identifiant
admin-login-username-placeholder = votre identifiant
admin-login-password-label = Mot de passe
admin-login-password-placeholder = votre mot de passe
admin-login-error-credentials = Identifiant ou mot de passe invalide.
admin-login-use-token = Se connecter avec le token administrateur
admin-login-use-password = Revenir à la connexion par mot de passe
# — Configuration du premier admin
admin-setup-title = Créez le compte administrateur
admin-setup-help = C'est le premier lancement. Choisissez un identifiant et un mot de passe pour l'administrateur.
admin-setup-error = Impossible de créer le compte. Vérifiez les informations.
admin-setup-password-label = Mot de passe
admin-setup-submit = Créer l'administrateur
# — Changement de mot de passe / première connexion
admin-pw-title = Changer le mot de passe
admin-pw-help = Définissez un nouveau mot de passe pour votre compte.
admin-pw-first-prompt = Vous utilisez un mot de passe défini par un administrateur. Définissez un nouveau mot de passe pour continuer.
admin-pw-current-label = Mot de passe actuel
admin-pw-new-label = Nouveau mot de passe
admin-pw-confirm-label = Confirmer le mot de passe
admin-pw-error-current = Le mot de passe actuel est incorrect.
admin-pw-error-mismatch = Les mots de passe ne correspondent pas.
admin-pw-error-short = Le mot de passe doit comporter au moins 8 caractères.
admin-pw-submit = Enregistrer le mot de passe
admin-pw-reveal = Afficher/masquer le mot de passe
# — Gestion des utilisateurs (admin)
admin-nav-users = Utilisateurs
admin-users-title = Utilisateurs
admin-users-subtitle = Créez et gérez qui peut se connecter, et à quel niveau.
admin-users-new = Nouvel utilisateur
admin-users-create = Créer
admin-users-initial-password = Mot de passe initial
admin-users-initial-password-hint = L'utilisateur devra le changer à la première connexion.
admin-users-role = Niveau
admin-users-col-user = Utilisateur
admin-users-col-role = Niveau
admin-users-col-created = Créé le
admin-users-col-actions = Actions
admin-users-you = vous
admin-users-must-change = Utilise encore le mot de passe initial
admin-users-save-role = Enregistrer le niveau
admin-users-groups = Groupes
admin-users-groups-placeholder = analystes, gestionnaires
admin-users-groups-hint = Les groupes séparés par des virgules déterminent les applis restreintes visibles par l'utilisateur.
admin-users-col-groups = Groupes
admin-users-save-groups = Enregistrer les groupes
admin-users-new-password = nouveau mot de passe
admin-users-reset-password = Réinitialiser le mot de passe
admin-users-delete = Supprimer l'utilisateur
admin-users-confirm-delete = Supprimer cet utilisateur ?
admin-users-flash-created = Utilisateur créé.
admin-users-flash-saved = Modifications enregistrées.
admin-users-flash-deleted = Utilisateur supprimé.
admin-users-flash-last-admin = Impossible de supprimer ou rétrograder le dernier administrateur.
admin-users-flash-bad-input = Données invalides (identifiant vide ou mot de passe de moins de 8 caractères).
admin-users-flash-exists = Un utilisateur portant ce nom existe déjà.

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
admin-dashboard-col-host = Hôte
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
admin-login-title = Se connecter
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
admin-specs-duplicate = Dupliquer
admin-specs-config-badge = config
admin-specs-config-defined = Défini dans le YAML — lecture seule ici; modifiez le fichier
admin-specs-delete = Supprimer

# Spec form (new / edit)
spec-form-title-new = Nouvelle application
spec-form-crumb-new = Nouvelle
spec-form-crumb-edit = Modifier
spec-form-cancel = Retour aux applications
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
spec-form-error-number = Un champ numérique a une valeur non numérique.
spec-form-error-cpu = La limite CPU doit être un nombre positif (ex. 0.5).
spec-form-error-memory = La limite mémoire doit être une taille comme 512m ou 1.5g.
spec-form-error-replica-range = Le max de réplicas doit être supérieur ou égal au min.

# Admin image library
admin-images-title = Bibliothèque de médias
admin-images-subtitle = PNG, JPEG et WebP sont convertis en WebP. Le SVG passe tel quel.
admin-images-drop-here = Cliquez pour choisir un fichier
admin-images-formats = PNG · JPEG · WebP · SVG · jusqu'à 10 Mo
admin-images-upload = Envoyer
admin-images-choose = Choisir une image
admin-images-builtin = Logos intégrés
admin-images-builtin-tag = intégré
admin-images-uploaded = Image envoyée :
admin-images-empty = Aucune image. Envoyez la première ci-dessus.
admin-images-delete = Supprimer
admin-images-delete-confirm = Supprimer cette image ? Les specs qui référencent le fichier afficheront le cover teinté.
admin-images-inuse = Utilisé
admin-images-inuse-help = Utilisé par une carte ou un logo de la landing
admin-images-delete-confirm-inuse = Cette image est UTILISÉE (une carte ou la landing). Supprimer quand même ?
admin-images-search = Rechercher des images…
admin-images-no-match = Aucune image ne correspond à la recherche.

# Admin credentials
admin-creds-title = Identifiants registry
admin-creds-subtitle = Les mots de passe sont chiffrés au repos avec AES-256-GCM. Ils n'apparaissent jamais dans le YAML ni dans le panneau après sauvegarde.
admin-creds-form-title = Ajouter / mettre à jour
admin-creds-name = Nom
admin-creds-name-help = Identifiant unique. Utilisez le même nom dans vos specs.
admin-creds-registry = Registry
admin-creds-username = Nom d'utilisateur
admin-creds-password = Mot de passe / jeton
admin-creds-password-help = Chiffré à l'enregistrement, jamais réaffiché. Ou indiquez une référence de variable d'environnement — résolue au pull, jamais stockée.
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
admin-landing-scope-help = Ces options (couleurs, textes d'intro, SEO, blocs personnalisés) s'appliquent à la page d'accueil publique en direct — enregistrées ici, affichées à la prochaine visite, sans redémarrage. C'est un ensemble fixe de réglages, pas un éditeur de CSS arbitraire.
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
status-title-new = Mis à jour récemment
status-title-updated = Mis à jour
status-title-none = Sans date de mise à jour
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

theme-light = Clair
theme-dark = Sombre
theme-auto = Automatique

# Top-right chrome cluster (#182 + #183)
chrome-cluster-label = Préférences de la page
chrome-theme-label = Thème
chrome-language-label = Langue
chrome-account-label = Compte
chrome-signin = Se connecter
chrome-signed-in-as-prefix = Connecté en tant que
chrome-panel = Panneau
chrome-change-password = Changer le mot de passe
chrome-signout = Se déconnecter

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
spec-form-choose-image = Choisir une image
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
spec-form-volumes-section = Volumes
spec-form-volumes = Montages de volume
spec-form-volumes-help = Un bind par ligne — /hôte:/conteneur (ajoutez :ro pour lecture seule). Ajoutez-en autant que nécessaire.
spec-help-volumes = Monte des répertoires de l'hôte dans le conteneur (ex. données persistantes, ou statiques servis par l'app). Admin uniquement ; monter des chemins de l'hôte équivaut à root.
spec-form-routing-section = Routage
spec-form-inject-base-href = Réécrire le HTML de l'app pour le sous-chemin
spec-form-inject-base-href-help = Activé par défaut. Ruscker réécrit <base href> et les URL relatives à la racine pour qu'une app qui se croit à la racine du serveur fonctionne sous son sous-chemin /app/. Désactivez uniquement si l'app lit X-Forwarded-Prefix et construit ses propres chemins.
spec-help-inject-base-href = Ruscker transmet toujours X-Forwarded-Prefix / X-Script-Name (et X-Forwarded-Proto/-Host). Des frameworks comme FastAPI (root_path), Dash et Streamlit peuvent s'auto-router avec — la réécriture HTML devient alors redondante.
spec-form-error-volume = Chaque volume doit être /hôte:/conteneur (optionnel :ro).
admin-nav-logs = Journaux
admin-proclog-title = Journaux
admin-proclog-subtitle = Journal récent du processus Ruscker (en direct).
admin-proclog-unavailable = Tampon de journal indisponible (le serveur a démarré sans la couche de journalisation).
admin-proclog-empty = Aucun journal capturé pour l'instant à ce niveau. Les nouveaux événements apparaissent ici au fur et à mesure ; lancez le serveur avec -v pour inclure les journaux de niveau info.

# ── spec-form advanced params (#211/#212) ──────────────────────────
spec-form-runtime-section = Runtime
spec-form-container-port = Port du conteneur
spec-help-container-port = Port sur lequel l'app écoute dans le conteneur. Vide = défaut par type (3838 pour Shiny). À définir pour Streamlit (8501), Dash (8050) ou Jupyter (8888).
spec-form-platform = Plateforme
spec-help-platform = Plateforme Docker (ex. : linux/amd64) pour exécuter une image d'une autre architecture par émulation. Vide = le démon choisit selon le manifeste.
spec-form-container-lifetime = Durée de vie du conteneur (min)
spec-help-container-lifetime = Limite souple en minutes avant recyclage du conteneur. Vide = pas de limite souple.
spec-form-stop-on-logout = Arrêter à la déconnexion
spec-help-stop-on-logout = Arrête le conteneur de l'utilisateur à sa déconnexion. Désactivé par défaut.
spec-form-env-section = Environnement + commande
spec-form-container-env = Variables d'environnement
spec-form-container-env-help = Une par ligne, NOM=valeur. Pour les secrets, référencez une variable d'environnement au lieu de coller la valeur.
spec-help-container-env = Injectées dans le conteneur (container-env). Vide = aucune. Pour les secrets, utilisez l'interpolation de variable d'environnement.
spec-form-container-cmd = Commande (remplacer)
spec-form-container-cmd-help = Un argument par ligne. Vide = utilise le CMD de l'image.
spec-help-container-cmd = Remplace la commande du conteneur (container-cmd), sous forme de liste d'arguments.
spec-form-registry-section = Registre (images privées)
spec-form-registry-domain = Domaine du registre
spec-help-registry-domain = Hôte du registre pour les images privées (ex. : docker.io, ghcr.io). Vide = Docker Hub.
spec-form-registry-username = Utilisateur
spec-help-registry-username = Utilisateur pour authentifier le téléchargement d'une image privée.
spec-form-registry-password = Mot de passe
spec-form-registry-password-keep = Vide conserve le mot de passe actuel
spec-help-registry-password = Utilisez une variable d'environnement — ne collez jamais le mot de passe en clair. Utilisé uniquement avec l'utilisateur.
spec-form-registry-credential = Identifiant enregistré
spec-help-registry-credential = Choisissez un identifiant nommé du coffre (page Identifiants) pour tirer des images privées. S'il est défini, il prime sur l'utilisateur/mot de passe en ligne.
spec-form-registry-help = Tirez une image privée en sélectionnant un identifiant enregistré. Créez et gérez les identifiants sur la page Identifiants — le mot de passe peut être littéral (chiffré) ou une référence de variable d'environnement.
spec-form-registry-inline-note = Cette app a des identifiants de registre en ligne (YAML importé). Ils sont conservés, mais préférez un identifiant enregistré ci-dessus.
spec-form-access-section = Accès
spec-form-access-groups = Groupes autorisés
spec-help-access-groups = Groupes qui peuvent voir et atteindre l'app (séparés par des virgules). Vide, avec utilisateurs aussi vide = ouvert à tous.
spec-form-access-users = Utilisateurs autorisés
spec-help-access-users = Utilisateurs qui peuvent voir et atteindre l'app (séparés par des virgules).
spec-form-access-help = Les deux vides = carte ouverte à tous (y compris anonyme). Avec une valeur, seuls les utilisateurs connectés correspondants — et toujours les admins.
spec-form-cpu-request = Réservation CPU
spec-help-cpu-request = Réservation souple de CPU en cœurs (container-cpu-request). Vide = pas de réservation.
spec-form-memory-request = Réservation mémoire
spec-help-memory-request = Réservation souple de mémoire, ex. : 256m. Vide = pas de réservation.
spec-form-max-body-size = Taille max. du corps
spec-help-max-body-size = Limite par spec du corps des requêtes, ex. : 10m. Vide = utilise la limite globale.
spec-form-scale-up = Seuil de scale-up
spec-help-scale-up = Fraction d'utilisation (0–1) qui déclenche la création d'une réplique. Vide = défaut du scaler.
spec-form-scale-down = Seuil de scale-down
spec-help-scale-down = Fraction d'utilisation (0–1) sous laquelle une réplique est retirée. Vide = défaut du scaler.
spec-form-scale-down-grace = Délai de scale-down (s)
spec-help-scale-down-grace = Secondes sous le seuil avant de retirer la réplique. Vide = défaut.
spec-form-drain-timeout = Délai de drainage (s)
spec-help-drain-timeout = Secondes pour drainer les sessions d'une réplique avant de l'arrêter. Vide = défaut.
spec-form-routing-strategy = Stratégie de routage
spec-help-routing-strategy = Comment l'équilibreur choisit une réplique. Vide = défaut par type (least-connections pour les apps, round-robin pour les API).
spec-form-routing-default = Défaut (par type)
spec-form-placement = Placement (multi-hôte)
spec-help-placement = Comment répartir les répliques entre hôtes Docker. Vide = spread. Pertinent seulement avec proxy.hosts.
spec-form-placement-default = Défaut (spread)
spec-form-anti-affinity = Anti-affinité
spec-help-anti-affinity = Préfère des hôtes distincts pour les répliques de ce spec (multi-hôte). Désactivé par défaut.
spec-form-error-port = Le port doit être un nombre entre 1 et 65535.
spec-form-error-threshold = Le seuil doit être un nombre entre 0 et 1.

# ── spec-form image picker (#213) ──────────────────────────────────
spec-form-logo-upload = Téléverser une image
spec-form-gallery-more = Afficher plus
spec-form-logo-clear = Retirer
spec-form-logo-none = Pas d'image — une teinte selon le type est utilisée.
spec-form-logo-builtin = Logos intégrés
spec-form-logo-path-advanced = Avancé : coller un chemin ou une URL
spec-form-cover-image = Image
spec-form-cover-image-help = Choisissez une image de la bibliothèque (ou téléversez-en une) comme fond de la carte.
admin-proclog-tail-note = Affichage des lignes les plus récentes
admin-proclog-download = Télécharger le journal complet

landing-empty = Rien ici pour l'instant.

admin-landing-style = Style (CSS)
admin-landing-custom-css = CSS personnalisé
admin-landing-custom-css-help = CSS injecté à la fin du <head> de la landing — remplace les styles intégrés. Ciblez des classes/variables stables (.rcard, .tint-*, --color-link, variables d'en-tête). De confiance (admin) ; attention à ne pas casser la mise en page.
admin-landing-logos = Logos en-tête/pied
admin-landing-logos-help = Ajoutez des logos à l'en-tête ou au pied. Plusieurs au même alignement s'affichent côte à côte.
admin-landing-logo-header = En-tête
admin-landing-logo-footer = Pied
admin-landing-logo-left = Gauche
admin-landing-logo-center = Centre
admin-landing-logo-right = Droite
admin-landing-logo-link = Lien (optionnel)
admin-landing-logo-height = Hauteur (px)
admin-landing-logo-add = Ajouter un logo

# — Gestion du disque (admin) #453
admin-nav-disk = Disque
admin-disk-title = Disque
admin-disk-subtitle = Récupérez de l'espace des conteneurs arrêtés et des images inutilisées.
admin-disk-backend-missing = Le backend Docker n'est pas connecté — lancez le serveur avec `--docker` pour gérer le disque.
admin-disk-containers-heading = Conteneurs Ruscker
admin-disk-prune = Supprimer les arrêtés
admin-disk-prune-confirm = Supprimer tous les conteneurs Ruscker arrêtés ?
admin-disk-no-containers = Aucun conteneur géré par Ruscker.
admin-disk-col-container = Conteneur
admin-disk-col-app = App
admin-disk-col-image = Image
admin-disk-col-status = Statut
admin-disk-running = en cours
admin-disk-remove = Supprimer
admin-disk-remove-confirm = Supprimer ce conteneur ?
admin-disk-remove-running-confirm = Ce conteneur est en cours d'exécution. L'arrêter et le supprimer ?
admin-disk-images-heading = Images
admin-disk-images-total = Total
admin-disk-images-note = Le total peut compter les couches partagées plusieurs fois. Seules les images inutilisées peuvent être supprimées (sans forcer).
admin-disk-no-images = Aucune image locale.
admin-disk-col-id = ID
admin-disk-col-size = Taille
admin-disk-col-usage = Utilisation
admin-disk-used-by-spec = utilisée par une app
admin-disk-used-by-container = utilisée par un conteneur
admin-disk-unused = inutilisée
admin-disk-in-use-hint = Utilisée — suppression impossible.
admin-disk-remove-image-confirm = Supprimer cette image ?
admin-disk-flash-removed = Supprimé.
admin-disk-flash-pruned = Conteneurs arrêtés supprimés.
admin-disk-flash-nothing = Rien à supprimer.
admin-disk-flash-error = L'opération a échoué. Consultez les journaux.
admin-disk-prune-images = Supprimer les inutilisées
admin-disk-prune-images-confirm = Supprimer toutes les images inutilisées ?
admin-disk-flash-images-pruned = Images inutilisées supprimées.
admin-disk-cleaning = Nettoyage…
admin-dashboard-metric-sessions-help = Sessions que les réplicas déclarent servir actuellement.
admin-dashboard-metric-tracker-help = Sessions persistantes (sticky) suivies par le proxy dans le heartbeat.
admin-landing-header = En-tête
admin-landing-portal-title = Titre du portail
admin-landing-portal-title-help = Affiché en haut de la landing. Vide : utilise le titre de la config (proxy.title).
admin-landing-portal-subtitle = Sous-titre
admin-landing-portal-subtitle-help = La ligne sous le titre. Laissez vide pour le masquer.
