{
  perSystem =
    {
      config,
      pkgs,
      lib,
      ...
    }:
    {
      packages = rec {
        default = way-shell;
        way-shell = pkgs.stdenv.mkDerivation {
          pname = "way-shell";
          inherit (config.wayShell) version;
          src = config.wayShell.source;
          strictDeps = true;
          enableParallelBuilding = false;
          nativeBuildInputs = with pkgs; [
            pkg-config
            glib
            wayland-scanner
            python3
            # A shell wrapper lets the schema check reuse the exact environment
            # of the installed application with a GLib probe as its executable.
            (wrapGAppsHook4.override { makeWrapper = pkgs.makeShellWrapper; })
          ];
          buildInputs = with pkgs; [
            glib
            gtk4
            gtk4-layer-shell
            libadwaita
            json-glib
            libpulseaudio
            networkmanager
            pipewire
            upower
            wayland
            wayland-protocols
            wireplumber
            dconf
          ];
          installFlags = [ "PREFIX=$(out)" ];
          doCheck = true;
          checkTarget = "check";
          postInstall = ''
            glib-compile-schemas "$out/share/glib-2.0/schemas"
            install -Dm644 LICENSE "$out/share/licenses/way-shell/LICENSE"
            substituteInPlace "$out/lib/systemd/user/way-shell.service" \
              --replace-fail /usr/bin/way-shell "$out/bin/way-shell"
          '';
          meta = {
            description = "GNOME-like desktop shell for Sway";
            homepage = "https://github.com/ldelossa/way-shell";
            license = lib.licenses.gpl2Only;
            platforms = [ "x86_64-linux" ];
            mainProgram = "way-shell";
          };
        };
      };
    };
}
