;; Guix manifest for building with the CEF engine.
;;
;; CEF's libcef.so links against many system libraries (GTK, NSS, ALSA, etc.)
;; that live in package-specific store paths. This manifest collects them and
;; exposes their lib dirs via LIBRARY_PATH so the linker can resolve them.
;;
;; Usage:
;;   guix shell -m manifest-cef.scm
;;   eval "$(./cef-link-flags.sh)"
;;   CC=gcc cargo run --example webview --no-default-features --features cef

(use-modules (guix packages)
             (guix search-paths)
             (gnu packages rust)
             (px packages rust)
             (gnu packages commencement)
             (gnu packages tls)
             (gnu packages base)
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
