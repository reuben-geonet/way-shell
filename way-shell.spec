Name: way-shell
Version: 0.0.10
Release: 11%{?dist}
Summary: A Gnome-like desktop shell for Wayland compositors.
License: GPL-2.0-only

URL: https://github.com/ldelossa/way-shell
Source0: %{name}-%{version}.tar.gz

BuildRequires: gcc
BuildRequires: glib2-devel
BuildRequires: rust >= 1.90
BuildRequires: cargo >= 1.90
BuildRequires: cargo-rpm-macros >= 26.4
BuildRequires: systemd-rpm-macros
BuildRequires: clang-devel

BuildRequires: pkgconfig(libadwaita-1)
BuildRequires: pkgconfig(wireplumber-0.5)
BuildRequires: pkgconfig(libnm)
BuildRequires: pkgconfig(libpipewire-0.3)
BuildRequires: pkgconfig(libpulse)
BuildRequires: pkgconfig(libpulse-mainloop-glib)
BuildRequires: pkgconfig(wayland-client)
BuildRequires: pkgconfig(gtk4-layer-shell-0)

Recommends: NetworkManager
Recommends: bluez
Recommends: pipewire
Recommends: pipewire-pulseaudio
Recommends: wireplumber
Recommends: upower
Recommends: (power-profiles-daemon or tuned-ppd)
Recommends: systemd
%{?systemd_ordering}

%description
A GNOME-inspired desktop shell for Sway and Niri, built with GTK4 and libadwaita.
Desktop services provide optional network, Bluetooth, audio, power and session
controls. Unavailable services disable their controls until they recover.

%prep
%autosetup
# Keep the pinned registry and Git source replacements supplied by Nix.
cp .cargo/config.toml vendor-config.toml
%cargo_prep -N
cat vendor-config.toml >> .cargo/config.toml
%cargo_vendor_manifest
# Fedora 43 workaround: cargo2rpm 0.1.18 rejects Git source suffixes.
# Remove this normalization when the oldest supported generator accepts them.
# Cargo.lock retains the pinned Git revisions.
sed -i 's/ (.*)//' cargo-vendor.txt
# Expand without macro arguments so the shell receives the redirections.
%{cargo_license} > cargo-licenses.txt

%build
%cargo_build -- --workspace --bins --frozen

%check
target/release/way-shell --help
target/release/way-sh --help

%install
DESTDIR="%{buildroot}" PREFIX="%{_prefix}" BINDIR="%{_bindir}" \
    SCHEMADIR="%{_datadir}/glib-2.0/schemas" USERUNITDIR="%{_userunitdir}" \
    LICENSEDIR="%{_datadir}/licenses/%{name}" CARGO_ARTIFACT_DIR=target/release \
    DEPENDENCY_LICENSES="$PWD/dependency-licenses" sh scripts/install.sh

install -m644 cargo-vendor.txt cargo-licenses.txt %{buildroot}%{_datadir}/licenses/%{name}/

%post
%systemd_user_post way-shell.service

%preun
%systemd_user_preun way-shell.service

%files
%license %{_datadir}/licenses/%{name}
%{_bindir}/way-shell
%{_bindir}/way-sh
%{_datadir}/glib-2.0/schemas/org.ldelossa.way-shell.gschema.xml
%{_userunitdir}/way-shell.service

%changelog
* Thu Sep 17 2026 Way Shell contributors - 0.0.10-11
- Modernize offline Cargo packaging, optional services and user service lifecycle.

* Wed Sep 16 2026 Way Shell contributors - 0.0.10-10
- Port Bluetooth quick settings, radio transitions and recovery to Rust.

* Wed Sep 16 2026 Way Shell contributors - 0.0.10-9
- Complete the Rust migration with Cargo builds and Rust test helpers.
- Preserve Sway/Niri integration and both power-profile providers.

* Mon May 27 2024 Louis DeLosSantos <louis.delos.deve@gmail.com>
- Initial packaging
