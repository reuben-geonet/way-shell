{
  perSystem =
    {
      config,
      pkgs,
      lib,
      ...
    }:
    {
      devShells.default = pkgs.mkShell {
        inputsFrom = [ config.packages.way-shell ];
        packages = with pkgs; [ gdb rustc cargo rustfmt clippy rust-analyzer dbus pipewire wireplumber ];
        nativeBuildInputs = [ pkgs.rustPlatform.bindgenHook ];
        shellHook = ''
            export GSETTINGS_SCHEMA_DIR="$PWD/.cache/nix/schemas"
          mkdir -p "$GSETTINGS_SCHEMA_DIR"
          rm -f "$GSETTINGS_SCHEMA_DIR/"*.gschema.xml
            cp data/*.gschema.xml "$GSETTINGS_SCHEMA_DIR/"
            glib-compile-schemas --strict "$GSETTINGS_SCHEMA_DIR"
            export GIO_EXTRA_MODULES="${lib.getLib pkgs.dconf}/lib/gio/modules''${GIO_EXTRA_MODULES:+:$GIO_EXTRA_MODULES}"
        '';
      };
    };
}
