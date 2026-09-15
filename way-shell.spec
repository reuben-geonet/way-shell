%define _userunitdir /usr/lib/systemd/user

Name: way-shell
Version: 0.0.10
Release: 1%{?dist}
Summary: A Gnome-like desktop shell for Wayland compositors.
License: GPL-2.0-only

URL: https://github.com/ldelossa/way-shell
Source0: %{name}-%{version}.tar.gz

BuildRequires: gcc
BuildRequires: make
BuildRequires: pkgconfig
BuildRequires: meson
BuildRequires: cmake
BuildRequires: gtk-doc
BuildRequires: wayland-devel
BuildRequires: glib2-devel
BuildRequires: python3
BuildRequires: rust >= 1.90
BuildRequires: cargo >= 1.90
BuildRequires: clang-devel
BuildRequires: pipewire
BuildRequires: pipewire-pulseaudio
BuildRequires: wireplumber
BuildRequires: dbus-daemon

BuildRequires: pkgconfig(libadwaita-1)
BuildRequires: pkgconfig(upower-glib)
BuildRequires: pkgconfig(wireplumber-0.5)
BuildRequires: pkgconfig(json-glib-1.0)
BuildRequires: pkgconfig(libnm)
BuildRequires: pkgconfig(libpipewire-0.3)
BuildRequires: pkgconfig(libpulse)
BuildRequires: pkgconfig(libpulse-simple)
BuildRequires: pkgconfig(libpulse-mainloop-glib)
BuildRequires: pkgconfig(wayland-client)
BuildRequires: pkgconfig(wayland-protocols)
BuildRequires: pkgconfig(gio-unix-2.0)
BuildRequires: pkgconfig(gtk4-layer-shell-0)

Requires: NetworkManager
Requires: wireplumber
Requires: upower
Requires: (power-profiles-daemon or tuned-ppd)
Requires: systemd
Requires: wayland-devel

%description
A GNOME-inspired desktop shell for Sway and Niri, written in Rust with GTK4,
libadwaita and gtk4-layer-shell.

Way-Shell expects a Gnome-like environment to be available.
This means DBus must be running and the following services must be available:

- Logind
- NetworkManager
- WirePlumber/Pipewire
- power-profiles-daemon or tuned-ppd
- UPower

If you're using Fedora these services should be available by default.

%prep
%setup -q

%build
export CARGO_BUILD_JOBS=1
cargo build --workspace --bins --release --frozen --jobs 1

%check
cargo test --workspace --frozen --jobs 1
cargo build -p way-shell --example schema-probe --frozen --jobs 1
if [ -n "${WAY_SHELL_TEST_ARTIFACTS:-}" ]; then
    install -Dm755 target/debug/examples/schema-probe "$WAY_SHELL_TEST_ARTIFACTS/schema-probe"
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
* Mon May 27 2024 Louis DeLosSantos <louis.delos.deve@gmail.com>
- Initial packaging
