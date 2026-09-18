{
  systems = [ "x86_64-linux" ];
  imports = [
    ./source.nix
    ./cargo.nix
    ./native.nix
    ./development.nix
    ./packaging
    ./checks.nix
    ./install-tests
  ];
}
