{
  self,
  inputs,
  config,
  getSystem,
  lib,
  ...
}:
{
  # Keep matrix metadata under a recognised output so strict flake checks pass.
  flake.lib.githubActions = inputs.nix-github-actions.lib.mkGithubMatrix {
    attrPrefix = "packages";
    checks = lib.genAttrs config.systems (system: (getSystem system).wayShell.installationTests);
  };

  perSystem =
    {
      config,
      pkgs,
      lib,
      system,
      ...
    }:
    let
      fedoraTests = import ./fedora.nix { inherit config pkgs lib; };
      tests = fedoraTests // {
        test-install-nixos = import ./nixos.nix { inherit config pkgs; };
      };
      releases = builtins.attrNames config.wayShell.fedora.releases;
      command = pkgs.writeShellApplication {
        name = "test-install";
        runtimeInputs = [ pkgs.nix ];
        text = ''
          fail() { echo "test-install: $*" >&2; exit 2; }
          target=test-install
          case "$#" in
            0) ;;
            1)
              case "$1" in
                --fedora) target=test-install-fedora ;;
                --nixos) target=test-install-nixos ;;
                --list)
                  printf 'fedora %s\n' ${lib.escapeShellArgs releases}
                  printf 'nixos (pinned nixpkgs)\n'
                  exit 0 ;;
                *) fail "unknown argument: $1" ;;
              esac ;;
            2)
              [ "$1" = --fedora ] || fail "expected --fedora VERSION"
              case "$2" in
                ${lib.concatStringsSep "|" releases}) target="test-install-fedora-$2" ;;
                *) fail "unsupported Fedora release: $2 (use --list)" ;;
              esac ;;
            *) fail "too many arguments" ;;
          esac
          # Match the RPM command: build from this snapshot, not the caller's cwd.
          exec nix build --no-update-lock-file \
            --option allow-import-from-derivation false --print-build-logs \
            "path:${self}#packages.${system}.$target"
        '';
      };
    in
    {
      wayShell.installationTests = tests;
      packages = tests // {
        test-install = pkgs.linkFarm "way-shell-installation-tests" tests;
        test-install-fedora = pkgs.linkFarm "way-shell-fedora-installation-tests" fedoraTests;
      };
      apps.test-install = {
        type = "app";
        program = lib.getExe command;
        meta.description = "Test package installation in disposable OS virtual machines";
      };
    };
}
