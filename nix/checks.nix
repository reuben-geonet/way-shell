{
  perSystem =
    {
      config,
      pkgs,
      lib,
      ...
    }:
    let
      package = config.packages.way-shell;
      schemaProbe =
        pkgs.runCommandCC "way-shell-schema-probe"
          {
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = [ pkgs.glib ];
          }
          ''
            mkdir -p "$out/bin"
            $CC ${./tests/schema-probe.c} -o "$out/bin/schema-probe" $(pkg-config --cflags --libs gio-2.0)
          '';
    in
    {
      checks =
        lib.mapAttrs' (
          release: output: lib.nameValuePair "rpm-fedora-${release}" output
        ) config.wayShell.verified
        // {
          bluetooth = pkgs.runCommand "way-shell-bluetooth-check" {
            nativeBuildInputs = [ pkgs.gcc pkgs.pkg-config pkgs.glib pkgs.dbus ];
            buildInputs = [ pkgs.glib ];
          } ''
            mkdir -p "$out" schemas
            cp ${../data/org.ldelossa.way-shell.gschema.xml} schemas/
            glib-compile-schemas --strict schemas
            $CC -g -Wall ${../tests/bluetooth.c} \
              ${../src/services/bluetooth_service/bluetooth_service.c} \
              ${../src/services/bluetooth_service/bluetooth_settings.c} \
              -I${../src/services/bluetooth_service} \
              -o bluetooth-test $(pkg-config --cflags --libs gio-2.0 gio-unix-2.0)
            GSETTINGS_SCHEMA_DIR="$PWD/schemas" GSETTINGS_BACKEND=memory \
              ./bluetooth-test > "$out/tests.txt"
          '';
          native-package = pkgs.runCommand "way-shell-native-package-check" { } ''
            set -x
            mkdir -p "$out"
            test -x ${package}/bin/way-shell
            test -x ${package}/bin/way-sh
            test -s ${package}/share/licenses/way-shell/LICENSE
            test -s ${package}/share/gsettings-schemas/${package.name}/glib-2.0/schemas/gschemas.compiled
            grep -Fx 'ExecStart=${package}/bin/way-shell' ${package}/lib/systemd/user/way-shell.service
            grep -F '.way-shell-wrapped' ${package}/bin/way-shell
            env -i HOME="$TMPDIR" ${package}/bin/way-shell --help > "$out/help.txt"
          '';
          native-schemas = pkgs.runCommand "way-shell-native-schemas-check" { } ''
            set -x
            mkdir -p "$out" empty home
            # Reuse the installed application's wrapper verbatim up to exec.
            # Only substitute its final executable with the GLib schema probe.
            grep -q '^exec ' ${package}/bin/way-shell
            grep -F '${lib.getLib pkgs.dconf}/lib/gio/modules' ${package}/bin/way-shell
            sed '/^exec /,$d' ${package}/bin/way-shell > probe-wrapper
            echo 'exec ${schemaProbe}/bin/schema-probe ${lib.escapeShellArgs config.wayShell.schemaIds}' >> probe-wrapper
            env -i HOME="$PWD/home" XDG_DATA_HOME="$PWD/empty" XDG_DATA_DIRS="$PWD/empty" \
              GSETTINGS_BACKEND=memory ${pkgs.bash}/bin/bash ./probe-wrapper > "$out/schemas.txt"
          '';
        };
    };
}
