# Cleaning duty

Levi inspects the barracks before every release. So should you.

## Checks

CI runs these on every push. Run them before you commit:

```sh
cargo fmt --check
cargo clippy --locked --all-targets
cargo test --locked
cargo deny check    # licenses and known vulnerabilities, per deny.toml
```

After upgrading dependencies (`cargo update`, or `cargo upgrade` for major versions), run
`cargo deny check` again.

## Releases

1. List the changes under "Unreleased" in [CHANGELOG.md](CHANGELOG.md), then date them under
   the new version.
2. Bump `version` in `Cargo.toml` following [SemVer](https://semver.org/).
3. Commit "Release x.y.z" and tag it `vx.y.z`.
4. Push the commit and the tag. The Release workflow builds the binaries and publishes the
   GitHub release from the changelog.
5. Run `cargo publish`, then `cargo clean -p levi-git`: the publish check leaves cargo's
   fingerprint pointing at the packaged copy, and later local builds would reuse a stale binary.

## The demo

`sh docs/demo.sh` rebuilds `docs/demo.gif`: it records `docs/demo.tape` with
[vhs](https://github.com/charmbracelet/vhs), renders the title card `docs/demo-card.svg`
(which takes `docs/logo.svg`), and joins them with ffmpeg. It needs vhs, rsvg-convert and
ffmpeg, and a full clone, since the demo browses levi's own history.

## Design

[docs/plan.md](docs/plan.md) records how levi works and why: the decisions, the search
semantics, the diff pane and what's left for later.
