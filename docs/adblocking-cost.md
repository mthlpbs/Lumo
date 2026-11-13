# Brave engine cost measurements

Measured locally on Windows x64 on 13 November 2026. These compare the previous
handwritten matcher with pinned `adblock-rust` 0.13.5, including regex support,
public-suffix domain resolution and CSS validation. The parser/matcher is built
with `opt-level = "s"`; rendering code keeps its existing optimization.

Both application builds used `CARGO_PROFILE_RELEASE_STRIP=false cargo wbuild
--offline`, then the same GNU strip and NSIS tools. Versions were unchanged;
these are local comparison artifacts, not published releases.

| Artifact | Before (bytes) | After (bytes) | Increase (bytes) |
|---|---:|---:|---:|
| Stripped EXE | 4,922,880 | 7,458,816 | 2,535,936 |
| NSIS installer | 2,032,708 | 2,783,823 | 751,115 |

The optional protection data adds 4,146,407 bytes on first installation, outside
the EXE and installer. The larger third-party notices are included in the
installer comparison. No new DLL or browser runtime is bundled.

## Engine memory and CPU

`crates/lumo-core/examples/adblock_metrics.rs` uses `GetProcessMemoryInfo` and
`QueryProcessCycleTime`. The old matcher was compiled with the same shared
harness in `tools/adblock_metrics_common.rs`. Each reads identical EasyList,
EasyPrivacy and resource snapshots, warms the matcher, then alternates 10,000
blocked/allowed script requests. Both blocked exactly 5,000 requests.

| Measurement | Old matcher | Brave engine |
|---|---:|---:|
| Private bytes before loading | 712,704 | 667,648 |
| Private bytes loaded and warmed | 29,913,088 | 9,027,584 |
| Working set loaded and warmed | 30,920,704 | 13,864,960 |
| Private bytes after dropping engine | 1,372,160 | 2,195,456 |
| Compile process CPU cycles | 385,591,176 | 216,751,122 |
| 10,000 checks process CPU cycles | 27,207,982,677 | 63,425,892 |

The measured additional private memory was 27.85 MiB before and
7.97 MiB after. This is engine-only process memory, not a measurement of
whole-app idle memory or WebView2 renderer memory. Allocators can retain some
pages after the last engine is dropped. CPU figures are one local sample and
cover these representative requests, not a general speed guarantee.

One engine is shared by retained Browser and AI views. The engine is not created
at startup or when blocking is disabled. Closing the last view releases it.
There is no new idle thread, timer, poll, scheduled update, or per-frame work.
Data preparation runs only when explicitly enabled/requested and is cancelled
on lock, display-off or battery saver. Cosmetic work is driven by page mutations
and pauses for hidden documents.

## Dependency code attribution

The requested `x86_64-w64-mingw32-nm --size-sort -S -C` command was run on both
unstripped binaries. This COFF toolchain reports zero for every symbol size, so
those totals cannot establish crate costs. `tools/adblock_binary_cost.py` also
uses x64 PE unwind function ranges, matched to demangled symbol names. Selected
after-build function byte totals follow; they exclude data and cannot assign
inlined code to its original crate. They must not be read as download sizes.

| Crate | Attributed function bytes |
|---|---:|
| `adblock` | 157,686 |
| `psl` | 310,343 |
| `regex_automata` | 273,690 |
| `regex_syntax` | 163,435 |
| `aho_corasick` | 142,170 |
| `selectors` | 41,432 |
| `cssparser` | 31,405 |
| `base64` | 3,700 |

The new dependencies are needed for mature list parsing, network exceptions,
regexes, real domain boundaries, cosmetic syntax and replacement resource
handling. Base64 decodes engine-selected redirect resources. Their measured
application-level cost is shown above; no list bodies are embedded in code.

Validation: 365 core tests, 260 app tests, and six desktop checks passed,
including real loopback HTTP request interception, redirect/scriptlet execution,
dynamic cosmetics, child-frame cosmetics, live switching, unavailable-data
blocking, settings, AI handoffs and gesture behavior. The app suite uses its
normal separate test threads; forcing all tests onto one thread causes an
existing COM-apartment conflict in an unrelated catalog HTTP test.

First-use downloads require the prepared add-on pre-release to be published by
the maintainer. The local test build includes verified external data beside its
EXE. See [the data staging instructions](../addons/web-blocking/README.md).
