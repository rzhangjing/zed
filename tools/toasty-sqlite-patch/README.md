# `toasty-driver-sqlite` 0.11.0 — local patch

Verbatim copy of the published `toasty-driver-sqlite 0.11.0` crate
(<https://crates.io/crates/toasty-driver-sqlite>, MIT), reached from
`crates/admin` (`Help > Admin Demo`) through `toasty 0.11`.

## Why it is here

RustGIS 0.6.0 (`crates/rustgis`) pins `rusqlite 0.41`, which requires
`libsqlite3-sys ^0.39`. `libsqlite3-sys` declares `links = "sqlite3"`, and cargo
allows exactly one package per `links` value in a graph, so every other
`rusqlite` in the tree has to reach the same `libsqlite3-sys`.

The published `toasty-driver-sqlite 0.11.0` requires `rusqlite ^0.40`
(`libsqlite3-sys ^0.38`). That is the last `rusqlite 0.40` consumer in this
workspace after `crates/octos/octos-fleet` and
`crates/fundamental/ew_operate_record` moved to the workspace's `rusqlite 0.41`;
`toasty-driver-sqlite` itself has no release on `rusqlite 0.41` (0.11.0 is the
newest).

## What changed

`Cargo.toml` only: `[dependencies.rusqlite] version = "0.40"` → `"0.41"`.
`src/lib.rs` and `src/value.rs` are byte-identical to the published crate;
`upstream.sha256` in this directory records their SHA256 (and the SHA256 of the
crates.io `.crate` archive) so an upgrade can be checked file by file:

```powershell
Get-FileHash src\lib.rs, src\value.rs -Algorithm SHA256
```

The driver's `rusqlite` surface is `Connection::open`/`open_in_memory`,
`prepare_cached`, `prepare`, `execute`, `query`, `params!`,
`params_from_iter`, `Row`, `types::{ToSql, ToSqlOutput, Value, ValueRef}` and
`rusqlite::Result` — all unchanged between 0.40 and 0.41, so no source change
was needed. `cargo check -p admin --all-targets` is what proves it.

## License

`LICENSE-MIT` is the MIT text the crate declares (`license = "MIT"`), kept here
because the published `.crate` archive carries no license file.


## Upstream

If `toasty-driver-sqlite` ever releases against `rusqlite 0.41`, delete this
directory and the `[patch.crates-io]` entry in the root `Cargo.toml` that points
at it.
