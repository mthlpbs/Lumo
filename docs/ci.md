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

The CI runner isolates native test state and retains failure artifacts; it
does not change which non-ignored tests must pass. Phone Drop shutdown also
updates its stop predicate while holding the request-wait mutex, preventing
a notification from being lost immediately before a request sleeps. The
existing shutdown test passed 100 consecutive local runs after that fix.

The release profile builds the blocking engine's parser, domain and regex
dependencies for size, like the engine itself. Application and rendering
code retain optimization level 3. A same-source GNU release comparison on
14 November 2026, with the added package overrides removed for the baseline,
measured 7,499,264 bytes before and 7,240,192 after stripping: 259,072 bytes
saved. Both builds used `CARGO_PROFILE_RELEASE_STRIP=false cargo wbuild`,
followed by the same GNU strip tool. `nm --size-sort -S -C` again reported
zero COFF symbol sizes; the PE unwind fallback in `adblock_binary_cost.py`
attributed reductions of 66,481 function bytes to `regex_automata`, 51,606
to `regex_syntax`, and 30,803 to `aho_corasick`. These function totals exclude
data and code inlined into callers. No dependency, idle worker, timer, poll,
network operation or per-frame work is added by the CI changes.

The static-CRT MSVC release measured in Actions run `43542407606` is
8.78 MB, down from 9.12 MB before those dependency overrides. The EXE
budget is 9 MB to cover the current browser/blocking implementation with
0.22 MB of headroom; the previous 8 MB cap already failed on the existing
implementation. Rendering optimization, features and optional-data packaging
are preserved. Memory, CPU, wake-up and thread limits are unchanged. The
same run measured 12.8 MiB private memory at rest, 45.7 MiB working set,
0.01 million CPU cycles/second and 0.1 wake-ups/second. These are hosted
runner samples, not guarantees for every computer.

Its NSIS installer is 2,858,484 bytes (2.86 MB). Packaging also checks the
existing 3 MB download budget on both manual and release runs. Built packages
remain available as artifacts if a later size, smoke or installer check fails;
publishing still requires the whole package job to succeed.
