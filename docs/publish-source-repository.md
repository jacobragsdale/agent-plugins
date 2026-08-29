# Publish a source repository catalog

This tutorial publishes a catalog that lists sources. Agent Plugins can browse the catalog; packages appear only after a listed source is added.

You need this only when you run your own distribution outside the marketplace server. The marketplace publishes its own catalog at `/api/catalog`, with one listed source per publisher namespace, and the app subscribes to it automatically — see [Publish to the marketplace](publish-to-marketplace.md).

## Create the catalog document

Create `agent-plugins-repository.json`:

```json
{
  "version": 1,
  "repository": { "id": "acme", "name": "Acme sources", "description": "Official portable sources." },
  "sources": [
    { "name": "Review workflows", "description": "Review skill and database MCP server.", "url": "https://nexus.example.com/repository/raw/sources/review-latest.zip" },
    { "name": "Data tools", "description": "Published analysis helpers.", "url": "https://nexus.example.com/repository/raw/sources/data-latest.zip" }
  ]
}
```

The document lists HTTPS archive URLs. It is not installable and does not contribute packages by itself.

## Validate locally

Validate the JSON shape against the checked-in schema:

```bash
jq empty agent-plugins-repository.json
npx ajv-cli validate --spec=draft2020 --strict=false \
  -s https://raw.githubusercontent.com/jacobragsdale/agent-plugins/main/schemas/v1/source-repository.schema.json \
  -d agent-plugins-repository.json
```

Then run the catalog validator:

```bash
cargo run --manifest-path /path/to/agent-plugins/src-tauri/Cargo.toml \
  --bin validate-source-repository -- /path/to/agent-plugins-repository.json
```

You can also validate a published HTTPS JSON URL:

```bash
cargo run --manifest-path /path/to/agent-plugins/src-tauri/Cargo.toml \
  --bin validate-source-repository -- \
  https://nexus.example.com/repository/raw/catalogs/acme.json
```

## Publish and browse

Serve the JSON at `{MARKETPLACE_URL}/api/catalog`, where `MARKETPLACE_URL` is the build-time constant in `src-tauri/src/locator.rs`. That is the only catalog the app reads; it takes no pasted catalog URLs. In Agent Plugins:

1. Open **Manage Sources**. The catalog's listed sources appear by name and description.
2. Select **Add** on one listed source.
3. Confirm. Its `agent-plugins.json` source manifest owns the namespace and packages. Nothing is installed until you install a package.

Need a source that is not listed? Ask the catalog owner to add it.

For the field rules, see [the source-repository reference](source-repository-reference.md). To publish one of the listed sources, see [Publish a portable source](publish-source.md).
