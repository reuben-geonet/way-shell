{
  inputs,
  config,
  getSystem,
  lib,
  ...
}:
{
  # Keep matrix metadata under a recognised output so strict flake checks pass.
  flake.lib.githubActions =
    let
      githubActions = inputs.nix-github-actions.lib.mkGithubMatrix {
        attrPrefix = "packages";
        checks = lib.genAttrs config.systems (system: (getSystem system).wayShell.installationTests);
      };
    in
    githubActions
    // {
      matrix.include = map (
        entry:
        entry
        // {
          package =
            if entry.name == "test-install-nixos" then
              "way-shell"
            else
              "package-${lib.removePrefix "test-install-" entry.name}";
        }
      ) githubActions.matrix.include;
    };

  perSystem =
    {
      config,
      pkgs,
      lib,
      ...
    }:
    let
      fedoraTests = import ./fedora { inherit config pkgs lib; };
      tests = fedoraTests // {
        test-install-arch = import ./arch.nix { inherit config pkgs; };
        test-install-nixos = import ./nixos.nix { inherit config pkgs; };
      };
    in
    {
      wayShell.installationTests = tests;
      packages = tests // {
        test-install = pkgs.linkFarm "way-shell-installation-tests" tests;
        test-install-fedora = pkgs.linkFarm "way-shell-fedora-installation-tests" fedoraTests;
      };
    };
}
