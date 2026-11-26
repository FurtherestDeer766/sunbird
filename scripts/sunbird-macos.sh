#!/bin/sh
# Installs and launches Sunbird for Desktop Phoenix on macOS (arm64).
# Keeps the original Discord Phoenix.app untouched by working on an ad-hoc
# signed copy under ~/Library/Application Support/sunbird so dyld honors
# DYLD_INSERT_LIBRARIES.
#
#   sh scripts/sunbird-macos.sh [/path/to/Discord Phoenix.app]
#   sh scripts/sunbird-macos.sh --install [/path/to/Discord Phoenix.app]
#   sh scripts/sunbird-macos.sh --no-runtime
#   sh scripts/sunbird-macos.sh --uninstall
#
# --install wraps Discord Phoenix.app in place with a native arm64 Mach-O
# launcher so launching from Finder, Spotlight, or the Dock always loads
# Sunbird and preserves login keychain access across restarts.
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

install_app=
no_runtime=
while [ $# -gt 0 ]; do
   case "$1" in
      --install) install_app=1; shift ;;
      --no-runtime|--gate-only) no_runtime=1; shift ;;
      *) break ;;
   esac
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

# Install libphoenix.dylib from a local build if provided, otherwise build from
# this checkout or download a prebuilt binary.
repo_root="$(cd "$(dirname "$0")/.." && pwd)"
dylib="$home/libphoenix.dylib"
if [ -n "${SUNBIRD_DYLIB:-}" ] && [ -f "$SUNBIRD_DYLIB" ]; then
   cp "$SUNBIRD_DYLIB" "$dylib"
   codesign --force --sign - "$dylib" >/dev/null 2>&1 || true
elif [ -f "$repo_root/target/release/libphoenix.dylib" ]; then
   cp "$repo_root/target/release/libphoenix.dylib" "$dylib"
   codesign --force --sign - "$dylib" >/dev/null 2>&1 || true
elif [ -f "$repo_root/target/dev-fast/libphoenix.dylib" ]; then
   cp "$repo_root/target/dev-fast/libphoenix.dylib" "$dylib"
   codesign --force --sign - "$dylib" >/dev/null 2>&1 || true
elif [ -f "$repo_root/target/debug/libphoenix.dylib" ]; then
   cp "$repo_root/target/debug/libphoenix.dylib" "$dylib"
   codesign --force --sign - "$dylib" >/dev/null 2>&1 || true
elif [ -f "$repo_root/Cargo.toml" ] && command -v cargo >/dev/null 2>&1; then
   echo "Building libphoenix.dylib..."
   (cd "$repo_root" && cargo build --release -p phoenix-loader)
   cp "$repo_root/target/release/libphoenix.dylib" "$dylib"
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

if [ -n "$install_app" ]; then
   if ! command -v clang >/dev/null 2>&1; then
      echo "--install requires clang (Xcode Command Line Tools) to build the native Mach-O launcher." >&2
      exit 1
   fi
   pkill -f "$discord/Contents/MacOS/Discord" 2>/dev/null || true
   pkill -f "$copy/Contents/MacOS/Discord" 2>/dev/null || true
   sleep 1

   stage=$(mktemp -d "${TMPDIR:-/tmp}/sunbird-install.XXXXXX")
   trap 'rm -rf "$stage"' EXIT
   cp -R "$discord" "$stage/Discord Phoenix.app"
   app="$stage/Discord Phoenix.app"
   xattr -cr "$app"
   rm -rf "$app/Contents/_CodeSignature"

   mkdir -p "$app/Contents/Frameworks"
   cp "$dylib" "$app/Contents/Frameworks/libphoenix.dylib"

   if [ ! -x "$app/Contents/MacOS/Discord.bin" ]; then
      mv "$app/Contents/MacOS/Discord" "$app/Contents/MacOS/Discord.bin"
   fi

   cat >"$stage/launcher.c" <<'EOF'
#include <libgen.h>
#include <limits.h>
#include <mach-o/dyld.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

int main(int argc, char **argv) {
   (void)argc;
   char raw[PATH_MAX];
   uint32_t size = sizeof(raw);
   if (_NSGetExecutablePath(raw, &size) != 0) return 1;
   char resolved[PATH_MAX];
   if (!realpath(raw, resolved)) return 1;
   char *dir = dirname(resolved);
   char dylib[PATH_MAX];
   char target[PATH_MAX];
   snprintf(dylib, sizeof(dylib), "%s/../Frameworks/libphoenix.dylib", dir);
   snprintf(target, sizeof(target), "%s/Discord.bin", dir);
   const char *existing = getenv("DYLD_INSERT_LIBRARIES");
   if (existing && existing[0] != '\0') {
      if (!strstr(existing, dylib)) {
         char combined[PATH_MAX * 2];
         snprintf(combined, sizeof(combined), "%s:%s", dylib, existing);
         setenv("DYLD_INSERT_LIBRARIES", combined, 1);
      }
   } else {
      setenv("DYLD_INSERT_LIBRARIES", dylib, 1);
   }
   argv[0] = target;
   execv(target, argv);
   perror("execv");
   return 1;
}
EOF
   clang -arch arm64 -O2 -o "$app/Contents/MacOS/Discord" "$stage/launcher.c"
   chmod +x "$app/Contents/MacOS/Discord"

   codesign --force --sign - "$app/Contents/Frameworks/libphoenix.dylib"
   codesign --force --sign - --identifier com.discord.DesktopGpui "$app/Contents/MacOS/Discord.bin"
   codesign --force --sign - --identifier com.discord.DesktopGpui "$app"

   rm -rf "$discord" "$copy"
   mv "$app" "$discord"
   echo "Installed Sunbird into $discord."
   open -a "$discord"
   exit 0
fi

# Refresh the ad-hoc signed copy when the source binary changed.
src_mtime=$(stat -f '%m' "$discord/Contents/MacOS/Discord")
copy_mtime=$(cat "$home/discord.mtime" 2>/dev/null || true)
if [ ! -x "$copy/Contents/MacOS/Discord" ] || [ "$src_mtime" != "$copy_mtime" ]; then
   echo "Preparing ad-hoc signed copy of Discord Phoenix..."
   rm -rf "$copy"
   cp -R "$discord" "$copy"
   xattr -cr "$copy"
   rm -rf "$copy/Contents/_CodeSignature"
   codesign --force --sign - --identifier com.discord.DesktopGpui "$copy/Contents/MacOS/Discord"
   codesign --force --sign - --identifier com.discord.DesktopGpui "$copy"
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
