%define _userunitdir /usr/lib/systemd/user

Name: way-shell
Version: 0.0.10
Release: 10%{?dist}
Summary: A Gnome-like desktop shell for Wayland compositors.
License: GPL-2.0-only

URL: https://github.com/ldelossa/way-shell
Source0: %{name}-%{version}.tar.gz

BuildRequires: gcc
BuildRequires: pkgconfig
BuildRequires: wayland-devel
BuildRequires: glib2-devel
BuildRequires: rust >= 1.90
BuildRequires: cargo >= 1.90
BuildRequires: clang-devel
BuildRequires: pipewire
BuildRequires: pipewire-pulseaudio
BuildRequires: wireplumber
BuildRequires: dbus-daemon

BuildRequires: pkgconfig(libadwaita-1)
BuildRequires: pkgconfig(wireplumber-0.5)
BuildRequires: pkgconfig(libnm)
BuildRequires: pkgconfig(libpipewire-0.3)
BuildRequires: pkgconfig(libpulse)
BuildRequires: pkgconfig(libpulse-mainloop-glib)
BuildRequires: pkgconfig(wayland-client)
BuildRequires: pkgconfig(gtk4-layer-shell-0)

Requires: NetworkManager
Requires: wireplumber
Requires: upower
Requires: (power-profiles-daemon or tuned-ppd)
Requires: systemd

%description
A GNOME-inspired desktop shell for Sway and Niri, written in Rust with GTK4,
libadwaita and gtk4-layer-shell.

Way-Shell requires a Wayland session and its selected Sway or Niri compositor.
A session D-Bus enables notification, tray and media integrations. Desktop
services supply the corresponding optional controls:

- Logind
- NetworkManager
- WirePlumber/Pipewire
- power-profiles-daemon or tuned-ppd
- UPower

Unavailable optional services disable their controls until they recover.

%prep
%setup -q

%build
export CARGO_BUILD_JOBS=1
cargo build --workspace --bins --release --frozen --jobs 1

%check
cargo test --workspace --frozen --jobs 1
cargo build -p way-shell --example schema-probe --example application-smoke --frozen --jobs 1
if [ -n "${WAY_SHELL_TEST_ARTIFACTS:-}" ]; then
    install -Dm755 target/debug/examples/schema-probe "$WAY_SHELL_TEST_ARTIFACTS/schema-probe"
    install -Dm755 target/debug/examples/application-smoke "$WAY_SHELL_TEST_ARTIFACTS/application-smoke"
fi
cargo build -p way-shell --example audio-compat --frozen --jobs 1
sh tests/audio-compat.sh target/debug/examples/audio-compat

%install
DESTDIR="%{buildroot}" PREFIX="%{_prefix}" BINDIR="%{_bindir}" \
    SCHEMADIR="%{_datadir}/glib-2.0/schemas" USERUNITDIR="%{_userunitdir}" \
    LICENSEDIR="%{_datadir}/licenses/%{name}" CARGO_ARTIFACT_DIR=target/release \
    DEPENDENCY_LICENSES="$PWD/dependency-licenses" sh scripts/install.sh

%post
glib-compile-schemas %{_datadir}/glib-2.0/schemas

%postun
glib-compile-schemas %{_datadir}/glib-2.0/schemas

%files
%license %{_datadir}/licenses/%{name}
%{_bindir}/way-shell
%{_bindir}/way-sh
%{_datadir}/glib-2.0/schemas/org.ldelossa.way-shell.gschema.xml
%{_userunitdir}/way-shell.service

%changelog
* Wed Sep 16 2026 Way Shell contributors - 0.0.10-10
- Port Bluetooth quick settings, radio transitions and recovery to Rust.

* Wed Sep 16 2026 Way Shell contributors - 0.0.10-9
- Complete the Rust migration with Cargo builds and Rust test helpers.
- Preserve Sway/Niri integration and both power-profile providers.

* Mon May 27 2024 Louis DeLosSantos <louis.delos.deve@gmail.com>
- Initial packaging
