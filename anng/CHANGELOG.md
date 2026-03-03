# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

### Changed

### Deprecated

### Removed

### Fixed

### Security

## [0.2.0-v2pre.2]

### Changed

- Adapt to NNG v2.0.0-alpha.7 API
  ([#42](https://github.com/nanomsg/nng-rs/pull/42)).

## [0.2.0-v2pre.1]

### Changed

- Migrate to NNG v2 API with improved error handling and initialization
  ([#37](https://github.com/nanomsg/nng-rs/pull/37)).

## [0.1.3]

### Fixed

- Handle system errors from `nng_pipe_get_addr`
  ([#26](https://github.com/nanomsg/nng-rs/pull/26)).

## [0.1.2]

### Added

- Add CHANGELOG.md
  ([#25](https://github.com/nanomsg/nng-rs/pull/25)).

### Fixed

- Add `NNG_ENOENT` to the list of valid errors for `nng_pipe_get_addr`
  ([#24](https://github.com/nanomsg/nng-rs/pull/24)).

## [0.1.1]

### Added

- Add repository link to cargo manifest
  ([#22](https://github.com/nanomsg/nng-rs/pull/22)).

## [0.1.0]

Initial public release.

[unreleased]: https://github.com/nanomsg/nng-rs/compare/anng-v0.2.0-v2pre.2...HEAD
[0.2.0-v2pre.2]: https://github.com/nanomsg/nng-rs/compare/anng-v0.2.0-v2pre.1...anng-v0.2.0-v2pre.2
[0.2.0-v2pre.1]: https://github.com/nanomsg/nng-rs/compare/anng-v0.1.3...anng-v0.2.0-v2pre.1
[0.1.3]: https://github.com/nanomsg/nng-rs/compare/anng-v0.1.2...anng-v0.1.3
[0.1.2]: https://github.com/nanomsg/nng-rs/compare/anng-v0.1.1...anng-v0.1.2
[0.1.1]: https://github.com/nanomsg/nng-rs/compare/anng-v0.1.0...anng-v0.1.1
[0.1.0]: https://github.com/nanomsg/nng-rs/releases/tag/anng-v0.1.0
