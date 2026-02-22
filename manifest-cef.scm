;; Guix manifest for building and running with the CEF engine.
;;
;; Uses --container --emulate-fhs to create an FHS-compatible environment
;; so CEF subprocesses can find .pak resources, icudtl.dat, and shared
;; libraries at standard paths (/usr/lib, /usr/share, etc.).
;;
;; Build and run:
;;
;;   guix shell --container --emulate-fhs --network \
;;     --share=$HOME/.cargo --share=$HOME/.cache \
;;     --expose=$XDG_RUNTIME_DIR --expose=/var/run/dbus \
;;     -m manifest-cef.scm -- sh -c \
;;     "XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR WAYLAND_DISPLAY=$WAYLAND_DISPLAY \
;;      DBUS_SESSION_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS \
;;      CARGO_TARGET_DIR=target-cef LD_LIBRARY_PATH=/lib:/lib/nss CC=gcc \
;;      cargo run --example webview --no-default-features --features cef"
;;
;; CARGO_TARGET_DIR  — separate target dir; host-built binaries won't run
;;                     inside the FHS container (different dynamic linker).
;; LD_LIBRARY_PATH   — gcc-toolchain's linker doesn't search /lib by default;
;;                     NSS puts .so files in /lib/nss/.

(use-modules (guix packages)
             (guix search-paths)
             (gnu packages rust)
             (px packages rust)
             (gnu packages commencement)
             (gnu packages tls)
             (gnu packages base)
             (gnu packages bash)
             (gnu packages llvm)
             (gnu packages pkg-config)
             (gnu packages freedesktop)
             (gnu packages xdisorg)
             (gnu packages vulkan)
             (gnu packages fontutils)
             (gnu packages gl)
             (gnu packages gdb)
             (gnu packages glib)
             (gnu packages nss)
             (gnu packages gnome)
             (gnu packages cups)
             (gnu packages xorg)
             (gnu packages linux)
             (gnu packages gtk)
             (gnu packages xml))

(define openssl-with-dir
  (package
    (inherit openssl)
    (native-search-paths
     (cons (search-path-specification
            (variable "OPENSSL_DIR")
            (files '("."))
            (file-type 'directory)
            (separator #f))
           (package-native-search-paths openssl)))))

(packages->manifest
 (list rust-1.92
       (list rust-1.92 "cargo")
       rust-analyzer
       gcc-toolchain
       clang-toolchain-21
       pkg-config
       gnu-make
       gdb
       openssl-with-dir

       ;; container essentials
       bash
       coreutils

       ;; shared deps (also in manifest.scm)
       wayland
       wayland-protocols
       libxkbcommon
       vulkan-loader
       fontconfig
       mesa

       ;; CEF runtime deps — libcef.so links against all of these
       glib
       nss
       nspr
       at-spi2-core
       dbus
       cups
       libx11
       libxcomposite
       libxdamage
       libxext
       libxfixes
       libxrandr
       expat
       libxcb
       cairo
       pango
       eudev
       alsa-lib))
