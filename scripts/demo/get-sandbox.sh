#!/usr/bin/env bash
# Downloads the official near-sandbox 2.14.0 build (NEAR's artifact bucket, the same one
# near-workspaces uses), checks its sha256, and installs it as .tools/near-sandbox.
set -euo pipefail
VERSION=2.14.0
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) PLAT=Linux-x86_64; SHA=a0ba004218b218bd74351795e7ce2eda4a2b6af5cd3e252bd5e8bfe3c8f2e691 ;;
  Darwin-arm64) PLAT=Darwin-arm64; SHA=ea53448ef97540c012e5c3d967c74d78292b0465a98cf7027811a936718fe232 ;;
  *) echo "near-sandbox $VERSION is published for Linux x86_64 and macOS arm64 only; use the Dockerfile" >&2; exit 1 ;;
esac
mkdir -p "$ROOT/.tools"
if [ -x "$ROOT/.tools/near-sandbox" ]; then echo "$ROOT/.tools/near-sandbox"; exit 0; fi
TMP="$(mktemp -d)"
curl -sSL --retry 6 --retry-all-errors --retry-delay 5 --connect-timeout 30 --max-time 600   "https://s3-us-west-1.amazonaws.com/build.nearprotocol.com/nearcore/$PLAT/$VERSION/near-sandbox.tar.gz" -o "$TMP/ns.tar.gz"
echo "$SHA  $TMP/ns.tar.gz" | (sha256sum -c - 2>/dev/null || shasum -a 256 -c -) >&2
tar xzf "$TMP/ns.tar.gz" -C "$TMP"
install -m 0755 "$TMP/$PLAT/near-sandbox" "$ROOT/.tools/near-sandbox"
rm -rf "$TMP"
echo "$ROOT/.tools/near-sandbox"
