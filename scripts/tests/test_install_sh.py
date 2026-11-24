"""Tests of scripts/install.sh, the one-command Linux installer.

Run from the repository root:  python3 -m unittest discover -s scripts/tests -t .
Standard library only. Nothing here touches the real system: every run gets a private PATH made of a few real
tools plus recording stubs for uname, id, ldd, sudo and the package managers, a fake root file system for
/etc/os-release and library folders (HEXIS_ROOTFS), a throw-away HOME, and a local HTTP server that plays
GitHub and the project site. What these tests do NOT prove: that a real apt/dnf/zypper/pacman accepts the commands
(that is what the container matrix in CI and scripts/test_installer_containers.sh are for).
"""
import hashlib
import http.server
import io
import os
import shutil
import stat
import subprocess
import tarfile
import tempfile
import threading
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parent.parent
INSTALLER = REPO_ROOT / "scripts" / "install.sh"

REAL_TOOLS = ["sed", "grep", "head", "tail", "cat", "cut", "tr", "sort", "mktemp", "mkdir", "cp", "rm", "rmdir",
              "chmod", "find", "tar", "dirname", "basename", "sha256sum", "curl", "mv", "gzip", "install", "ls"]

OS_RELEASES = {
    "ubuntu": 'ID=ubuntu\nID_LIKE=debian\nPRETTY_NAME="Ubuntu 22.04.5 LTS"\n',
    "debian": 'ID=debian\nPRETTY_NAME="Debian GNU/Linux 12 (bookworm)"\n',
    "mint": 'ID=linuxmint\nID_LIKE="ubuntu debian"\nPRETTY_NAME="Linux Mint 22"\n',
    "fedora": 'ID=fedora\nPRETTY_NAME="Fedora Linux 41"\n',
    "rocky": 'ID="rocky"\nID_LIKE="rhel centos fedora"\nPRETTY_NAME="Rocky Linux 9"\n',
    "opensuse": 'ID="opensuse-tumbleweed"\nID_LIKE="opensuse suse"\nPRETTY_NAME="openSUSE Tumbleweed"\n',
    "arch": 'ID=arch\nPRETTY_NAME="Arch Linux"\n',
    "cachyos": 'ID=cachyos\nID_LIKE=arch\nPRETTY_NAME="CachyOS"\n',
    "void": 'ID=void\nPRETTY_NAME="Void Linux"\n',
    "alpine": 'ID=alpine\nPRETTY_NAME="Alpine Linux"\n',
    "nixos": 'ID=nixos\nPRETTY_NAME="NixOS"\n',
    "silverblue": 'ID=fedora\nVARIANT_ID=silverblue\nPRETTY_NAME="Fedora Linux 41 (Silverblue)"\n',
}

RECORDER = '#!/bin/sh\necho "$(basename "$0") $*" >> "$STUB_LOG"\nexit 0\n'


def sha(b):
    return hashlib.sha256(b).hexdigest()


class FakeWeb:
    """A tiny HTTP server: path -> bytes. Anything else is 404."""

    def __init__(self, files):
        files_ = files

        class H(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                body = files_.get(self.path)
                if body is None:
                    self.send_response(404); self.end_headers(); return
                self.send_response(200); self.send_header("Content-Length", str(len(body))); self.end_headers()
                self.wfile.write(body)

            def log_message(self, *a):
                pass

        self.srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.url = f"http://127.0.0.1:{self.srv.server_address[1]}"
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()

    def close(self):
        self.srv.shutdown(); self.srv.server_close()


def make_release(version="0.1.0", apk=True):
    """Fake release files by name, and their SHA256SUMS."""
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tf:
        def add(name, data, mode=0o644):
            ti = tarfile.TarInfo(f"hexis-{version}/{name}"); ti.size = len(data); ti.mode = mode
            tf.addfile(ti, io.BytesIO(data))
        add("hexis", b"#!/bin/sh\necho hexis-fake\n", 0o755)
        add("hexis.desktop", b"[Desktop Entry]\nType=Application\nName=Hexis\nExec=hexis\nIcon=hexis\n")
        add("hexis.png", b"\x89PNG fake")
        add("LICENSE", b"MIT")
    files = {
        f"hexis-{version}-linux-x86_64.tar.gz": buf.getvalue(),
        f"Hexis_{version}_amd64.deb": b"deb bytes",
        f"Hexis-{version}-1.x86_64.rpm": b"rpm bytes",
        f"Hexis_{version}_amd64.AppImage": b"appimage bytes",
    }
    if apk:
        files[f"hexis-{version}-android-arm64-testkey.apk"] = b"apk bytes"
    sums = "".join(f"{sha(v)}  {k}\n" for k, v in sorted(files.items())).encode()
    files["SHA256SUMS"] = sums
    return files


class InstallerCase(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="hexis-inst-test-"))
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.home = self.tmp / "home"; self.home.mkdir()
        (self.tmp / "tmp").mkdir()
        self.root = self.tmp / "rootfs"; (self.root / "etc").mkdir(parents=True)
        self.log = self.tmp / "stub.log"; self.log.write_text("")
        self.bin = self.tmp / "bin"; self.bin.mkdir()
        for t in REAL_TOOLS:
            p = shutil.which(t)
            if p:
                (self.bin / t).symlink_to(p)
        self.webs = []

    def web(self, files):
        w = FakeWeb(files); self.webs.append(w); self.addCleanup(w.close); return w

    def stub(self, name, body=RECORDER):
        p = self.bin / name
        if p.is_symlink() or p.exists():
            p.unlink()
        p.write_text(body); p.chmod(0o755)

    def os_release(self, key):
        (self.root / "etc" / "os-release").write_text(OS_RELEASES[key])

    def system(self, distro, tools=(), uid=1000, glibc="2.36", musl=False, machine="x86_64", kernel="Linux", os_name="GNU/Linux"):
        """Describe the pretend machine."""
        self.os_release(distro)
        self.stub("uname", f'#!/bin/sh\ncase "$1" in -s) echo {kernel};; -m) echo {machine};; -o) echo {os_name};; *) echo {kernel};; esac\n')
        self.stub("id", f'#!/bin/sh\necho {uid}\n')
        if musl:
            self.stub("ldd", '#!/bin/sh\necho "musl libc (x86_64)" >&2; echo "Version 1.2.5" >&2; exit 1\n')
        else:
            self.stub("ldd", f'#!/bin/sh\necho "ldd (Debian GLIBC {glibc}-1) {glibc}"\n')
        for t in tools:
            if t == "dpkg":
                self.stub("dpkg", '#!/bin/sh\necho "dpkg $*" >> "$STUB_LOG"\n[ "$1" = "-s" ] && exit 1\nexit 0\n')
            elif t == "rpm":
                self.stub("rpm", '#!/bin/sh\necho "rpm $*" >> "$STUB_LOG"\n[ "$1" = "-q" ] && exit 1\nexit 0\n')
            else:
                self.stub(t)

    def lib(self, name):
        d = self.root / "usr" / "lib"; d.mkdir(parents=True, exist_ok=True); (d / name).write_text("")

    def run_installer(self, *args, env=None, site=None, base=None, expect=None):
        e = {"PATH": str(self.bin), "HOME": str(self.home), "TMPDIR": str(self.tmp / "tmp"), "LC_ALL": "C",
             "STUB_LOG": str(self.log), "HEXIS_REPO": "owner/project", "HEXIS_SITE": site or "http://127.0.0.1:9",
             "HEXIS_ROOTFS": str(self.root)}
        if base:
            e["HEXIS_BASE_URL"] = base
        e.update(env or {})
        r = subprocess.run(["/bin/sh", str(INSTALLER), *args], env=e, capture_output=True, text=True, timeout=60)
        r.out = r.stdout + r.stderr
        if expect is not None:
            self.assertEqual(r.returncode, expect, r.out)
        return r

    def calls(self):
        return self.log.read_text().splitlines()


def repo_site(apt=True, rpm=True):
    f = {"/hexis-archive-keyring.gpg": b"key", "/hexis-archive-keyring.asc": b"key"}
    if apt:
        f["/apt/dists/stable/InRelease"] = b"fake"
    if rpm:
        f["/rpm/x86_64/repodata/repomd.xml"] = b"<fake/>"
    return f


class Detection(InstallerCase):
    def test_detect_reports_family_package_manager_and_helper(self):
        self.system("mint", tools=["apt-get", "dpkg", "sudo"])
        r = self.run_installer("--detect", expect=0)
        self.assertIn("family=debian", r.out)
        self.assertRegex(r.out, r"package manager\s+apt")
        self.assertRegex(r.out, r"root helper\s+sudo")
        self.assertRegex(r.out, r"libc\s+glibc 2.36")

    def test_derivatives_map_to_their_parent_family(self):
        cases = {"rocky": ("dnf", "redhat"), "opensuse": ("zypper", "suse"), "cachyos": ("pacman", "arch"), "ubuntu": ("apt-get", "debian")}
        for distro, (pm, fam) in cases.items():
            with self.subTest(distro=distro):
                self.system(distro, tools=[pm, "dpkg"] if pm == "apt-get" else [pm])
                self.assertIn(f"family={fam}", self.run_installer("--detect", expect=0).out)

    def test_library_probe_uses_the_root_folders(self):
        self.system("void", tools=[])
        self.assertRegex(self.run_installer("--detect").out, r"webkit2gtk-4.1\+gtk3\s+0")
        self.lib("libwebkit2gtk-4.1.so.0"); self.lib("libgtk-3.so.0")
        self.assertRegex(self.run_installer("--detect").out, r"webkit2gtk-4.1\+gtk3\s+1")


class Plans(InstallerCase):
    def plan(self, *args, **kw):
        return self.run_installer("--dry-run", *args, **kw)

    def test_debian_family_uses_the_signed_apt_repository(self):
        self.system("ubuntu", tools=["apt-get", "dpkg", "sudo"])
        web = self.web(repo_site())
        r = self.plan(site=web.url, expect=0)
        self.assertIn("Plan: repo", r.out)
        self.assertIn("+ sudo install -Dm644", r.out)
        self.assertIn("/etc/apt/sources.list.d/hexis.sources", r.out)
        self.assertIn("+ sudo apt-get install -y hexis", r.out)
        self.assertEqual(self.calls(), [], "a dry run must not run anything")

    def test_fedora_uses_the_rpm_repository_with_dnf(self):
        self.system("fedora", tools=["dnf", "sudo"])
        web = self.web(repo_site())
        r = self.plan(site=web.url, expect=0)
        self.assertIn("Plan: repo", r.out)
        self.assertIn("/etc/yum.repos.d/hexis.repo", r.out)
        self.assertIn("+ sudo dnf install -y hexis", r.out)

    def test_opensuse_uses_zypper_and_its_own_repo_folder(self):
        self.system("opensuse", tools=["zypper", "sudo"])
        web = self.web(repo_site())
        r = self.plan(site=web.url, expect=0)
        self.assertIn("/etc/zypp/repos.d/hexis.repo", r.out)
        self.assertIn("zypper --non-interactive install hexis", r.out)

    def test_without_a_published_repository_it_falls_back_to_the_release_file(self):
        self.system("ubuntu", tools=["apt-get", "dpkg", "sudo"])
        web = self.web({"/" + k: v for k, v in make_release().items()})
        r = self.plan(site=web.url, base=web.url, expect=0)  # the site has no /apt/ files
        self.assertIn("repository is not published yet", r.out)
        self.assertIn("Plan: deb", r.out)
        self.assertIn("Hexis_0.1.0_amd64.deb", r.out)

    def test_arch_installs_the_tarball_and_gets_libraries_from_pacman(self):
        self.system("arch", tools=["pacman", "sudo"])
        rel = {"/" + k: v for k, v in make_release().items()}
        web = self.web(rel)
        r = self.plan(base=web.url, expect=0)
        self.assertIn("Plan: tarball", r.out)
        self.assertIn("+ sudo pacman -S --needed --noconfirm webkit2gtk-4.1 gtk3", r.out)
        self.assertIn("into /usr/local", r.out)

    def test_arch_uses_the_signed_pacman_repository_when_published(self):
        self.system("arch", tools=["pacman", "pacman-key", "sudo"])
        fpr = "340B746B21B3559ECD22528F1FA528212EB790E9"
        files = {**repo_site(), "/arch/x86_64/hexis.db": b"db", "/arch/x86_64/hexis.db.sig": b"sig",
                 "/key-fingerprint.txt": (fpr + "\n").encode()}
        web = self.web(files)
        r = self.plan(site=web.url, expect=0)
        self.assertIn("Plan: repo", r.out)
        self.assertIn(f"+ sudo pacman-key --lsign-key {fpr}", r.out)
        self.assertIn("+ sudo tee -a /etc/pacman.conf", r.out)
        self.assertIn("/var/lib/pacman/sync/hexis.db", r.out)
        self.assertIn("+ sudo pacman -S --needed --noconfirm hexis", r.out)
        self.assertNotIn("-Sy", r.out, "refreshing every repository first is the partial upgrade Arch warns about")

    def test_a_malformed_key_fingerprint_stops_the_pacman_setup(self):
        self.system("arch", tools=["pacman", "pacman-key", "sudo"])
        files = {**repo_site(), "/arch/x86_64/hexis.db": b"db", "/key-fingerprint.txt": b"not a fingerprint\n"}
        web = self.web(files)
        r = self.plan(site=web.url, expect=1)
        self.assertIn("not a fingerprint", r.out)

    def test_unknown_distribution_installs_for_the_user_only(self):
        self.system("void", tools=[])
        rel = {"/" + k: v for k, v in make_release().items()}
        web = self.web(rel)
        r = self.plan(base=web.url, expect=0)
        self.assertIn("Plan: appimage", r.out)
        self.lib("libwebkit2gtk-4.1.so.0"); self.lib("libgtk-3.so.0")
        self.assertIn("Plan: tarball", self.plan(base=web.url, expect=0).out)

    def test_no_root_helper_means_a_user_install(self):
        self.system("ubuntu", tools=["apt-get", "dpkg"])  # no sudo/doas
        rel = {"/" + k: v for k, v in make_release().items()}
        web = self.web(rel)
        r = self.plan(base=web.url, expect=0)
        self.assertIn(str(self.home / ".local"), r.out)
        self.assertNotIn("Plan: deb", r.out)

    def test_immutable_system_is_never_changed_with_its_package_manager(self):
        self.system("silverblue", tools=["dnf", "sudo"])
        (self.root / "run").mkdir(); (self.root / "run" / "ostree-booted").write_text("")
        rel = {"/" + k: v for k, v in make_release().items()}
        web = self.web(rel)
        r = self.plan(base=web.url, expect=0)
        self.assertNotIn("dnf", r.out.replace("package manager", ""))
        self.assertIn(str(self.home / ".local"), r.out)

    def test_nightly_and_pinned_versions_skip_the_repository(self):
        self.system("ubuntu", tools=["apt-get", "dpkg", "sudo"])
        rel = {"/" + k: v for k, v in make_release().items()}
        web = self.web({**rel, **repo_site()})
        for args in (["--nightly"], ["--version", "0.1.0"]):
            with self.subTest(args=args):
                r = self.plan(*args, site=web.url, base=web.url, expect=0)
                self.assertIn("Plan: deb", r.out)

    def test_method_repo_with_a_nightly_channel_is_an_error(self):
        self.system("ubuntu", tools=["apt-get", "dpkg", "sudo"])
        r = self.plan("--method", "repo", "--nightly", expect=1)
        self.assertIn("stable releases only", r.out)

    def test_wrong_method_for_the_system_is_refused(self):
        self.system("arch", tools=["pacman", "sudo"])
        self.assertIn("needs apt", self.plan("--method", "deb", expect=1).out)

    def test_termux_gets_the_apk(self):
        self.system("void", machine="aarch64", os_name="Android")
        rel = {"/" + k: v for k, v in make_release().items()}
        web = self.web(rel)
        r = self.plan(base=web.url, expect=0)
        self.assertIn("Plan: apk", r.out)


class Refusals(InstallerCase):
    def test_musl_is_refused_with_a_reason(self):
        self.system("alpine", musl=True)
        r = self.run_installer(expect=1)
        self.assertIn("musl", r.out)

    def test_old_glibc_is_refused(self):
        self.system("debian", tools=["apt-get", "dpkg"], glibc="2.31")
        r = self.run_installer(expect=1)
        self.assertIn("glibc 2.31 is too old", r.out)

    def test_glibc_at_the_floor_is_accepted(self):
        self.system("debian", tools=["apt-get", "dpkg", "sudo"], glibc="2.34")
        self.run_installer("--detect", expect=0)
        self.run_installer("--dry-run", "--method", "repo", expect=1)  # fails only because no repository is published

    def test_other_cpus_are_refused_but_pointed_to_the_apk(self):
        self.system("debian", tools=["apt-get", "dpkg"], machine="aarch64")
        r = self.run_installer(expect=1)
        self.assertIn("--apk", r.out)

    def test_nixos_and_other_kernels_are_refused(self):
        self.system("nixos")
        self.assertIn("NixOS", self.run_installer(expect=1).out)
        self.system("debian", kernel="Darwin")
        self.assertIn("Linux and Android", self.run_installer(expect=1).out)

    def test_unbuilt_copy_refuses_to_run(self):
        self.system("debian", tools=["apt-get", "dpkg"])
        r = self.run_installer("--detect", env={"HEXIS_REPO": "@REPO@"}, expect=1)
        self.assertIn("not built for a project", r.out)

    def test_options(self):
        self.system("debian", tools=["apt-get", "dpkg"])
        self.assertIn("Usage:", self.run_installer("--help", expect=0).out)
        self.assertIn("unknown option", self.run_installer("--frobnicate", expect=1).out)
        self.assertIn("unknown channel", self.run_installer(env={"HEXIS_CHANNEL": "beta"}, expect=1).out)


class RealInstalls(InstallerCase):
    """These run the file operations for real, inside the throw-away HOME."""

    def setUp(self):
        super().setUp()
        self.system("void", tools=[])
        self.rel = make_release()
        self.web_files = {"/" + k: v for k, v in self.rel.items()}

    def test_user_tarball_install_then_uninstall_keeps_the_vault(self):
        web = self.web(self.web_files)
        self.run_installer("--method", "tarball", "--user", base=web.url, expect=0)
        prog = self.home / ".local" / "bin" / "hexis"
        self.assertTrue(os.access(prog, os.X_OK))
        desktop = (self.home / ".local/share/applications/hexis.desktop").read_text()
        self.assertIn(f"Exec={prog}", desktop, "the menu entry must point at the installed program, not at PATH")
        self.assertTrue((self.home / ".local/share/icons/hicolor/128x128/apps/hexis.png").exists())
        manifest = (self.home / ".local/share/hexis/install-manifest").read_text()
        self.assertIn("method=tarball", manifest); self.assertIn("file=bin/hexis", manifest)
        vault = self.home / ".local/share/dev.habibullah.hexis/vault.db"
        vault.parent.mkdir(parents=True); vault.write_text("secret data")
        out = self.run_installer("--uninstall", expect=0).out
        self.assertIn("Hexis is removed", out)
        self.assertFalse(prog.exists())
        self.assertFalse((self.home / ".local/share/applications/hexis.desktop").exists())
        self.assertEqual(vault.read_text(), "secret data", "uninstall must never touch the vault")

    def test_appimage_install_without_fuse_is_user_scoped(self):
        # the fake "AppImage" is a script that can unpack itself, like the real runtime does
        script = b'#!/bin/sh\nif [ "$1" = "--appimage-extract" ]; then mkdir -p squashfs-root; printf "#!/bin/sh\\necho app\\n" > squashfs-root/AppRun; chmod +x squashfs-root/AppRun; echo png > squashfs-root/hexis.png; fi\n'
        files = dict(self.rel); name = "Hexis_0.1.0_amd64.AppImage"; files[name] = script
        files["SHA256SUMS"] = "".join(f"{sha(v)}  {k}\n" for k, v in sorted(files.items()) if k != "SHA256SUMS").encode()
        web = self.web({"/" + k: v for k, v in files.items()})
        self.run_installer("--method", "appimage", "--user", base=web.url, expect=0)
        launcher = (self.home / ".local/bin/hexis").read_text()
        self.assertIn("share/hexis/app/AppRun", launcher)
        self.assertTrue((self.home / ".local/share/hexis/app/AppRun").exists())
        self.run_installer("--uninstall", expect=0)
        self.assertFalse((self.home / ".local/share/hexis/app").exists())

    def test_a_wrong_checksum_installs_nothing(self):
        files = dict(self.web_files)
        files["/SHA256SUMS"] = files["/SHA256SUMS"].replace(b"  hexis-0.1.0-linux", b"  hexis-0.1.0-linux")  # keep names
        good = sha(self.rel["hexis-0.1.0-linux-x86_64.tar.gz"]).encode()
        files["/SHA256SUMS"] = files["/SHA256SUMS"].replace(good, b"0" * 64)
        web = self.web(files)
        r = self.run_installer("--method", "tarball", "--user", base=web.url, expect=1)
        self.assertIn("CHECKSUM MISMATCH", r.out)
        self.assertFalse((self.home / ".local").exists(), "nothing may be installed after a mismatch")

    def test_apk_download_is_verified_and_lands_in_the_current_folder(self):
        web = self.web(self.web_files)
        work = self.tmp / "work"; work.mkdir()
        e = {"PATH": str(self.bin), "HOME": str(self.home), "TMPDIR": str(self.tmp / "tmp"), "STUB_LOG": str(self.log),
             "HEXIS_REPO": "owner/project", "HEXIS_SITE": "http://127.0.0.1:9", "HEXIS_ROOTFS": str(self.root), "HEXIS_BASE_URL": web.url}
        r = subprocess.run(["/bin/sh", str(INSTALLER), "--apk"], env=e, cwd=work, capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual((work / "hexis-0.1.0-android-arm64-testkey.apk").read_bytes(), b"apk bytes")
        self.assertIn("-testkey", r.stdout)

    def test_missing_stable_release_falls_back_to_nightly_with_a_warning(self):
        # GitHub answers 404 for /releases/latest/download/... when only a pre-release exists
        nightly = {f"/owner/project/releases/download/nightly{k}": v for k, v in self.web_files.items()}
        web = self.web(nightly)
        r = self.run_installer("--method", "tarball", "--user", env={"HEXIS_GITHUB": web.url}, expect=0)
        self.assertIn("No stable release is published yet", r.out)
        self.assertTrue((self.home / ".local/bin/hexis").exists())

    def test_stable_channel_reads_the_latest_release_folder(self):
        latest = {f"/owner/project/releases/latest/download{k}": v for k, v in self.web_files.items()}
        web = self.web(latest)
        r = self.run_installer("--method", "tarball", "--user", env={"HEXIS_GITHUB": web.url}, expect=0)
        self.assertNotIn("No stable release", r.out)

    def test_prefix_option_installs_under_that_folder_without_root(self):
        web = self.web(self.web_files)
        prefix = self.tmp / "opt-hexis"
        self.run_installer("--method", "tarball", "--prefix", str(prefix), base=web.url, expect=0)
        self.assertTrue((prefix / "bin" / "hexis").exists())

    def test_unwritable_prefix_without_a_root_helper_is_a_clear_error(self):
        web = self.web(self.web_files)
        r = self.run_installer("--method", "tarball", "--prefix", "/proc/hexis-no", base=web.url, expect=1)
        self.assertIn("Cannot write to", r.out)

    def test_nothing_to_uninstall_is_not_an_error(self):
        self.assertIn("Nothing installed", self.run_installer("--uninstall", expect=0).out)


class CheckAndUpdate(InstallerCase):
    """--check says what is installed and what is newest; --update brings it up to date the way it was installed."""

    def setUp(self):
        super().setUp()
        self.system("void", tools=[])
        self.old = {"/" + k: v for k, v in make_release("0.1.0").items()}
        self.new = {"/" + k: v for k, v in make_release("0.2.0").items()}

    def install_old_user_tarball(self):
        web = self.web(self.old)
        self.run_installer("--method", "tarball", "--user", base=web.url, expect=0)
        self.assertTrue((self.home / ".local/bin/hexis").exists())

    # ---- --check
    def test_check_with_nothing_installed_says_how_to_install(self):
        r = self.run_installer("--check", expect=0)
        self.assertIn("not installed by this installer", r.out)
        self.assertIn("install.sh | sh", r.out)

    def test_check_reports_an_available_update_and_changes_nothing(self):
        self.install_old_user_tarball()
        before = (self.home / ".local/share/hexis/install-manifest").read_text()
        web = self.web(self.new)
        r = self.run_installer("--check", base=web.url, expect=0)
        self.assertIn("Newest stable release: 0.2.0", r.out)
        self.assertIn("Installed: 0.1.0", r.out)
        self.assertIn("An update is available", r.out)
        self.assertIn("--update", r.out)
        self.assertEqual((self.home / ".local/share/hexis/install-manifest").read_text(), before)

    def test_check_says_up_to_date_when_the_versions_match(self):
        self.install_old_user_tarball()
        web = self.web(self.old)
        r = self.run_installer("--check", base=web.url, expect=0)
        self.assertIn("Up to date", r.out)
        self.assertNotIn("An update is available", r.out)

    def test_check_notices_an_install_that_is_newer_than_the_release(self):
        self.install_old_user_tarball()
        web = self.web({"/" + k: v for k, v in make_release("0.0.9").items()})
        r = self.run_installer("--check", base=web.url, expect=0)
        self.assertIn("newer than the newest published", r.out)

    def test_nightly_builds_are_told_apart_by_their_checksum(self):
        # the nightly keeps one version number; a rebuilt tarball has another checksum
        web = self.web(self.old)
        self.run_installer("--nightly", "--method", "tarball", "--user", base=web.url, expect=0)
        manifest = (self.home / ".local/share/hexis/install-manifest").read_text()
        self.assertIn("sha256=" + sha(self.old["/hexis-0.1.0-linux-x86_64.tar.gz"]), manifest)
        same = self.web(self.old)
        self.assertIn("Up to date", self.run_installer("--check", "--nightly", base=same.url, expect=0).out)
        rebuilt = {k: v for k, v in self.old.items() if k != "/SHA256SUMS"}
        name = "/hexis-0.1.0-linux-x86_64.tar.gz"
        rebuilt[name] = rebuilt[name] + b"\0"  # different bytes, same version number
        rebuilt["/SHA256SUMS"] = "".join(f"{sha(v)}  {k[1:]}\n" for k, v in sorted(rebuilt.items())).encode()
        other = self.web(rebuilt)
        self.assertIn("An update is available", self.run_installer("--check", "--nightly", base=other.url, expect=0).out)

    # ---- --update
    def test_update_replaces_the_files_and_keeps_the_vault(self):
        self.install_old_user_tarball()
        vault = self.home / ".local/share/dev.habibullah.hexis/vault.db"
        vault.parent.mkdir(parents=True); vault.write_text("secret data")
        web = self.web(self.new)
        r = self.run_installer("--update", base=web.url, expect=0)
        self.assertIn("Updated Hexis 0.1.0 -> 0.2.0", r.out)
        manifest = (self.home / ".local/share/hexis/install-manifest").read_text()
        self.assertIn("version=0.2.0", manifest)
        self.assertEqual(vault.read_text(), "secret data")
        self.assertTrue(os.access(self.home / ".local/bin/hexis", os.X_OK))

    def test_update_when_already_current_does_nothing(self):
        self.install_old_user_tarball()
        stamp = (self.home / ".local/bin/hexis").stat().st_mtime_ns
        web = self.web(self.old)
        r = self.run_installer("--update", base=web.url, expect=0)
        self.assertIn("Already up to date (0.1.0)", r.out)
        self.assertEqual((self.home / ".local/bin/hexis").stat().st_mtime_ns, stamp)

    def test_update_removes_files_the_new_version_no_longer_has(self):
        self.install_old_user_tarball()
        leftover = self.home / ".local/share/hexis/old-helper"
        leftover.write_text("from the old version")
        manifest = self.home / ".local/share/hexis/install-manifest"
        manifest.write_text(manifest.read_text() + "file=share/hexis/old-helper\n")
        web = self.web(self.new)
        self.run_installer("--update", base=web.url, expect=0)
        self.assertFalse(leftover.exists(), "a file only the old manifest listed must be removed")

    def test_update_keeps_a_failed_download_from_damaging_the_install(self):
        self.install_old_user_tarball()
        files = dict(self.new)
        good = sha(self.new["/hexis-0.2.0-linux-x86_64.tar.gz"]).encode()
        files["/SHA256SUMS"] = files["/SHA256SUMS"].replace(good, b"0" * 64)
        web = self.web(files)
        r = self.run_installer("--update", base=web.url, expect=1)
        self.assertIn("CHECKSUM MISMATCH", r.out)
        self.assertIn("version=0.1.0", (self.home / ".local/share/hexis/install-manifest").read_text())
        self.assertTrue((self.home / ".local/bin/hexis").exists())

    def test_update_with_nothing_installed_is_a_clear_error(self):
        r = self.run_installer("--update", expect=1)
        self.assertIn("nothing to update", r.out)
        self.assertIn("install.sh | sh", r.out)

    def test_update_command_next_to_the_program_runs_the_site_installer_with_update(self):
        self.install_old_user_tarball()
        wrapper = (self.home / ".local/bin/hexis-update").read_text()
        self.assertIn("/install.sh | sh -s -- --update", wrapper)
        self.assertIn("file=bin/hexis-update", (self.home / ".local/share/hexis/install-manifest").read_text())
        self.run_installer("--uninstall", expect=0)
        self.assertFalse((self.home / ".local/bin/hexis-update").exists())

    def test_the_actions_exclude_each_other(self):
        self.assertIn("Choose one of", self.run_installer("--update", "--uninstall", expect=1).out)
        self.assertIn("Choose one of", self.run_installer("--check", "--update", expect=1).out)

    def test_android_is_pointed_at_the_apk(self):
        r = self.run_installer("--update", "--apk", expect=1)
        self.assertIn("--apk", r.out)

    # ---- repository installs go through the package manager
    def installed_dpkg(self, version="0.1.0"):
        self.stub("dpkg", f'#!/bin/sh\necho "dpkg $*" >> "$STUB_LOG"\n[ "$1" = "-s" ] && {{ echo "Status: install ok installed"; exit 0; }}\nexit 0\n')
        self.stub("dpkg-query", f'#!/bin/sh\necho "dpkg-query $*" >> "$STUB_LOG"\nprintf "{version}-1"\n')
        self.stub("apt-get")

    def test_update_of_a_repository_install_asks_apt_to_upgrade_only_hexis(self):
        self.system("ubuntu", tools=["apt-get"], uid=0)
        self.installed_dpkg()
        sources = self.root / "etc/apt/sources.list.d"; sources.mkdir(parents=True)
        (sources / "hexis.sources").write_text("Types: deb\n")
        site = self.web(repo_site())
        r = self.run_installer("--update", "--dry-run", site=site.url, expect=0)  # dry run: the commands are printed, not run
        self.assertIn("+ apt-get update", r.out)
        self.assertIn("+ apt-get install -y --only-upgrade hexis", r.out)
        self.assertNotIn("SHA256SUMS", r.out)  # the repository path never reads the release list

    def test_check_of_a_repository_install_offers_the_package_manager_command(self):
        self.system("ubuntu", tools=["apt-get"], uid=0)
        self.installed_dpkg()
        sources = self.root / "etc/apt/sources.list.d"; sources.mkdir(parents=True)
        (sources / "hexis.sources").write_text("Types: deb\n")
        web = self.web(self.new)
        r = self.run_installer("--check", base=web.url, expect=0)
        self.assertIn("An update is available", r.out)
        self.assertIn("apt-get install --only-upgrade hexis", r.out)

    def test_update_of_a_downloaded_deb_installs_the_new_deb(self):
        self.system("ubuntu", tools=["apt-get"], uid=0)
        self.installed_dpkg()
        web = self.web(self.new)
        r = self.run_installer("--update", "--dry-run", base=web.url, expect=0)
        self.assertRegex(r.out, r"\+ apt-get install -y \S*Hexis_0\.2\.0_amd64\.deb")

    def test_update_of_a_removed_package_with_leftover_settings_is_not_an_install(self):
        # dpkg -s still answers for a package that was removed but kept its configuration files
        self.system("ubuntu", tools=["apt-get"], uid=0)
        self.stub("dpkg", '#!/bin/sh\n[ "$1" = "-s" ] && { echo "Status: deinstall ok config-files"; exit 0; }\nexit 0\n')
        self.assertIn("nothing to update", self.run_installer("--update", expect=1).out)


if __name__ == "__main__":
    unittest.main()
