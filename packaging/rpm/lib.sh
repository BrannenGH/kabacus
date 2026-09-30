# SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
#
# SPDX-License-Identifier: GPL-3.0-or-later

# Sourced by the scripts that run inside a distribution's container
# (build-in-container): which family it is, and how to
# install packages there.

. /etc/os-release
case "$ID" in
  fedora) family=fedora ;;
  opensuse*|sles|sled|sle-micro|sl-micro) family=suse ;;
  *)
    case " $ID_LIKE " in
      *" rhel "*) family=el ;;
      *) echo "kabacus rpm: unsupported distribution $ID" >&2; exit 1 ;;
    esac ;;
esac

pkg_install() {
  case "$family" in
    suse) zypper --non-interactive install --no-recommends "$@" ;;
    *) dnf install -y --setopt=install_weak_deps=False "$@" ;;
  esac
}

# EL keeps Qt 6's development packages in CRB and KDE Frameworks 6 in
# EPEL; EPEL's crb script turns on whichever CRB this release has.
# crb runs dnf config-manager, which is in dnf-plugins-core: the minimal
# CentOS Stream and AlmaLinux images don't have it, and crb aborts.
# CentOS Stream 9 also needs EPEL Next, built against Stream's newer
# libraries.
enable_el_repos() {
  [ "$family" = el ] || return 0
  pkg_install epel-release dnf-plugins-core
  crb enable
  if [ "$ID" = centos ] && [ "${VERSION_ID%%.*}" = 9 ]; then
    pkg_install epel-next-release
  fi
}
