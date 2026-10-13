Decision: Firn ships the full-LTO build used in performance comparisons because users should run the measured build, instead of the incremental developer build that optimizes the runtime separately.

Decision: The quickstart checker executes the document's marked shell blocks literally because separately maintained test commands can pass while user instructions drift, instead of keeping a second command sequence in the checker.

Decision: The quickstart checker independently verifies client replies and persisted AOF contents because redis-cli can exit successfully after a server error, instead of accepting command exit codes alone.

Decision: The container keeps its AOF in a named volume at `/data` because replacing a container must preserve the dataset, instead of keeping it only in the container's writable layer.

Decision: The container binds every IPv4 address internally because its loopback default is unreachable through a published Docker port, instead of retaining the process's loopback bind inside the container.

Decision: The container runs as a non-root user because the server needs access only to its data directory, instead of granting it root privileges.

Decision: Firn ships a dynamic glibc build at the Ubuntu 24.04 baseline because it follows the established Linux CI toolchain and checks its runtime dependencies there, instead of a static build or an older distribution baseline.

Decision: The runtime image uses Debian trixie-slim because its glibc supports the Ubuntu 24.04 build without a compiler toolchain, instead of Debian bookworm-slim whose glibc predates that baseline; a new runtime dependency or loader incompatibility reopens the choice.

Decision: GitHub Releases and GHCR are Firn's only distribution channels because they share the project's existing GitHub credentials and permissions, instead of adding another download host or image registry.

Decision: Only tags without a hyphen after `vX.Y.Z` move `latest` because the default quickstart should select a non-prerelease, instead of letting prerelease tags replace the default download and image.

Decision: Only pushed `v*` tags publish checked artifacts because a review or validation run must not create a public release, instead of publishing on pull requests or manual dispatches.

Decision: The release workflow refuses to republish an existing release tag because versioned downloads must retain their original bytes and checksums, instead of replacing published assets or image tags on a repeated tag push.

Decision: Publishing reuses the artifacts that passed the release checks because rebuilding could publish different bytes, instead of building again in the publish job.

Decision: Only the separate publish job receives contents and packages write permissions because builds and public download checks need no publishing access, instead of granting those permissions to the whole workflow.
