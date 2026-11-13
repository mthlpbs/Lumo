# Windows CI and releases

The `ci` workflow checks pushes to `main` and pull requests. It builds test
executables with the pinned Windows MSVC toolchain, discovers their tests,
and runs each non-ignored test in a fresh process. A native access violation
fails that test and is recorded with its name and exit code; other tests
still run. Tests are never retried automatically. Desktop-only tests retain
their existing ignored status. Rust documentation tests run separately.

The `lumo-tests` artifact contains `results.json`, compiler output, test
listings, and the output of each test. Artifacts are saved even after a test
fails. A missing test executable, a test that runs zero cases, a timeout,
or a nonzero exit code fails the job.

After the tests pass, manual runs and published numeric app releases also
build the release executable, package the installer and portable ZIP, and
check the desktop, idle costs, and installer. Push and pull-request checks
do not package or publish an app. Add-on releases are skipped.

To test the complete release path from an updated branch without publishing:

```powershell
gh workflow run ci.yml --ref <branch> -f release_dry_run=true
```

Manual runs never create or update a tag or release, including when the
legacy `release_dry_run` input is false. Only a published release event can
attach assets or invoke the package-manager workflow. Update-feed deployment
remains disabled.

Each test run records its exact checked-out commit in the job summary.
**Re-running an old release job uses its original commit and workflow.**
Changes on `main` cannot repair that run. Verify fixes with a new manual run
from the updated branch. When the maintainer authorizes another app release,
prepare its new numeric version and release tag from the tested commit;
never move the existing release tag to newer code.

For local isolation with an installed Windows toolchain:

```powershell
python tools/ci_tests.py --target x86_64-pc-windows-msvc
```

`--package lumo` selects only the app; `--target x86_64-pc-windows-gnu`
supports the local MinGW toolchain. The runner itself has no third-party
Python dependencies and adds nothing to the shipped application.
