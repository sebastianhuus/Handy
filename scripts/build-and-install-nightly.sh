#!/bin/bash
# Builds the nightly macOS app, then hand-signs and packages it into a .pkg,
# and reveals it in Finder for you to install yourself.
#
# `tauri build` fails at its final "strip extra attributes" cleanup step on
# this machine because a corporate endpoint-security agent tags build output
# files with its own extended attributes that no local process can remove,
# even as file owner or root - deliberate EndpointSecurity-level tamper
# protection, not a bug, and not something fixable with a device-specific
# policy exception. The .app bundle itself is already fully built by the
# time that step runs, so we tolerate the failure and just check the
# bundle's mtime to make sure a real build error didn't also happen. Because
# tauri aborts there, it never reaches its own codesign/dmg/pkg steps - so we
# do those ourselves below, entirely outside tauri's bundler, which never
# touches the attributes that break it.
#
# Not auto-installing to /Applications: this machine's MDM/endpoint-security
# sets the SF_NOUNLINK (sunlnk) flag there, which blocks replacing existing
# entries even for an admin user without sudo. Installing via the .pkg's
# Installer.app prompt (or dragging the .app via Finder) sidesteps this -
# both trigger a native macOS authentication dialog for the privileged
# overwrite instead of failing outright like `rm`/`cp` do.
set -uo pipefail

APP_NAME="Handy Nightly.app"
BUNDLE_DIR="src-tauri/target/release/bundle/macos"
APP_PATH="$BUNDLE_DIR/$APP_NAME"
SIGNING_IDENTITY="Handy Nightly Local"
PKG_IDENTIFIER="com.pais.handy.nightly"

before_mtime=""
if [ -e "$APP_PATH" ]; then
  before_mtime=$(stat -f "%m" "$APP_PATH")
fi

bun run tauri:build:nightly
build_status=$?

if [ ! -e "$APP_PATH" ]; then
  echo "error: $APP_PATH was not produced, build failed for real" >&2
  exit 1
fi

after_mtime=$(stat -f "%m" "$APP_PATH")
if [ "$after_mtime" = "$before_mtime" ]; then
  echo "error: $APP_PATH was not rebuilt (mtime unchanged), build failed before bundling completed" >&2
  exit 1
fi

if [ "$build_status" -ne 0 ]; then
  echo "warning: tauri build exited $build_status (expected: xattr cleanup step fails due to endpoint security agent), but a fresh bundle was produced, continuing" >&2
fi

echo "Built: $APP_PATH"

echo "Signing with '$SIGNING_IDENTITY'..."
codesign --force --deep --sign "$SIGNING_IDENTITY" "$APP_PATH"
codesign --verify --deep --strict "$APP_PATH"

VERSION=$(node -p "require('./package.json').version")
PKG_PATH="$BUNDLE_DIR/Handy Nightly ${VERSION}.pkg"

# `pkgbuild --component` alone marks the package relocatable: PackageKit
# checks Launch Services for any existing bundle with our identifier and
# installs onto that instead of --install-location if one is found. Since
# our own build output has that identifier, install "succeeds" but lands
# back on the .app in the build dir instead of /Applications. Building from
# an explicit --root + --component-plist with BundleIsRelocatable=false
# forces it to install where we say.
PKG_ROOT=$(mktemp -d)
trap 'rm -rf "$PKG_ROOT"' EXIT
cp -R "$APP_PATH" "$PKG_ROOT/$APP_NAME"

COMPONENT_PLIST=$(mktemp -t handy-component).plist
pkgbuild --analyze --root "$PKG_ROOT" "$COMPONENT_PLIST"
/usr/libexec/PlistBuddy -c "Set :0:BundleIsRelocatable false" "$COMPONENT_PLIST"

echo "Packaging $PKG_PATH..."
rm -f "$PKG_PATH"
pkgbuild \
  --root "$PKG_ROOT" \
  --component-plist "$COMPONENT_PLIST" \
  --install-location "/Applications" \
  --identifier "$PKG_IDENTIFIER" \
  --version "$VERSION" \
  "$PKG_PATH"

echo "Built: $PKG_PATH"
open -R "$PKG_PATH"
