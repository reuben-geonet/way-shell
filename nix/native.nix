{
  perSystem =
    {
      config,
      pkgs,
      lib,
      ...
    }:
    {
      # wf-recorder 0.6.0 binds DMA-BUF v4 even when only v3 is advertised,
      # disconnecting from software-rendered Niri before SHM capture can start.
      # Remove this override when the pinned upstream version negotiates the
      # protocol version or skips unsupported DMA-BUF feedback requests.
      wayShell.recorder = pkgs.wf-recorder.overrideAttrs (old: {
        patches = (old.patches or [ ]) ++ lib.optional (old.version == "0.6.0")
          ./patches/wf-recorder-0.6.0-dmabuf-version.patch;
      });
      packages = rec {
        default = way-shell;
        way-shell = pkgs.rustPlatform.buildRustPackage (
          config.wayShell.rustBuildArgs
          // {
            pname = "way-shell";
            nativeBuildInputs = config.wayShell.rustBuildArgs.nativeBuildInputs ++ [ pkgs.wrapGAppsHook4 ];
            preFixup = ''
              gappsWrapperArgs+=(--prefix PATH : ${lib.makeBinPath [ pkgs.grim config.wayShell.recorder pkgs.ffmpeg_8 ]})
            '';
            # Keep the artifact path used by the shared installer.
            buildPhase = ''
              runHook preBuild
              cargo build --workspace --bins --release --frozen --jobs 1
              runHook postBuild
            '';
            # Keep Cargo tests separate from package builds.
            doCheck = false;
            installPhase = ''
              runHook preInstall
              PREFIX="$out" CARGO_ARTIFACT_DIR=target/release \
                DEPENDENCY_LICENSES=${config.wayShell.dependencyLicenses} \
                sh scripts/install.sh
              runHook postInstall
            '';
            postInstall = ''
              glib-compile-schemas "$out/share/glib-2.0/schemas"
              substituteInPlace "$out/lib/systemd/user/way-shell.service" \
                --replace-fail /usr/bin/way-shell "$out/bin/way-shell"
            '';
            meta = {
              description = "GNOME-like desktop shell for Sway and Niri";
              homepage = "https://github.com/ldelossa/way-shell";
              license = lib.licenses.agpl3Only;
              platforms = [ "x86_64-linux" ];
              mainProgram = "way-shell";
            };
          }
        );
      };
    };
}
