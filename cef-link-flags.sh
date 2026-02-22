#!/bin/sh
# Generate RUSTFLAGS and LD_LIBRARY_PATH for building/running CEF on Guix.
#
# libcef.so has transitive shared library deps (NSS, pango, etc.) that
# the linker must resolve via -rpath-link at build time and LD_LIBRARY_PATH
# at runtime. NSS puts its .so files in a lib/nss/ subdirectory.
#
# Usage:
#   eval "$(./cef-link-flags.sh)"
#   CC=gcc cargo run --example webview --no-default-features --features cef

IFS=":"
FLAGS=""
LDPATH=""
for dir in $LIBRARY_PATH; do
    if [ -d "$dir" ]; then
        FLAGS="$FLAGS -L native=$dir -C link-arg=-Wl,-rpath-link,$dir"
        LDPATH="$LDPATH:$dir"
        # NSS puts .so files in a lib/nss/ subdirectory
        if [ -d "$dir/nss" ]; then
            FLAGS="$FLAGS -L native=$dir/nss -C link-arg=-Wl,-rpath-link,$dir/nss"
            LDPATH="$LDPATH:$dir/nss"
        fi
    fi
done
# Strip leading colon
LDPATH="${LDPATH#:}"

echo "export RUSTFLAGS=\"$FLAGS\""
echo "export LD_LIBRARY_PATH=\"$LDPATH\""
