-- Postgres twin of migrations/0017 (#894). Durable provenance of the
-- container images Ruscker has managed, so the Disk panel removes/prunes
-- only Ruscker's own images, never a neighbour's idle cache. Survives spec
-- deletion (unlike spec_versions). Backfilled from the current catalog.
CREATE TABLE ruscker_images (
    image_ref   TEXT PRIMARY KEY,
    recorded_at TIMESTAMPTZ NOT NULL
);

INSERT INTO ruscker_images (image_ref, recorded_at)
SELECT container_image, now()
FROM specs
WHERE container_image IS NOT NULL AND trim(container_image) <> ''
ON CONFLICT (image_ref) DO NOTHING;
