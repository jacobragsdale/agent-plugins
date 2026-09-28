# TODO

Work to do now, before the port. `WORK_PORT_PLAN.md` is the runbook for bringing this into the internal network later.

When an item here lands, update `WORK_PORT_PLAN.md` in the same change so the port stays as small as possible:

- drop steps the item made unnecessary
- turn code steps into config steps
- add any new config or facts the port has to supply

## Publish skills from CI

Done: [Publish skills from CI](docs/publish-from-ci.md). Checked end to end with GitHub Actions against the home-lab marketplace, on a self-hosted runner because the marketplace has no public ingress. The Azure DevOps template and the Entra configuration are `WORK_PORT_PLAN.md` §4.4.

### Not planned until someone needs it

- **Declared versions** (`metadata.version` in `SKILL.md`), for a team that wants a major bump.
- **Pruning** packages that left the repository.
- **Provenance** on the package page: the repository and commit a version came from.
- **Workers pulling skills:** a bundle `tar.gz` endpoint with `ETag`s, plus a boot and refresh how-to. Machine identities already cover their auth.
- **A headless Linux CLI.** `curl` covers CI.
