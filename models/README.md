# Shared Model Catalog

`catalog.json` is the standard model catalog shipped with yi-token. It is
embedded at compile time and applied to the database on startup, so it can be
versioned and shared through Git. `catalog.schema.json` provides editor
validation; the application additionally validates protocol-specific effort
levels, unique `(name, company)` pairs, token relationships and Provider
references.

Entries are keyed by `(name, company)`, where `company` is `openai` (shared by the
Chat and Responses protocols) or `anthropic`. Applying a catalog updates `brand` and
`capabilities` for matching entries while keeping the database id, so Provider
references stay valid. Standard models created only in Model Management are not
removed and are not written back to `catalog.json`. File `id` values are
informational and may be omitted; the database generates and preserves its own
ids. Standard definitions contain no Provider credentials, endpoints, routes or
local reference counts. Token capacities are stored as integer counts; K/M are
editor shorthand only.

Packaged builds use the embedded catalog. Set `YI_TOKEN_MODEL_CATALOG` to an
absolute file path to use an external shared catalog instead; development and
packaged builds behave the same in this mode. The current source and path are
shown in Model Management.

External file changes are loaded on the next model, Provider or terminal read. Invalid JSON,
unsupported catalog versions, duplicate definitions and changes that invalidate
Provider references are rejected, retaining the last valid database cache.
Correct the file and refresh Model Management. Reapply terminal configuration to
refresh static native model catalogs after changing model capabilities.
