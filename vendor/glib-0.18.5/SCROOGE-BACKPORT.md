# Scrooge backport

This directory is the crates.io archive for `glib` 0.18.5. The archive SHA-256 is
`233daaf6e83ae6a12a52055f568f9d7cf4671dabb78ff9560ab6da230ce00ee5`.
The original license and copyright files are included.

`src/variant_iter.rs` applies the two-line upstream fix from
[gtk-rs/gtk-rs-core #1343](https://github.com/gtk-rs/gtk-rs-core/pull/1343)
for [RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html).
The C variadic out-argument now receives a mutable reference. Tauri's GTK 3
dependencies still require the 0.18 API. Remove the override when the Linux
dependency graph resolves a fixed upstream release.
