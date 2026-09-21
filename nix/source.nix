{ lib, ... }:
{
  perSystem = { ... }: {
    options.wayShell = lib.mkOption {
      type = lib.types.attrs;
      internal = true;
    };
    config.wayShell = {
      # Both package formats consume this snapshot: Cargo sources, fixtures,
      # resources, protocol XML and installation scripts, without local builds.
      source = lib.cleanSourceWith {
        src = ../.;
        name = "way-shell-source";
        filter =
          path: type:
          let
            name = baseNameOf path;
            relative = lib.removePrefix (toString ../. + "/") (toString path);
          in
          lib.cleanSourceFilter path type
          && !(builtins.elem name [
            ".cache"
            ".agents"
            ".codex"
            ".direnv"
            ".github"
            ".vscode"
            "build"
            "dist"
            "outputs"
            "staging"
            "target"
            "gschemas.compiled"
          ])
          && !(lib.hasPrefix "result" name)
          && !(builtins.elem relative [
            "nix"
            "flake.nix"
            "flake.lock"
            "way-shell"
            "way-sh/way-sh"
            "gresources.c"
            "gresources.h"
            "compile_commands.json"
            ".gdb_history"
          ])
          && !(lib.hasSuffix ".o" name)
          && !(lib.hasSuffix ".d" name)
          && !(lib.hasSuffix ".a" name)
          && !(lib.hasSuffix ".rlib" name)
          && !(lib.hasSuffix ".rmeta" name)
          && !(lib.hasSuffix ".pending" name)
          && !(lib.hasPrefix "tests/" relative && lib.hasSuffix "-test" name);
      };
      version = let
        cargoVersion = (builtins.fromTOML (builtins.readFile ../Cargo.toml)).workspace.package.version;
        rpmVersion = lib.removePrefix "Version: " (
          lib.findFirst (lib.hasPrefix "Version: ") (throw "way-shell.spec has no Version") (
            lib.splitString "\n" (builtins.readFile ../way-shell.spec)
          )
        );
      in assert lib.assertMsg (cargoVersion == rpmVersion) "Cargo and RPM versions disagree"; cargoVersion;
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
