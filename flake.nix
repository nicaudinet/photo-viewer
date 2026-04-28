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
        in
        {
          default = python.pkgs.buildPythonApplication {
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
              echo "  python main.linux.py [path/to/image/or/directory]"
              echo ""
              echo "Run tests:"
              echo "  pytest"
              echo "  pytest --cov=lib            # with coverage"
              echo ""
              echo "Build with Nix:"
              echo "  nix build                   # produces result/bin/photo-viewer"
              echo ""
            '';
          };
        }
      );
    };
}
