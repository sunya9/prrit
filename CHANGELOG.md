# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.2](https://github.com/sunya9/prrit/compare/v0.1.1...v0.1.2) - 2026-09-13

### Added

- add --version (-V)
- ship an agent skill and print it with `prrit skill`

### Other

- build releases from the calling commit instead of an input ref
- one code block per install method
- trust the tap before brew install

## [0.1.1](https://github.com/sunya9/prrit/compare/v0.1.0...v0.1.1) - 2026-09-12

### Other

- note the Actions setting release-plz needs to open PRs
- actually use the per-directory counter
- keep temporary directories unique across parallel tests

## [0.1.0](https://github.com/sunya9/prrit/releases/tag/v0.1.0) - 2026-09-12

### Added

- Gerrit-style stacked PRs for GitHub via a git remote helper

### Other

- let release-plz manage the package
- add CI, release-plz and binary distribution pipeline
- apply rustfmt
