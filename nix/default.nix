{
  systems = [ "x86_64-linux" ];
  imports = [
    ./source.nix
    ./native.nix
    ./development.nix
    ./fedora.nix
    ./rpm.nix
    ./checks.nix
  ];
}
