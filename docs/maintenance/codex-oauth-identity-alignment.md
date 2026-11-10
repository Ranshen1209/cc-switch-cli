# Codex OAuth account identity

Source: farion1231/cc-switch PR #6780 (c2ec78dd471eff5187f586b8bae73e9aa9b63b65), with the current manager at 28dc5198115575ced2e3502f52181e7d211859c9 for follow-up duplicate-account and login lifecycle handling.

The CLI uses the upstream manager's UUID local IDs, separate workspace IDs, OIDC subject checks, v2 account store, legacy-account quarantine, targeted reauthentication, duplicate checks, account/lifecycle/storage locks, and pending-flow cancellation/generation checks. The upstream identity and persistence regression tests are retained.

CLI adapters expose reauthentication through `auth login --account-id` and the account action menu. Requests, model discovery, quota inspection and quota reset resolve the workspace from the same selected local account used to obtain the token. Proxy tests cover two local members of the same workspace and replacement of incoming credentials with the bound member's credentials.

The CLI currently supports managed Codex OAuth as a Claude proxy provider. It does not implement the desktop's managed native Codex auth.json activation/ownership workflow. Consequently native auth.json token adoption, synchronization, removal and official native-bearer passthrough validation are not imported. Native Codex login behavior is unchanged. The device-flow before-commit callback is a no-op because there is no native-auth publication to coordinate; account publication remains protected by the upstream manager locks.

Existing upstream refresh and cross-process manager behavior is retained; this change does not add a separate identity algorithm or change the SQLite schema. Existing managed accounts must be reauthenticated explicitly; ambiguous legacy rows are not silently assigned to a workspace member.
