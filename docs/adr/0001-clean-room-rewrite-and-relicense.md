# Clean-room rewrite, relicensed MIT OR Apache-2.0

Vellum 1.x was a trimmed-down fork of awww (GPL-3.0), and its TUI grew without a plan. For 2.0 we rewrote every crate from scratch, and the new code contains nothing from awww. That freed us to relicense under MIT OR Apache-2.0, the usual Rust dual license and the easiest one for packagers and contributors. We accepted the cost of re-implementing the Wayland and transition code ourselves in exchange for an architecture designed around a minimal footprint. Nothing was migrated from 1.x: the only user was the author.

## Consequences

Do not copy code from awww or swww into this repository. GPL-3.0 code would force the whole project back under GPL.
