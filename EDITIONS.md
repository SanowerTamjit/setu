# Editions: what is free and what is paid

Setu follows an open-core model with one rule:

> **Never paywall what an individual needs. Charge only for what an organization needs: control, audit, scale and support.**

And one promise:

> **Features never move from Community to a paid edition.** Once something ships here under Apache-2.0, it stays here.

| Edition | Who it is for | Where the code lives | License |
| --- | --- | --- | --- |
| **Community** | Individuals, students, open-source teams | This repository | Apache-2.0 |
| Team | Small teams who want hosted sync and shared vaults | Hosted service | Subscription |
| Enterprise | Companies and regulated industries | Separate private repository | Commercial |

## Community (this repository)

Terminal and local shells, SSH/SFTP and other connections, file transfer, tunnels and network tools, host inventory, server management, agentless monitoring, the encrypted local vault, the editor and workspaces, the Git panel, the plugin API, the `setu` CLI, self-hosted sync, role packs, and the opt-in AI assistant with bring-your-own model.

## Enterprise (not in this repository)

SSO (SAML/OIDC) and SCIM, RBAC, session recording, tamper-evident audit logs, command policy, just-in-time access, bastion/gateway mode, fleet dashboards, governed fleet AI agents, central policy rollout, air-gapped licensing and the admin console.

The Community build must always run fully offline and must never depend on Enterprise code. Enterprise code talks to the core only through documented, public interfaces in this repository.
