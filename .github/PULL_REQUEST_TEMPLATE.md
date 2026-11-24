## What changed and why

<!-- One concern per pull request. Link the issue if there is one. -->

## How you verified it

- [ ] `scripts/verify_all.sh` ends with `ALL CHECKS PASSED` (and `--e2e` if the change touches the core, the IPC layer or the UI flows)
- [ ] A new test fails without this change (bug fixes) or covers the failure paths (features); test data is made up
- [ ] `ai-docs/` and `CHANGELOG.md` updated where behaviour, a command, an error code or a limit changed

## What you did not verify

<!-- "Not run on a device", "only tried with a synthetic file", "no real S3 provider". Honest gaps are welcome. -->

## Security check

- [ ] No key, seed, password or hash is returned over IPC, logged, or placed in an error message
- [ ] No new network destination, timer, background service or notification
- [ ] No new dependency, or its latest stable version was checked and recorded in `ARCHITECTURE.md` §2
- [ ] A change in `crypto/` is matched by `CRYPTO_SPEC.md` and the stored fixtures
