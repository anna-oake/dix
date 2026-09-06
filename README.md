# Diff Nix

A blazingly fast tool to diff Nix related things.

Currently only supports closures (a derivation graph, such as a system build or
package).

![output of `dix /nix/var/nix/profiles/system-69-link/ /run/current-system`](.github/dix.png)

## Persistent snapshots

This fork adds versioned snapshot files while keeping the upstream version and
`dix OLD NEW` interface unchanged. Pin the fork revision when packaging it.

```sh
# Capture a built NixOS/nix-darwin system or any other built store output.
# The parent directory must exist. --file publishes the JSON atomically.
dix snapshot /run/current-system --file current.json

# Without --file, compact JSON is written to stdout.
dix snapshot /run/current-system > current.json

# These commands need neither Nix nor the original store paths.
dix diff-snapshots old.json new.json
dix diff-snapshots old.json new.json --output json
```

Snapshot export always uses the correctness-preserving backend chain. Keep the
output GC-rooted until export completes. Snapshot commands require the `json`
Cargo feature, enabled by default. Diagnostics go to stderr, including with `-v`.

Schema version 1 contains `schema_version`, `root` (canonical store path),
`closure` (unique objects with `path` and `nar_size`, in bytes), and `selected`
(store paths selected by the system profile). Generic package outputs normally
have an empty `selected` list. Export sorts paths deterministically. Import
rejects unsupported versions, malformed paths, duplicates, negative/overflowing
sizes, and roots or selected paths missing from the closure. Store paths are
validated syntactically, never looked up during import or comparison.

These files preserve the data needed for the existing package/version, selection,
path-count, and NAR-size reports. They do not contain file contents or NixOS
option values. Raw `nix path-info --json` is not this format. Repository, commit,
and host indexing belong to the caller, not the portable snapshot.

For Buildbot, export while the result link still protects the output, and reuse
snapshots for outputs skipped because they were already built. The future
`nix-diffs` service will index snapshots under
`/var/lib/nix-diffs/snapshots/<owner>/<repo>/<commit>/<path-name>.json`, serve
`/diff/<owner>/<repo>/<old-commit>/<new-commit>`, and cache comparison JSON under
`/var/lib/nix-diffs/diffs/<owner>/<repo>/<old-commit>/<new-commit>/<path-name>.json`.
That service and GitHub posting are not implemented by this fork.

## Usage
```bash
$ dix --help
Diff Nix

Usage: dix [OPTIONS] <OLD_PATH> <NEW_PATH>

Arguments:
  <OLD_PATH>


  <NEW_PATH>


Options:
  -v, --verbose...
          Increase logging verbosity

  -q, --quiet...
          Decrease logging verbosity

      --color <WHEN>
          Controls when to use color

          [default: auto]
          [possible values: auto, always, never]

      --force-correctness
          Fall back to a backend chain that skips SQLite immutable mode.

          This is relevant if the output of dix is to be used for more critical applications and not just as human-readable overview.

          The default backend falls back to opening Nix's SQLite database with `?immutable=1` if the normal connection fails. That is faster than Nix commands, but can be inaccurate if the database is being written to at the same time.

      --output <OUTPUT>
          Select the output format to use

          Possible values:
          - human: Output in the default dix format highlighting version changes
          - json:  Display the output as JSON for machine parsing (requires `json` feature)

          [default: human]

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version

$ dix /nix/var/profiles/system-69-link /run/current-system
```

# Usage in CI

If you're planning on using dix in CI, you might want to set the
`--force-correctness` flag to ensure that the results are definitely accurate.\
Dix will fall back to a connection using `?immutable=1` to Nix's SQLite database
if it fails connecting normally; This can however result in inaccurate output if
the database is being written to at the same time.\
Passing `--force-correctness` will make dix fall back to Nix commands if
connection to the database fails, which ensures correct output, potentially at
the cost of speed.

## Releasing

`dix-diff` is a separate crate because it owns the pure package/version diff
engine. Publish it before publishing `dix`; the `dix` package depends on the
same exact `dix-diff` version.

```sh
cargo publish -p dix-diff
cargo publish -p dix
```

## Contributing

If you have any problems, feature requests or want to contribute code or want to
provide input in some other way, feel free to create an issue or a pull request!

## Thanks

Huge thanks to [nvd](https://git.sr.ht/~khumba/nvd) for the original idea! Dix
is heavily inspired by this and basically just a "Rewrite it in Rust" version of
nvd, with a few things like version diffing done better.

Furthermore, many thanks to the amazing people who made this projects possible
by contributing code and offering advice:

- [@Dragyx](https://github.com/Dragyx) - Cool SQL queries. Much of dix's speed
  is thanks to him.
- [@NotAShelf](https://github.com/NotAShelf) - Implementing proper error
  handling.
- [@RGBCube](https://github.com/RGBCube) - Giving the codebase a deep scrub.

## License

Dix is licensed under [GPLv3](LICENSE.md). See the license file for more
details.
