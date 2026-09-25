{ config, pkgs, lib }:
let
  cfg = config.wayShell;
in
assert lib.assertMsg (lib.hasInfix "\nVersion: ${cfg.version}\n" (builtins.readFile ../../way-shell.spec)) "Cargo and RPM versions disagree";
pkgs.runCommand "way-shell-${cfg.version}.tar.gz" { } ''
  mkdir source
  cp -r ${cfg.source}/. source/
  cp ${../../way-shell.spec} source/way-shell.spec
  chmod -R u+w source
  cp -rL ${cfg.cargoVendor} source/vendor
  cp -r ${cfg.dependencyLicenses} source/dependency-licenses
  mkdir -p source/.cargo
  sed 's/directory = "cargo-vendor-dir"/directory = "vendor"/' \
    ${cfg.cargoVendor}/.cargo/config.toml > source/.cargo/config.toml
  ! grep -F /nix/store source/.cargo/config.toml
  tar --sort=name --mtime=@1 --owner=0 --group=0 --numeric-owner \
    --mode='u+rwX,go+rX,go-w' --format=gnu \
    --transform='s,^\.,way-shell-${cfg.version},' \
    -C source -cf - . | gzip -n > "$out"
''
