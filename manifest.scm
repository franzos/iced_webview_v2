(use-modules (guix packages)
             (guix search-paths)
             (gnu packages rust)
             (gnu packages commencement)
             (gnu packages tls)
             (gnu packages base)
             (gnu packages llvm)
             (gnu packages pkg-config)
             (gnu packages freedesktop)
             (gnu packages xdisorg)
             (gnu packages vulkan))

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
 (list rust-1.88
       (list rust-1.88 "cargo")
       rust-analyzer
       gcc-toolchain
       clang-toolchain
       pkg-config
       wayland
       wayland-protocols
       libxkbcommon
       vulkan-loader
       openssl-with-dir))
