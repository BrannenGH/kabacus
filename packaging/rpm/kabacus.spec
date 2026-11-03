# SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later

# The kabacus app (crates/kabacus-kde), built against the distribution's
# own Qt 6 and Kirigami: one spec for Fedora, EL (CentOS Stream, AlmaLinux)
# and openSUSE. packaging/rpm/build-rpm builds it in a container of each
# distribution; the specification's supported platforms say where it
# builds and why not elsewhere. The local gateway and automation CLI ship
# with the app; the gateway is also available as a container image.
#
# Defines:
#   pkg_version  scripts/package-version (0.1.0, or 0.1.0~git<n>.g<commit>)
#   pkg_release  1 for a tag; 1.<run number> for each CI build of master,
#                so no two different builds share a file name
#   pkg_source_url, pkg_commit
#                the repository and the exact commit the package is built
#                from (scripts/package-source), which the package names
# --with rustup: build with the rustup toolchain on PATH instead of the
# distribution's cargo, whose Rust is too old on EL. CI pins it to the
# same version on every distribution.

%bcond_with rustup

# Rust's own debug info would make a debuginfo package ten times the
# app's size; the repository only serves the app.
%global debug_package %{nil}
# Fedora and EL compile C++ with -flto=auto; cxx's objects are linked by
# rustc, not by an LTO-aware gcc.
%global _lto_cflags %{nil}

Name:           kabacus
Version:        %{pkg_version}
Release:        %{pkg_release}%{?dist}
Summary:        Manage hledger books
License:        GPL-3.0-or-later AND AGPL-3.0-or-later
URL:            https://github.com/BrannenGH/kabacus
VCS:            git+%{pkg_source_url}.git#%{pkg_commit}
Source0:        kabacus-%{version}.tar.gz
ExclusiveArch:  x86_64

BuildRequires:  gcc-c++
%if %{without rustup}
# The newest rust-version among Cargo.lock's crates.
BuildRequires:  cargo >= 1.89
BuildRequires:  rust >= 1.89
%endif
# cxx-qt links Qt Core, Gui, Qml, Quick and QuickControls2 and finds Qt
# through qmake6; qmlcachegen compiles the QML module.
%if 0%{?suse_version}
BuildRequires:  qt6-base-devel
BuildRequires:  qt6-declarative-devel
BuildRequires:  qt6-declarative-tools
BuildRequires:  qt6-quickcontrols2-devel
%else
BuildRequires:  qt6-qtbase-devel
BuildRequires:  qt6-qtdeclarative-devel
%endif
# KI18n translates the app (crates/kabacus-kde/src/translation.h); its
# library is found automatically at run time. Same name everywhere.
BuildRequires:  kf6-ki18n-devel

# The Qt libraries are found automatically. The QML modules the app
# imports are only loaded at run time, so they're listed here.
%if 0%{?suse_version}
# openSUSE's Qt packages provide each QML module they install.
Requires:       qt6qmlimport(QtQuick)
Requires:       qt6qmlimport(QtQuick.Controls)
Requires:       qt6qmlimport(QtQuick.Dialogs)
Requires:       qt6qmlimport(QtQuick.Layouts)
Requires:       qt6qmlimport(org.kde.kirigami)
Requires:       qt6qmlimport(org.kde.kirigamiaddons.components)
%else
# Fedora and EPEL don't, so these are the packages that install them.
# QtQuick, its Controls, Dialogs and Layouts are all qtdeclarative.
Requires:       qt6-qtdeclarative%{?_isa}
Requires:       kf6-kirigami%{?_isa}
Requires:       kf6-kirigami-addons%{?_isa}
%endif
# Plasma's look outside Plasma; the app works without it.
Recommends:     kf6-qqc2-desktop-style
Requires:       hicolor-icon-theme

%description
A Kirigami app for hledger books: one convergent UI for phone, tablet
and desktop. It reads and writes only through kabacus-server, which
validates and versions every change.

Source: %{pkg_source_url}/tree/%{pkg_commit}

%prep
%autosetup -n kabacus-%{version}

%build
# cxx-qt-build asks qmake which Qt to build against.
qmake="$(command -v qmake6 || command -v qmake-qt6 || true)"
[ -n "$qmake" ] && export QMAKE="$qmake"
cargo build --release --locked -p kabacus-kde --features gui
cargo build --release --locked -p kabacus-server --bins
# The translations (po/), compiled for KI18n.
cargo run --release --locked -p xtask -- i18n-build target/locale

%install
install -Dm755 target/release/kabacus %{buildroot}%{_bindir}/kabacus
install -Dm755 target/release/kabacus-server %{buildroot}%{_bindir}/kabacus-server
install -Dm755 target/release/kabacus-cli %{buildroot}%{_bindir}/kabacus-cli
install -Dm644 packaging/desktop/me.hallb.kabacus.desktop %{buildroot}%{_datadir}/applications/me.hallb.kabacus.desktop
install -Dm644 packaging/desktop/me.hallb.kabacus.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/me.hallb.kabacus.svg
install -Dm644 packaging/desktop/me.hallb.kabacus.metainfo.xml %{buildroot}%{_datadir}/metainfo/me.hallb.kabacus.metainfo.xml
install -Dm644 packaging/desktop/me.hallb.kabacus.mime.xml %{buildroot}%{_datadir}/mime/packages/me.hallb.kabacus.xml
install -Dm644 crates/kabacus-kde/kabacus.categories %{buildroot}%{_datadir}/qlogging-categories6/kabacus.categories
mkdir -p %{buildroot}%{_datadir}/locale
cp -r target/locale/. %{buildroot}%{_datadir}/locale/
# Where the package's source is (GPLv3 section 6).
mkdir -p %{buildroot}%{_docdir}/kabacus
SOURCE_URL=%{pkg_source_url} SOURCE_COMMIT=%{pkg_commit} scripts/package-source text > %{buildroot}%{_docdir}/kabacus/SOURCE
%find_lang kabacus

%files -f kabacus.lang
%license LICENSES/GPL-3.0-or-later.txt LICENSES/AGPL-3.0-or-later.txt
%doc %{_docdir}/kabacus/SOURCE
%{_bindir}/kabacus
%{_bindir}/kabacus-server
%{_bindir}/kabacus-cli
%{_datadir}/applications/me.hallb.kabacus.desktop
%{_datadir}/icons/hicolor/scalable/apps/me.hallb.kabacus.svg
%{_datadir}/metainfo/me.hallb.kabacus.metainfo.xml
%{_datadir}/mime/packages/me.hallb.kabacus.xml
%dir %{_datadir}/qlogging-categories6
%{_datadir}/qlogging-categories6/kabacus.categories

%changelog
* Thu Oct 15 2026 Brannen Hall <brannen@hallb.me>
- Package the app for Fedora, EL and openSUSE; history is in git.
