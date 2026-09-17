{
  systems = [ "x86_64-linux" ];
  imports = [
    ./source.nix
    ./cargo.nix
    ./native.nix
    ./development.nix
    ./fedora.nix
    ./rpm.nix
    ./checks.nix
    ./install-tests
  ];
}
