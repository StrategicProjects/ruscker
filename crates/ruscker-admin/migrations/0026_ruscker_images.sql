-- Durable provenance of container images Ruscker has managed (#894).
-- An image ref recorded here is one a Ruscker spec referenced, or that
-- Ruscker pulled — so the Disk panel can offer to remove/prune ONLY
-- Ruscker's own images and never a side-by-side ShinyProxy's idle image
-- cache (irrecoverable on a host that can't re-pull the registry).
--
-- It must survive spec deletion (unlike spec_versions, which cascades),
-- so a deleted spec's now-orphaned image stays reclaimable. Keyed by the
-- image ref (tag or digest) exactly as written on specs. Backfill from the
-- current catalog so existing installs treat their apps' images as managed
-- immediately.
CREATE TABLE ruscker_images (
    image_ref   TEXT PRIMARY KEY,
    recorded_at TEXT NOT NULL
);

INSERT OR IGNORE INTO ruscker_images (image_ref, recorded_at)
SELECT container_image, datetime('now')
FROM specs
WHERE container_image IS NOT NULL AND trim(container_image) <> '';
