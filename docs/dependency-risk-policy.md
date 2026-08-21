# Dependency risk policy

Dreamseq fails CI on every RustSec vulnerability and on any informational
warning that has not been explicitly reviewed. The executable policy is
`scripts/security-audit.sh`; this document records why its exceptions exist
and when they must be removed.

## Reviewed Tauri Linux exceptions

The 17 current warnings enter through Tauri's Linux WebKit stack. They cover
the archived GTK3 bindings (`RUSTSEC-2024-0411` through `0420`, excluding
`0421`–`0428`), `proc-macro-error` (`RUSTSEC-2024-0370`), the `rust-unic`
family used by `urlpattern` (`RUSTSEC-2025-0075`, `0080`, `0081`, `0098`,
`0100`), and `glib`'s `VariantStrIter` advisory (`RUSTSEC-2024-0429`).

The application does not call `glib::VariantStrIter`; `glib` is present only
as a transitive Linux GUI dependency. This limits exposure but does not make
the advisory disappear, so it remains visible and gated.

## Review and removal criteria

- Review this list whenever Tauri, Wry, WebKitGTK, or `Cargo.lock` changes.
- Remove an exception as soon as the resolved graph no longer reports it.
- A new advisory must fail CI until its reachability and impact are reviewed.
- A vulnerability with a patched compatible version is never allowlisted;
  update the dependency graph instead.
- Reassess the GTK3 exceptions at least quarterly and when Tauri adopts a
  maintained Linux binding stack.

Run `scripts/security-audit.sh`, `cargo deny check`, and `npm audit` before a
release. The script intentionally fails when the actual warning set differs
from the reviewed set in either direction.
