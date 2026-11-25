# Distribution

How firn is shipped: the program and container image a release holds, where they are published, and when a tag publishes them.

### decision release-ships-the-full-lto-build
The release workflow builds the shipped Linux x86-64 program with `make firn-lto`, the full-LTO build that performance comparisons measure.
- rejected: an agent proposed shipping the incremental developer build; the owner rejected it, because it optimizes the runtime apart from the full-LTO build that performance comparisons use, and users should run the build that was measured.

### decision quickstart-check-runs-the-document-literally
The quickstart checker runs the shell blocks marked in `docs/quickstart.md` literally, against both the tarball and the image.
- rejected: an agent proposed quickstart test commands kept apart from the document; the owner rejected it, because their success would not catch a wrong command in the user's document.

### decision container-keeps-the-aof-in-a-data-volume
The container keeps its AOF in a named volume at `/data`.
- rejected: an agent proposed keeping the data only in the container's writable layer; the owner rejected it, because removing the container would remove the dataset.

### decision container-binds-every-ipv4-address
Inside the container, firn binds every IPv4 address, `0.0.0.0`.
- rejected: an agent proposed keeping the process's loopback bind inside the container; the owner rejected it, because Docker's published port cannot reach that listener.

### decision container-runs-as-uid-10001
The container runs firn as UID 10001, which owns `/data`.
- rejected: an agent proposed a root container user; the owner rejected it, because neither the data directory, owned by UID 10001, nor port 6379 needs root.

### decision release-is-a-dynamic-glibc-build-at-ubuntu-24-04
The shipped program links glibc dynamically and is built on Ubuntu 24.04, so it needs glibc 2.39 or newer.
- rejected: an agent proposed a static build or an older glibc baseline; the owner rejected it, because the release's dependency and startup checks cover neither.

### decision runtime-image-is-debian-trixie-slim
The runtime image is built on Debian trixie-slim.
- rejected: an agent proposed Debian bookworm-slim; the owner rejected it, because its glibc 2.36 is older than the glibc 2.39 of the Ubuntu 24.04 baseline.

### decision release-channels-are-github-releases-and-ghcr
A release is published only to GitHub Releases and GHCR, both with the repository's own GITHUB_TOKEN.
- rejected: an agent proposed Docker Hub and a separate download host; the owner rejected it, because they add publishing authority beyond the repository-scoped GITHUB_TOKEN that covers both chosen channels.

### decision only-plain-version-tags-are-stable-releases
Only a tag matching `^v[0-9]+\.[0-9]+\.[0-9]+$` is a stable release and moves `latest`; every other accepted tag is a prerelease.
- rejected: an agent proposed treating tags without a prerelease hyphen as stable; the owner rejected it, because names such as `v-preview` would become the default release without a stable version number.

### decision only-pushed-v-tags-publish
Only a pushed tag starting with `v` publishes; pull requests and manual dispatches only build and check.
- rejected: an agent proposed publishing on pull requests or manual dispatches; the owner rejected it, because a validation request is not a release request.

### decision publication-refuses-an-existing-version
Publication stops when GitHub Releases or GHCR already has the version's tag, or when it cannot confirm that neither has it.
- rejected: an agent proposed replacing an existing version, or going on after a failed existence check; the owner rejected it, because either could overwrite a published version's bytes and checksums without showing that the target is new.

### decision publication-pushes-images-before-the-release
Publication pushes the image tags to GHCR before it creates the GitHub release.
- rejected: an agent proposed creating the GitHub release before pushing its image; the owner rejected it, because the default release lookup could then point to a version whose documented image cannot yet be pulled.