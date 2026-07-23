{
  description = "A photo viewer built with PySide6";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in
    {
      packages = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          python = pkgs.python3;

          # The cross-platform CLI/GUI application.
          cli = python.pkgs.buildPythonApplication {
            pname = "photo-viewer";
            version = "0.1.0";
            pyproject = true;

            src = pkgs.lib.cleanSource ./.;

            build-system = [ python.pkgs.setuptools ];

            dependencies = [
              python.pkgs.pyside6
              python.pkgs.pillow
            ];

            nativeBuildInputs = [ pkgs.qt6.wrapQtAppsHook ];
            buildInputs = [ pkgs.qt6.qtbase ];

            # Combine Python wrapper with Qt wrapper
            dontWrapQtApps = true;
            makeWrapperArgs = [
              "\${qtWrapperArgs[@]}"
            ];

            # Copy icons into the installed package so the relative path
            # resolution in lib/photo.py continues to work
            postInstall = ''
              site_packages=$out/${python.sitePackages}
              cp -r $src/icons $site_packages/icons
            '';
          };

          # macOS bundle metadata. CFBundleDocumentTypes is what lets Finder
          # offer "Open With > PhotoViewer" and set it as the default viewer.
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

          # A proper PhotoViewer.app bundle built entirely with Nix.
          appBundle = pkgs.stdenv.mkDerivation {
            pname = "photo-viewer-app";
            version = "0.1.0";
            dontUnpack = true;

            # makeBinaryWrapper (not the shell variant) so the bundle
            # executable is a real Mach-O binary, which LaunchServices
            # launches reliably.
            nativeBuildInputs = [
              pkgs.makeBinaryWrapper
              pkgs.imagemagick
              pkgs.libicns
            ];

            buildCommand = ''
              app="$out/Applications/PhotoViewer.app"
              mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

              cp ${infoPlist} "$app/Contents/Info.plist"

              # Build a multi-resolution .icns from the 512x512 source PNG
              for size in 16 32 128 256 512; do
                magick ${./icons/camera.png} -resize "''${size}x''${size}" "icon_''${size}.png"
              done
              png2icns "$app/Contents/Resources/photo-viewer.icns" \
                icon_16.png icon_32.png icon_128.png icon_256.png icon_512.png

              # Bundle executable wraps the CLI application
              makeWrapper ${cli}/bin/photo-viewer "$app/Contents/MacOS/PhotoViewer"
            '';
          };

          # Copies the bundle into ~/Applications so Launch Services indexes
          # it (nix profile does not put .app bundles where Finder looks).
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
              echo "Right-click an image -> Open With -> PhotoViewer, or set"
              echo "it as the default for a file type via Get Info."
            '';
          };
        in
        {
          photo-viewer = cli;
          default = if pkgs.stdenv.isDarwin then appBundle else cli;
        } // pkgs.lib.optionalAttrs pkgs.stdenv.isDarwin {
          app = appBundle;
          install-app = installApp;
        }
      );

      apps = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = {
            type = "app";
            program = "${self.packages.${system}.photo-viewer}/bin/photo-viewer";
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
          pkgs = nixpkgs.legacyPackages.${system};
          python = pkgs.python3;
        in
        {
          default = pkgs.mkShell {
            packages = [
              (python.withPackages (ps: [
                ps.pyside6
                ps.pillow
                ps.pytest
                ps.pytest-qt
                ps.pytest-cov
              ]))
            ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
              pkgs.qt6.qtwayland
            ];

            shellHook = ''
              export QT_PLUGIN_PATH="${pkgs.qt6.qtbase}/${pkgs.qt6.qtbase.qtPluginPrefix}"
              echo ""
              echo "photo-viewer dev shell"
              echo "====================="
              echo ""
              echo "Run the app:"
              echo "  python -m lib.main [path/to/image/or/directory]"
              echo ""
              echo "Run tests:"
              echo "  pytest"
              echo "  pytest --cov=lib            # with coverage"
              echo ""
              echo "Build / install with Nix:"
              echo "  nix run                     # launch the app"
              echo "  nix build .#app             # build PhotoViewer.app (macOS)"
              echo "  nix run .#install-app       # install into ~/Applications (macOS)"
              echo ""
            '';
          };
        }
      );
    };
}
