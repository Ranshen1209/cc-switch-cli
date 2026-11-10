# Key-field live configuration writes

Source: `farion1231/cc-switch` at
`df40d53c36b604e044b4d5b9091aeb961aa56990`.

The upstream refactor starts at `81df5a08d16dffe8f3b038f2bdea8815e43c4413`
(shared engine), `63ed5002e56ce8a7b81988bf8dea5f4941fe03a8`
(Claude), `0e6430abdb2255a36956f3cf8fb440fae4ce460e` (Codex), and
`fb5645370221a9171eddf6d24ed01f9f27d8b944` (Gemini).

## Source parity

- `live/project/{claude,codex,gemini}.rs` and `live/residue.rs` use the upstream
  projections and their tests. The sole import adaptation is Claude's `[1m]`
  marker: this CLI has no Claude Desktop module, so the identical literal is
  used at that reference.
- `live/floor.rs`, the JSON/TOML/dotenv patchers, the durable engine and
  `mode/operation.rs` already match this upstream revision. Keep ownership,
  parsing, conflict checks and recovery in these shared modules.
- Codex includes upstream's top-level `base_url` routing fix
  (`84457b510516a7580128b86f92a9f2f1f6680527`). Its native login adapter remains
  documented in `codex-upstream-alignment.md`.

## CLI integration

The CLI prepares writes as part of its existing database/config transactions.
`services/provider/key_field_live.rs` connects those transactions and the CLI
proxy lifecycle to the upstream patches and `AppWrite`. Direct Claude and Gemini
switches defer the database current pointer until publication; pending writes
recover forward rather than restoring an old live-file snapshot. Proxy updates
use the same recovery boundary: once publication is pending, the selected row
and restore projection remain available for the engine to finish.

Ordinary switch/add/update/sync operations do not replay saved common snippets,
backfill live globals into provider rows, or re-project MCP just because the
provider changed. User-owned configuration stays live. Malformed documents
fail closed. Gemini's two files are planned together before either is published.

The CLI's explicit common-snippet command remains available. Its intentional
edits are expressed as upstream JSON/dotenv patch operations, like the existing
Codex TOML adapter. An invalid old snippet cannot establish ownership of live
keys, so replacing it does not erase unrelated live content.

Claude temporary launches (CLI and TUI) project onto the current live settings
with the same upstream patch, without replaying saved global snippets or writing
the real settings file. When no direct provider remains during stale recovery,
Claude/Gemini use upstream empty projections to clear owned fields.

The CLI daemon/start/proxy lifecycle remains the integration boundary; this
change does not introduce GUI Stack mode, Grok, or Claude Desktop support.

## Upstream behavior retained

`GEMINI_BASE_URL` is not in upstream's Gemini floor definition; it remains a
user-owned variable. Only upstream's recognized routing/auth/model fields are
cleared. Do not expand the floor locally to fix this or other upstream behavior.

Claude proxy projections likewise leave `OPENROUTER_API_KEY` and `OPENAI_API_KEY`
untouched: upstream does not classify those as Claude-owned fields. Proxy role
aliases follow upstream (`claude-sonnet-5`, `claude-opus-5`).
