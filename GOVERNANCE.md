# Governance

## Model

Setu is **maintainer-led**. The project lead has final say on scope and releases, and delegates areas to maintainers as the project grows. Once there are three or more regular external maintainers, a small steering group will be formed and this document updated.

## Areas

| Area | Paths | Owners |
| --- | --- | --- |
| Core | `crates/` | @SanowerTamjit |
| UI | `apps/`, `packages/` | @SanowerTamjit |
| Security | `crates/setu-ssh`, `crates/setu-vault`, `SECURITY.md` | @SanowerTamjit |
| Docs | `docs/`, `*.md` | @SanowerTamjit |

The authoritative list is [`.github/CODEOWNERS`](.github/CODEOWNERS).

## Becoming a maintainer

Contributors who make sustained, high-quality contributions to an area and review others' work may be invited to become maintainers of that area.

## Decisions

- Day-to-day decisions happen in pull requests.
- Technical decisions with lasting impact are recorded as ADRs in [`docs/adr/`](docs/adr/).
- Larger changes go through a public RFC issue with at least a week of discussion.
- The free / paid line is defined in [EDITIONS.md](EDITIONS.md). Changes to it are announced publicly, and features never move from Community to a paid edition.

## Trademark

"Setu" and the Setu logo are trademarks of the project. You may use the name to refer to the project and to unmodified builds. Forks and modified distributions must use a different name and logo, and must not imply endorsement. A full trademark policy will be published before 1.0.
