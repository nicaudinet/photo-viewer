{
  description = "A keyboard-driven photo viewer (Rust + iced; migrating off PySide6)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, rust-overlay, crane }:
    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;

      # Per-system pkgs with the rust-overlay applied.
      pkgsFor = system: import nixpkgs {
        inherit system;
        overlays = [ (import rust-overlay) ];
      };
    in
    {
      packages = forAllSystems (system:
        let
          pkgs = pkgsFor system;
          inherit (pkgs) lib stdenv;

          #############################
          # Rust + iced app (primary) #
          #############################

          # Minimal profile (rustc + cargo + std) — no rust-docs/clippy/rustfmt,
          # which the build doesn't need and are slow to fetch/unpack.
          rustToolchain = pkgs.rust-bin.stable.latest.minimal;
          craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

          # Graphics/windowing libs iced (wgpu + winit) needs. Most are
          # dlopen'd at runtime, so they must be on the rpath of the binary,
          # not just present at build time.
          runtimeLibs = with pkgs; [
            vulkan-loader
            libGL
            wayland
            libxkbcommon
            xorg.libX11
            xorg.libXcursor
            xorg.libXrandr
            xorg.libXi
            fontconfig
            freetype
          ];

          # Keep Rust sources plus the icons we embed via include_bytes!;
          # crane's default cleanCargoSource would strip the .png files.
          src = lib.cleanSourceWith {
            src = ./.;
            filter = path: type:
              (lib.hasSuffix ".png" path) || (craneLib.filterCargoSources path type);
            name = "source";
          };

          commonArgs = {
            inherit src;
            strictDeps = true;
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = lib.optionals stdenv.isLinux runtimeLibs;
          };

          # Build the dependency tree once and cache it.
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;

          viewer = craneLib.buildPackage (commonArgs // {
            inherit cargoArtifacts;
            # Linux: bake the runtime graphics libs into the rpath, else the
            # app panics at launch ("no suitable graphics adapter found").
            postInstall = lib.optionalString stdenv.isLinux ''
              patchelf --add-rpath "${lib.makeLibraryPath runtimeLibs}" \
                "$out/bin/photo-viewer"
            '';
          });

          ############################
          # macOS .app bundle plumbing #
          ############################

          infoPlist = pkgs.writeText "Info.plist" ''
            <?xml version="1.0" encoding="UTF-8"?>
            <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
            <plist version="1.0">
            <dict>
              <key>CFBundleName</key>                <string>PhotoViewer</string>
              <key>CFBundleDisplayName</key>         <string>PhotoViewer</string>
              <key>CFBundleIdentifier</key>          <string>com.nicaudinet.photo-viewer</string>
              <key>CFBundleVersion</key>             <string>0.1.0</string>
              <key>CFBundleShortVersionString</key>  <string>0.1.0</string>
              <key>CFBundleExecutable</key>          <string>PhotoViewer</string>
              <key>CFBundleIconFile</key>            <string>photo-viewer</string>
              <key>CFBundlePackageType</key>         <string>APPL</string>
              <key>NSHighResolutionCapable</key>     <true/>
              <key>LSMinimumSystemVersion</key>      <string>11.0</string>
              <key>CFBundleDocumentTypes</key>
              <array>
                <dict>
                  <key>CFBundleTypeName</key>        <string>Image</string>
                  <key>CFBundleTypeRole</key>        <string>Viewer</string>
                  <key>LSHandlerRank</key>           <string>Alternate</string>
                  <key>LSItemContentTypes</key>
                  <array>
                    <string>public.image</string>
                    <string>public.jpeg</string>
                    <string>public.png</string>
                  </array>
                </dict>
              </array>
            </dict>
            </plist>
          '';

          appBundle = pkgs.stdenv.mkDerivation {
            pname = "photo-viewer-app";
            version = "0.1.0";
            dontUnpack = true;

            nativeBuildInputs = [
              pkgs.makeBinaryWrapper
              pkgs.imagemagick
              pkgs.libicns
            ];

            buildCommand = ''
              app="$out/Applications/PhotoViewer.app"
              mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

              cp ${infoPlist} "$app/Contents/Info.plist"

              for size in 16 32 128 256 512; do
                magick ${./icons/camera.png} -resize "''${size}x''${size}" "icon_''${size}.png"
              done
              png2icns "$app/Contents/Resources/photo-viewer.icns" \
                icon_16.png icon_32.png icon_128.png icon_256.png icon_512.png

              makeWrapper ${viewer}/bin/photo-viewer "$app/Contents/MacOS/PhotoViewer"
            '';
          };

          installApp = pkgs.writeShellApplication {
            name = "install-photo-viewer";
            runtimeInputs = [ pkgs.coreutils ];
            text = ''
              dest="$HOME/Applications/PhotoViewer.app"
              echo "Installing PhotoViewer.app -> $dest"
              rm -rf "$dest"
              mkdir -p "$HOME/Applications"
              cp -R "${appBundle}/Applications/PhotoViewer.app" "$dest"
              chmod -R u+w "$dest"
              echo "Installed. Finder/Spotlight will index it shortly."
            '';
          };
        in
        {
          viewer = viewer;
          default = viewer;
        } // pkgs.lib.optionalAttrs pkgs.stdenv.isDarwin {
          app = appBundle;
          install-app = installApp;
        }
      );

      apps = forAllSystems (system:
        let
          pkgs = pkgsFor system;
        in
        {
          # `nix run` launches the Rust viewer.
          default = {
            type = "app";
            program = "${self.packages.${system}.viewer}/bin/photo-viewer";
          };
        } // pkgs.lib.optionalAttrs pkgs.stdenv.isDarwin {
          install-app = {
            type = "app";
            program = "${self.packages.${system}.install-app}/bin/install-photo-viewer";
          };
        }
      );

      devShells = forAllSystems (system:
        let
          pkgs = pkgsFor system;
          inherit (pkgs) lib stdenv;

          # Minimal profile + only the dev extras we actually use (still no docs).
          rustToolchain = pkgs.rust-bin.stable.latest.minimal.override {
            extensions = [ "rust-src" "rust-analyzer" "clippy" ];
          };

          runtimeLibs = with pkgs; [
            vulkan-loader libGL wayland libxkbcommon
            xorg.libX11 xorg.libXcursor xorg.libXrandr xorg.libXi
            fontconfig freetype
          ];
        in
        {
          default = pkgs.mkShell {
            packages = [ rustToolchain pkgs.pkg-config ]
              ++ lib.optionals stdenv.isLinux runtimeLibs;

            # Linux: runtime graphics libs are dlopen'd, so cargo-run binaries
            # need them on LD_LIBRARY_PATH.
            LD_LIBRARY_PATH = lib.optionalString stdenv.isLinux
              (lib.makeLibraryPath runtimeLibs);

            shellHook = ''
              echo ""
              echo "photo-viewer — Rust dev shell"
              echo "============================="
              echo "  cargo run [-- path]   # launch the viewer"
              echo "  cargo build / clippy / test"
              echo "  nix run               # build+run via Nix"
              echo ""
            '';
          };
        }
      );
    };
}
