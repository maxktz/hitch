# Contributing

Hitch is a Rust CLI distributed through npm as `hitch-cli`.

## Development

Run checks before committing:

```sh
cargo fmt -- --check
cargo test
```

Build the local development binary:

```sh
cargo build --release
```

## Release

Create a release commit and tag:

```sh
npm run release -- 0.1.1
git push origin main --tags
```

GitHub Actions builds native binaries, publishes `hitch-cli` to npm, and creates a GitHub release.
