{
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
      ...
    }:
    let
      fedoraTests = import ./fedora { inherit config pkgs lib; };
      tests = fedoraTests // {
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
