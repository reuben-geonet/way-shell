{
  perSystem = { config, pkgs, ... }: {
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
