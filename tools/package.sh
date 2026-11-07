#!/usr/bin/env bash
# Build lumo.exe (cross, mingw), a portable zip and the NSIS installer into dist/.
#
#   tools/package.sh                 # makensis must be on PATH
#   NSIS_HOST=user@host tools/package.sh   # build the installer on a Linux box with nsis
#
# Optional: SELFTEST_HOST=user@host also runs `lumo.exe --selftest` under Wine + Xvfb
# there and copies the screenshots back to dist/selftest/.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | sed -E 's/.*"(.*)".*/\1/')"
export PATH="$HOME/.cargo/bin:$PATH"

cd "$ROOT"
cargo test -p lumo-core --quiet
cargo build -p lumo --release --target x86_64-pc-windows-gnu
EXE="$ROOT/target/x86_64-pc-windows-gnu/release/lumo.exe"

mkdir -p "$ROOT/dist"
cp "$EXE" "$ROOT/dist/lumo.exe"
# Portable build inside its own folder, so lumo.exe never sits loose next to
# other downloads (Windows looks for some DLLs in the exe's folder first).
python3 "$ROOT/tools/package_portable.py" --exe "$EXE" --out "$ROOT/dist/Lumo-$VERSION-portable.zip"

if [[ -n "${NSIS_HOST:-}" ]]; then
  REMOTE_DIR="lumo-pkg"
  ssh -o BatchMode=yes "$NSIS_HOST" "rm -rf ~/$REMOTE_DIR && mkdir -p ~/$REMOTE_DIR/installer/art ~/$REMOTE_DIR/crates/lumo/res ~/$REMOTE_DIR/dist"
  scp -q -o BatchMode=yes "$ROOT/installer/lumo.nsi" "$NSIS_HOST:$REMOTE_DIR/installer/"
  scp -q -o BatchMode=yes "$ROOT/installer/art/nsis-welcome.bmp" "$ROOT/installer/art/nsis-header.bmp" "$NSIS_HOST:$REMOTE_DIR/installer/art/"
  scp -q -o BatchMode=yes "$ROOT/crates/lumo/res/lumo.ico" "$NSIS_HOST:$REMOTE_DIR/crates/lumo/res/"
  scp -q -o BatchMode=yes "$ROOT/THIRD-PARTY-NOTICES.txt" "$NSIS_HOST:$REMOTE_DIR/"
  scp -q -o BatchMode=yes "$EXE" "$NSIS_HOST:$REMOTE_DIR/lumo.exe"
  ssh -o BatchMode=yes "$NSIS_HOST" "cd ~/$REMOTE_DIR && makensis -V2 -DEXE=\$HOME/$REMOTE_DIR/lumo.exe -DOUTDIR=\$HOME/$REMOTE_DIR/dist installer/lumo.nsi"
  scp -q -o BatchMode=yes "$NSIS_HOST:$REMOTE_DIR/dist/Lumo-Setup-$VERSION.exe" "$ROOT/dist/"
elif command -v makensis >/dev/null; then
  makensis -V2 -DEXE="$EXE" -DOUTDIR="$ROOT/dist" "$ROOT/installer/lumo.nsi"
else
  echo "makensis not found and NSIS_HOST not set: skipping the installer" >&2
fi

if [[ -n "${SELFTEST_HOST:-}" ]]; then
  ssh -o BatchMode=yes "$SELFTEST_HOST" "mkdir -p ~/lumo-test"
  scp -q -o BatchMode=yes "$EXE" "$SELFTEST_HOST:lumo-test/lumo.exe"
  ssh -o BatchMode=yes "$SELFTEST_HOST" 'cd ~/lumo-test && rm -rf st-out && export WINEPREFIX=$HOME/lumo-test/wp WINEDEBUG=-all && timeout 180 xvfb-run -a -s "-screen 0 1920x1080x24" wine ~/lumo-test/lumo.exe --selftest st-out; echo "selftest exit=$?"'
  rm -rf "$ROOT/dist/selftest" && mkdir -p "$ROOT/dist/selftest"
  scp -q -r -o BatchMode=yes "$SELFTEST_HOST:lumo-test/st-out/*" "$ROOT/dist/selftest/" || true
fi

ls -la "$ROOT/dist"
