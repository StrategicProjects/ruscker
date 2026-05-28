-- Seed a single "welcome" landing block on fresh installs (#187 #4+#5).
-- Postgres twin of the SQLite migration with the same number.
-- Idempotent via `INSERT ... SELECT ... WHERE NOT EXISTS`: on a database
-- that already has any landing_blocks row the seed is skipped.
INSERT INTO landing_blocks (
    id, slot, position, enabled, title, html, csp_origins,
    created_at, updated_at
)
SELECT
    'welcome-seed',
    'bottom',
    0,
    TRUE,
    'Welcome (auto-seeded on first install)',
    '<section class="ruscker-welcome">' ||
      '<h2 class="ruscker-welcome-heading">Welcome to Ruscker</h2>' ||
      '<p class="ruscker-welcome-sub">Ruscker can host these and more. Add your first spec from the admin panel.</p>' ||
      '<ul class="ruscker-welcome-techs">' ||
        '<li class="ruscker-welcome-tech" style="--tc:#447099"><span>Shiny</span></li>' ||
        '<li class="ruscker-welcome-tech" style="--tc:#FF4B4B"><span>Streamlit</span></li>' ||
        '<li class="ruscker-welcome-tech" style="--tc:#3F4F75"><span>Dash</span></li>' ||
        '<li class="ruscker-welcome-tech" style="--tc:#75AADB"><span>R Markdown</span></li>' ||
        '<li class="ruscker-welcome-tech" style="--tc:#39729E"><span>Quarto</span></li>' ||
        '<li class="ruscker-welcome-tech" style="--tc:#F37726"><span>Jupyter</span></li>' ||
        '<li class="ruscker-welcome-tech" style="--tc:#B4C66F"><span>Bokeh</span></li>' ||
        '<li class="ruscker-welcome-tech" style="--tc:#B43E58"><span>Plumber</span></li>' ||
        '<li class="ruscker-welcome-tech" style="--tc:#009688"><span>FastAPI</span></li>' ||
        '<li class="ruscker-welcome-tech" style="--tc:#6E40C9"><span>Voilà</span></li>' ||
      '</ul>' ||
      '<a class="ruscker-welcome-docs" href="https://ruscker.com" target="_blank" rel="noopener">Read the Ruscker manual →</a>' ||
    '</section>',
    '',
    now(),
    now()
WHERE NOT EXISTS (SELECT 1 FROM landing_blocks);
