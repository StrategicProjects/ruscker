# Homebrew tap

`ruscker.rb` here is the **canonical** Homebrew formula. The published
formula lives in a separate tap repo so users can
`brew install strategicprojects/tap/ruscker`.

## One-time setup (maintainer)

1. Create a public repo **`StrategicProjects/homebrew-tap`** (the
   `homebrew-` prefix is what lets `brew tap strategicprojects/tap`
   resolve it).
2. Copy this formula into it as `Formula/ruscker.rb`:
   ```sh
   mkdir -p Formula
   cp /path/to/ruscker/packaging/homebrew/ruscker.rb Formula/ruscker.rb
   git add Formula/ruscker.rb && git commit -m "ruscker 0.1.0" && git push
   ```

## Installing (users)

```sh
brew install strategicprojects/tap/ruscker
# or:
brew tap strategicprojects/tap && brew install ruscker
```

- **Linux** (Linuxbrew): downloads the prebuilt static musl tarball
  that `release.yml` attaches to the release (amd64 / arm64).
- **macOS**: builds from source (`depends_on "rust" => :build`) — there
  is no prebuilt macOS binary yet. (Follow-up: add a macOS build job to
  `release.yml` and switch the formula to a prebuilt `url` + `bottle`.)

## Bumping on each release

Update in `ruscker.rb` (then copy to the tap, or let an automated PR do
it):

- `version "X.Y.Z"`
- the two **Linux** `sha256` values — from the release's
  `ruscker-X.Y.Z-linux-{amd64,arm64}.tar.gz.sha256` assets.
- the **macOS** source `sha256` — of
  `https://github.com/StrategicProjects/ruscker/archive/refs/tags/vX.Y.Z.tar.gz`
  (`curl -L … | shasum -a 256`).

Automating the bump (a `release.yml` step that opens a PR to the tap on
each `v*` tag) is a possible follow-up; manual for now.
