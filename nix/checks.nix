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
      schemaProbe = package.testHelpers;
    in
    {
      checks =
        lib.mapAttrs' (
          release: output: lib.nameValuePair "rpm-fedora-${release}" output
        ) config.wayShell.verified
        // {
          native-package = pkgs.runCommand "way-shell-native-package-check" { } ''
            set -x
            mkdir -p "$out"
            test -x ${package}/bin/way-shell
            test -x ${package}/bin/way-sh
            test ! -e ${package}/bin/schema-probe
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
            # Only substitute its final executable with the Rust/GIO schema probe.
            test -x ${schemaProbe}/bin/schema-probe
            # The helper must not add its own wrapper environment to this test.
            test "$(head -c 4 ${schemaProbe}/bin/schema-probe)" = "$(printf '\177ELF')"
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
