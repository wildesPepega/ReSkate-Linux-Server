# Release notes

`v<version>.md` is the text of the GitHub release for that version (the version in
`Cargo.toml`). The release workflow (`.github/workflows/release.yml`) publishes a release when a
version without one reaches `main`, and uses this file for its text, or GitHub's generated notes
when there is none. Write it in the same PR that changes the version.
