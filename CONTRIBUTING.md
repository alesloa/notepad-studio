# Contributing to Notepad Studio

Thanks for taking the time to contribute! Notepad Studio is a small, friendly
project and contributions of all sizes are welcome — bug reports, fixes,
features, docs, or just ideas.

## Prerequisites

- A stable Rust toolchain. The easiest way to get one is [rustup](https://rustup.rs/).

That's the whole list. The project is self-contained: `gpui` is vendored in-repo
under [`vendor/gpui`](vendor/gpui), so there's no extra setup, no external
checkout, and nothing to fetch beyond the regular crates.io dependencies.

## Build, run, and check

```sh
cargo build            # debug build
cargo run              # build + run (debug)
cargo build --release  # optimised build
cargo check            # type-check only, no binary
```

There's no formal test suite yet. If you add tests, run them with `cargo test`.

## Reporting bugs and requesting features

Open a [GitHub issue](https://github.com/alesloa/notepad-studio/issues/new/choose)
and pick the matching template:

- **Bug report** — tell us what happened, the steps to reproduce, what you
  expected, your OS, and the version.
- **Feature request** — describe the problem you're trying to solve and any
  solution you have in mind.

For **security issues**, please don't open a public issue — see
[SECURITY.md](SECURITY.md) for how to report privately.

## Pull requests

- Keep PRs **small and focused** — one change per PR is easiest to review.
- **Describe the change** and reference any related issue (e.g. `Closes #123`).
- **Keep the build green:** `cargo build` should pass before you open the PR.
- Try to match the style of the surrounding code.
- Commit messages: we lean on [Conventional Commits](https://www.conventionalcommits.org/)
  (`feat:`, `fix:`, `docs:`, etc.) as a gentle preference — it's not a hard
  requirement, so don't sweat it.

## License of contributions

Contributions are accepted under the [Apache License 2.0](LICENSE). By submitting
a contribution, you agree that it is licensed inbound under the same terms as the
project (Apache-2.0 §5, inbound = outbound). There's **no CLA** and **no DCO
sign-off** required — just open a PR.

## Code of Conduct

This project follows a [Code of Conduct](CODE_OF_CONDUCT.md). By participating,
you're expected to uphold it.
