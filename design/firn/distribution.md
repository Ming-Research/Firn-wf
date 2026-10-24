Decision: Firn ships the full-LTO build because users should run the build used in performance comparisons.

Decision: The quickstart checker executes the document's marked shell blocks literally because maintaining a second command sequence would let the user instructions drift from the checked procedure.

Decision: The container keeps its AOF in a named volume at `/data` because the dataset must survive container replacement.

Decision: The container binds every IPv4 address internally because a published Docker port must reach the server.

Decision: The container runs as UID 10001 because its data writes and port 6379 need no root privileges.

Decision: Firn ships a dynamic glibc build at the Ubuntu 24.04 baseline because that environment is covered by the release dependency and startup checks; a selected deployment requiring an older host reopens the baseline.

Decision: The runtime image uses Debian trixie-slim because its glibc supports the Ubuntu 24.04 build without carrying a compiler toolchain; a new runtime dependency or loader incompatibility reopens the choice.

Decision: GitHub Releases and GHCR are Firn's only distribution channels because one repository-scoped GITHUB_TOKEN can publish both the program and image.

Decision: Only tags matching `^v[0-9]+\.[0-9]+\.[0-9]+$` are stable and move `latest` because every other accepted tag must remain an explicit prerelease selection.

Decision: Only pushed `v*` tags publish checked artifacts because validation runs must not create public releases.

Decision: Publication refuses an existing GitHub release or GHCR version tag because versioned downloads must retain their original bytes and checksums; an unverified existence check stops publication.

Decision: Publication pushes the image tags before creating the GitHub release because the default release lookup must not advertise a version whose image is still missing; a later release-creation failure leaves the image published and still protected from overwrite.

Rejected:
- The incremental developer build: rejected because it optimizes the runtime separately from the full-LTO build used in performance comparisons.
- Separately maintained quickstart test commands: rejected because their success would not detect an incorrect command in the user document.
- Data only in the container's writable layer: rejected because removing the container removes the dataset.
- The process's loopback bind inside the container: rejected because Docker's published port cannot reach that listener.
- A root container user: rejected because neither the data directory owned by UID 10001 nor port 6379 requires root authority.
- A static build or an older glibc baseline for this release: rejected because neither alternative is covered by the selected release build's dependency and startup evidence; a deployment that needs either reopens the choice.
- Debian bookworm-slim for the current program: rejected because its glibc 2.36 predates the Ubuntu 24.04 baseline of glibc 2.39.
- Docker Hub and a separate binary download host: rejected because they add publication authority beyond the repository-scoped GITHUB_TOKEN that covers both chosen channels.
- Treating tags without a prerelease hyphen as stable: rejected because it would promote names such as `v-preview` to the default release despite their lack of a stable version number.
- Publication on pull requests or manual dispatches: rejected because a validation request is not a release request.
- Replacing an existing version or proceeding after a failed existence query: rejected because either can overwrite the original version's bytes without establishing that the target is new.
- Creating the GitHub release before pushing its image: rejected because the default release lookup could expose a version that its documented image path cannot yet download.
