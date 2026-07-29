#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 7 ]]; then
    echo "usage: package-macos.sh UNIVERSAL_BINARY OUTPUT_DIR VERSION COMMIT LICENSE_NOTICES SBOM SIGN_IDENTITY" >&2
    exit 2
fi

binary=$1
output=$2
version=$3
commit=$4
notices=$5
sbom=$6
identity=$7
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]] || { echo "invalid version" >&2; exit 2; }
[[ $commit =~ ^[0-9a-f]{40}$ ]] || { echo "invalid commit" >&2; exit 2; }
for path in "$binary" "$notices" "$sbom" LICENSE packaging/macos/generate-icon.swift; do
    [[ -f $path ]] || { echo "missing release input" >&2; exit 2; }
done
lipo "$binary" -verify_arch x86_64 arm64

if [[ $identity == "-" ]]; then
    signature_mode=ad-hoc-verification
    notarization=not-applicable
else
    signature_mode=production
    notarization=required-and-verified
    [[ -n ${APPLE_NOTARY_KEY_PATH:-} && -n ${APPLE_NOTARY_KEY_ID:-} && -n ${APPLE_NOTARY_ISSUER:-} ]] || {
        echo "production macOS packaging requires protected notarization bindings" >&2
        exit 2
    }
fi

name="codex-notifier-v${version}-macos-universal"
mkdir -p "$output"
stage=$(mktemp -d "$output/.${name}.XXXXXX")
trap 'rm -rf -- "$stage"' EXIT
app="$stage/Codex Notifier.app"
contents="$app/Contents"
mkdir -p "$contents/MacOS" "$contents/Resources"
install -m 0755 "$binary" "$contents/MacOS/codex-notifier"
install -m 0644 LICENSE "$contents/Resources/LICENSE"
install -m 0644 "$notices" "$contents/Resources/THIRD-PARTY-LICENSES.html"
install -m 0644 "$sbom" "$contents/Resources/codex-notifier.spdx.json"
cat > "$contents/Resources/RELEASE-METADATA.json" <<EOF
{"schema_version":1,"product":"codex-notifier","version":"$version","target":"universal-apple-darwin","commit":"$commit","signature":"$signature_mode","notarization":"$notarization"}
EOF
cat > "$contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleDisplayName</key><string>Codex Notifier</string>
<key>CFBundleExecutable</key><string>codex-notifier</string>
<key>CFBundleIconFile</key><string>codex-notifier</string>
<key>CFBundleIdentifier</key><string>io.github.leopardrich.codex-notifier</string>
<key>CFBundleName</key><string>Codex Notifier</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>CFBundleVersion</key><string>$version</string>
<key>LSMinimumSystemVersion</key><string>14.0</string>
<key>LSUIElement</key><true/>
</dict></plist>
EOF

iconset="$stage/codex-notifier.iconset"
swift packaging/macos/generate-icon.swift "$iconset"
iconutil -c icns "$iconset" -o "$contents/Resources/codex-notifier.icns"
rm -rf -- "$iconset"

if [[ $identity == "-" ]]; then
    codesign --force --deep --options runtime --timestamp=none --sign - "$app"
else
    codesign --force --deep --options runtime --timestamp --sign "$identity" "$app"
fi
codesign --verify --deep --strict --verbose=2 "$app"

archive="$output/$name.zip"
rm -f -- "$archive"
ditto -c -k --sequesterRsrc --keepParent "$app" "$archive"
if [[ $identity != "-" ]]; then
    xcrun notarytool submit "$archive" \
        --key "$APPLE_NOTARY_KEY_PATH" \
        --key-id "$APPLE_NOTARY_KEY_ID" \
        --issuer "$APPLE_NOTARY_ISSUER" \
        --wait
    xcrun stapler staple "$app"
    xcrun stapler validate "$app"
    spctl --assess --type execute --verbose=2 "$app"
    rm -f -- "$archive"
    ditto -c -k --sequesterRsrc --keepParent "$app" "$archive"
fi
shasum -a 256 "$archive" | sed "s|  .*/|  |" > "$archive.sha256"
printf '%s\n' "$archive"
