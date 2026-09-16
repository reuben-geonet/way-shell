{
  perSystem = { config, pkgs, ... }: {
    # Shared by the native package and the standalone Rust checks.
    wayShell.rustBuildArgs = {
      inherit (config.wayShell) version;
      src = config.wayShell.source;
      strictDeps = true;
      enableParallelBuilding = false;
      cargoDeps = config.wayShell.cargoVendor;
      # Keep the native Cargo layout used by the installer and test harnesses.
      auditable = false;
      CARGO_TARGET_DIR = "target";
      # Match the RPM check profile without changing local or release debug info.
      CARGO_INCREMENTAL = "0";
      CARGO_PROFILE_DEV_DEBUG = "0";
      CARGO_PROFILE_TEST_DEBUG = "0";
      nativeBuildInputs = with pkgs; [
        pkg-config
        glib
        rustPlatform.bindgenHook
      ];
      buildInputs = with pkgs; [
        glib
        gtk4
        gtk4-layer-shell
        libadwaita
        libpulseaudio
        networkmanager
        pipewire
        wayland
        wireplumber
        dconf
      ];
    };
    wayShell.cargoVendor = pkgs.rustPlatform.importCargoLock {
      lockFile = ../Cargo.lock;
      outputHashes = {
        "wireplumber-0.2.0" = "sha256-3SC3ThadKZGbf6s9ThXs6bRhIZh01yj1FomMyu5YR24=";
        "glib-signal-0.4.0" = "sha256-/RPKGJALL35c9Sl/r7RhQkXdmtinzJUeI2uhbFdyMjM=";
      };
    };
    wayShell.dependencyLicenses = pkgs.runCommand "way-shell-dependency-licenses" { } ''
      mkdir -p "$out"
      for crate in ${config.wayShell.cargoVendor}/*; do
        [ -f "$crate/Cargo.toml" ] || continue
        destination="$out/$(basename "$crate")"
        mkdir -p "$destination"
        cp "$crate/Cargo.toml" "$destination/Cargo.toml"
        (cd "$crate" && find . -type f \( -iname 'license*' -o -iname 'copying*' -o -iname 'copyright*' \) \
          -exec cp --parents '{}' "$destination/" \;)
      done
    '';
    packages.cargo-vendor = config.wayShell.cargoVendor;
  };
}
