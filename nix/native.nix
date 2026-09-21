{
  perSystem =
    {
      config,
      pkgs,
      lib,
      ...
    }:
    let
      cfg = config.wayShell;
      common =
        builtins.removeAttrs cfg.rustBuildArgs [
          "cargoDeps"
          "auditable"
        ]
        // {
          pname = "way-shell";
          cargoVendorDir = cfg.craneVendor;
          # Nixpkgs otherwise derives -frandom-seed from each output path.
          # Bindgen tracks that flag, so both stages need the same seed.
          NIX_OUTPATH_USED_AS_RANDOM_SEED = cfg.cargoVendor;
          nativeBuildInputs = cfg.rustBuildArgs.nativeBuildInputs ++ [ pkgs.wrapGAppsHook4 ];
          cargoExtraArgs = "--workspace --bins --frozen --jobs 1";
          buildPhaseCargoCommand = "cargo build --workspace --bins --release --frozen --jobs 1 --verbose";
          doCheck = false;
        };
    in
    {
      packages = rec {
        cargo-deps-native = cfg.craneLib.buildDepsOnly (
          builtins.removeAttrs common [ "src" ] // { dummySrc = cfg.dummySource; }
        );
        default = way-shell;
        way-shell = cfg.craneLib.buildPackage (
          common
          // {
            cargoArtifacts = cargo-deps-native;
            doNotPostBuildInstallCargoBinaries = true;
            # License directories copied from the store are read-only. Only
            # binaries need Crane's reference stripping below.
            doNotRemoveReferencesToVendorDir = true;
            doNotRemoveReferencesToRustToolchain = true;
            installPhase = ''
              runHook preInstall
              PREFIX="$out" CARGO_ARTIFACT_DIR=target/release \
                DEPENDENCY_LICENSES=${config.wayShell.dependencyLicenses} \
                sh scripts/install.sh
              runHook postInstall
            '';
            postInstall = ''
              # The Crane vendor directory contains configuration; Rust source
              # paths point at importCargoLock's original vendor directory.
              removeReferencesToVendoredSources "$out/bin" "${cfg.cargoVendor}"
              removeReferencesToRustToolchain "$out/bin"
              glib-compile-schemas "$out/share/glib-2.0/schemas"
              substituteInPlace "$out/lib/systemd/user/way-shell.service" \
                --replace-fail /usr/bin/way-shell "$out/bin/way-shell"
            '';
            meta = {
              description = "GNOME-like desktop shell for Sway and Niri";
              homepage = "https://github.com/ldelossa/way-shell";
              license = lib.licenses.gpl2Only;
              platforms = [ "x86_64-linux" ];
              mainProgram = "way-shell";
            };
          }
        );
      };
    };
}
