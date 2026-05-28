-- Seed a single "welcome" landing block on fresh installs (#187 #4+#5).
-- Idempotent via `INSERT ... SELECT ... WHERE NOT EXISTS`: on a database
-- that already has any landing_blocks row (e.g. the operator already
-- populated one via /admin/blocks) the seed is skipped.
--
-- The block sits in the `bottom` slot (after the card grid). Content is
-- a short welcome heading + a strip of framework name chips Ruscker can
-- host + a link out to the manual. Operators can edit or delete it via
-- /admin/blocks like any other block; that's why the URL points at the
-- documented Ruscker docs site (admin can swap to ruscker.com once the
-- domain is live).
INSERT INTO landing_blocks (
    id, slot, position, enabled, title, html, csp_origins,
    created_at, updated_at
)
SELECT
    'welcome-seed',
    'bottom',
    0,
    1,
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
    datetime('now'),
    datetime('now')
WHERE NOT EXISTS (SELECT 1 FROM landing_blocks);
