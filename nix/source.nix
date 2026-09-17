{ lib, ... }:
{
  perSystem = { ... }: {
    options.wayShell = lib.mkOption {
      type = lib.types.attrs;
      internal = true;
    };
    config.wayShell = {
      # Only application/build inputs affect compilation. The RPM archive adds
      # its spec separately; documentation and packaging checks stay outside.
      source = lib.fileset.toSource {
        root = ../.;
        fileset = lib.fileset.unions [
          ../Cargo.toml
          ../Cargo.lock
          ../LICENSE
          ../crates
          ../data
          ../gresources.xml
          ../scripts
          ../tests
          ../contrib/systemd
        ];
      };
      version = (builtins.fromTOML (builtins.readFile ../Cargo.toml)).workspace.package.version;
      schemaIds = map (match: builtins.elemAt match 0) (
        builtins.filter builtins.isList (
          builtins.split ''<schema[^>]*id="([^"]+)"'' (
            builtins.readFile ../data/org.ldelossa.way-shell.gschema.xml
          )
        )
      );
    };
  };
}
