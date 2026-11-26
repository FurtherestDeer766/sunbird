#!/bin/sh
# Installs and launches Sunbird for Desktop Phoenix on macOS (arm64).
# Keeps the original Discord Phoenix.app untouched by working on an ad-hoc
# signed copy under ~/Library/Application Support/sunbird so dyld honors
# DYLD_INSERT_LIBRARIES.
#
#   sh scripts/sunbird-macos.sh [/path/to/Discord Phoenix.app]
#   sh scripts/sunbird-macos.sh --no-runtime
#   sh scripts/sunbird-macos.sh --uninstall
#
# SUNBIRD_DYLIB points at a locally built libphoenix.dylib.
# SUNBIRD_SITE points at a custom download server (default: https://assets.sunbird.tools).
set -eu

site=${SUNBIRD_SITE:-https://assets.sunbird.tools}
home="$HOME/Library/Application Support/sunbird"
copy="$home/Discord Phoenix.app"

if [ "${1:-}" = --uninstall ]; then
   rm -rf "$home"
   echo "Sunbird is uninstalled. Your original Discord Phoenix.app was not changed."
   exit 0
fi

no_runtime=
while [ "${1:-}" = --no-runtime ] || [ "${1:-}" = --gate-only ]; do
   no_runtime=1
   shift
done

discord=${1:-}
if [ -z "$discord" ]; then
   for candidate in \
      "/Applications/Discord Phoenix.app" \
      "$HOME/Applications/Discord Phoenix.app"; do
      if [ -d "$candidate" ]; then
         discord=$candidate
         break
      fi
   done
fi

if [ -z "$discord" ] || [ ! -x "$discord/Contents/MacOS/Discord" ]; then
   echo "Couldn't find Discord Phoenix.app. Pass its path explicitly:" >&2
   echo "  sh $0 '/Applications/Discord Phoenix.app'" >&2
   exit 1
fi

mkdir -p "$home"

# Install libphoenix.dylib from a local build if provided, otherwise download.
dylib="$home/libphoenix.dylib"
if [ -n "${SUNBIRD_DYLIB:-}" ] && [ -f "$SUNBIRD_DYLIB" ]; then
   cp "$SUNBIRD_DYLIB" "$dylib"
   codesign --force --sign - "$dylib" >/dev/null 2>&1 || true
elif [ -f "$(dirname "$0")/../target/release/libphoenix.dylib" ]; then
   cp "$(dirname "$0")/../target/release/libphoenix.dylib" "$dylib"
   codesign --force --sign - "$dylib" >/dev/null 2>&1 || true
elif [ -f "$(dirname "$0")/../target/dev-fast/libphoenix.dylib" ]; then
   cp "$(dirname "$0")/../target/dev-fast/libphoenix.dylib" "$dylib"
   codesign --force --sign - "$dylib" >/dev/null 2>&1 || true
elif [ ! -f "$dylib" ]; then
   echo "Downloading Sunbird for macOS (arm64)..."
   tmp=$(mktemp "$home/libphoenix.dylib.XXXXXX")
   if curl -fsSL --max-time 60 -o "$tmp" "$site/plugins/libphoenix-macos-arm64.dylib"; then
      mv "$tmp" "$dylib"
      codesign --force --sign - "$dylib" >/dev/null 2>&1 || true
   else
      rm -f "$tmp"
      echo "Could not download libphoenix-macos-arm64.dylib. Build locally with:" >&2
      echo "  cargo build --release -p phoenix-loader" >&2
      exit 1
   fi
fi

# Refresh the ad-hoc signed copy when the source binary changed.
src_mtime=$(stat -f '%m' "$discord/Contents/MacOS/Discord")
copy_mtime=$(cat "$home/discord.mtime" 2>/dev/null || true)
if [ ! -x "$copy/Contents/MacOS/Discord" ] || [ "$src_mtime" != "$copy_mtime" ]; then
   echo "Preparing ad-hoc signed copy of Discord Phoenix..."
   rm -rf "$copy"
   cp -R "$discord" "$copy"
   xattr -cr "$copy"
   codesign --force --sign - "$copy/Contents/MacOS/Discord"
   echo "$src_mtime" >"$home/discord.mtime"
fi

# Stop any running Discord Phoenix instance so the new one owns the session.
pkill -f "$copy/Contents/MacOS/Discord" 2>/dev/null || true
pkill -f "$discord/Contents/MacOS/Discord" 2>/dev/null || true
sleep 1

if [ -n "$no_runtime" ]; then
   export SUNBIRD_GATE_ONLY=1
fi
export DYLD_INSERT_LIBRARIES="$dylib${DYLD_INSERT_LIBRARIES:+:$DYLD_INSERT_LIBRARIES}"
echo "Launching Discord Phoenix with Sunbird..."
exec "$copy/Contents/MacOS/Discord"
