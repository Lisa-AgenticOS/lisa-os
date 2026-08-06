//! `install_plan` — deciding which disk `lisa install` is allowed to erase.
//!
//! `lisa install <disk>` is a byte-copy of the release image over a whole
//! block device (PLAN §6, the proto-installer; the guided OOBE is M7). It
//! is the only verb in this repo whose bug class is *somebody else's data
//! is gone*, and until now its entire protection was one line:
//!
//! ```text
//! mounts.lines().any(|l| l.split_whitespace().next()
//!     .is_some_and(|d| d.starts_with(target_str.as_ref())))
//! ```
//!
//! A prefix test over `/proc/mounts` is wrong in both directions, and the
//! direction that matters is the permissive one:
//!
//! - **It cannot see the disk it booted from unless something from that
//!   disk is mounted right now.** systemd-stub records the ESP the loader
//!   ran from in `LoaderDevicePartUUID`; nothing read it. A live session
//!   whose `/` is not a plain block device — an initrd that never
//!   switch-rooted, a rescue shell, anything on overlay — sails straight
//!   past and erases its own stick mid-write.
//! - **`starts_with` is not "is a partition of".** eMMC is the everyday
//!   counterexample: `/dev/mmcblk0boot0` is a *separate block device*, not
//!   a partition of `/dev/mmcblk0`. Mounting `/dev/mmcblk0boot0` made the
//!   old check refuse the perfectly-installable `/dev/mmcblk0`, and — the
//!   expensive half — targeting `/dev/mmcblk0boot0` while the running
//!   system sat on `/dev/mmcblk0p2` was *allowed*, because `p2` does not
//!   start with `boot0`.
//! - **It never asked whether the target was a whole disk.**
//!   `lisa install /dev/nvme0n1p3` writes a GPT-carrying disk image into
//!   the middle of somebody's third partition. Nothing objected.
//! - **It never asked whether the image fits.** The A/B layout has a hard
//!   floor (`MIN_TARGET_BYTES` below). Below it, `io::copy` runs until
//!   ENOSPC — having already overwritten every byte of the old disk.
//!
//! What replaced it was wrong twice more, in ways worth keeping written
//! down, because both were *the right check evaluated over the wrong
//! set* rather than a missing check (issue #290):
//!
//! - **A disk was its depth-1 `part` children and nothing else.** Every
//!   `crypt`, `lvm`, `raid*` and `dm-*` node was dropped at parse time,
//!   so a `/` mounted from `/dev/mapper/vg-root` or `/dev/md0` belonged
//!   to no disk on the machine. LUKS, LVM and md are not exotic; they
//!   are the default of every distro installer that offers encryption.
//!   Ownership is now membership in the disk's whole subtree, and a
//!   filesystem that spans devices without nesting — multi-device btrfs
//!   — is related by the UUID its members share.
//! - **The fail-closed gate was global; the refusal was per-disk.** The
//!   gate asked "did *any* signal resolve *any* disk", and one EFI
//!   `Loader` match on whichever disk carried the ESP answered yes —
//!   for every other disk on the machine. But the loader variable names
//!   an ESP, not a root: it says which disk to refuse, never which disk
//!   is safe. Only resolving `/` clears a disk that is not this one, so
//!   that is what the gate asks now.
//!
//! The two compounded into the worst output this file can produce: the
//! running system's own disk marked `[installable]`, captioned
//! "(this system is running from /dev/sda, not this disk)", above a
//! prompt that takes the word ERASE.
//!
//! Everything here is a pure function over injected facts: a parsed
//! `lsblk -J` topology and a `SystemFacts` (mounts + the EFI loader
//! partition). That is deliberate. A disk-selection guard that can only be
//! exercised by attaching a disk is a guard nobody tests, and the tests at
//! the bottom of this file are the only place the wrong-disk logic is ever
//! going to be made to go red on purpose.
//!
//! The impure half — `read_topology`, `read_facts` — is three functions
//! long and holds no decisions.

use std::fmt;

/// The smallest disk the image can be written to, from the partition
/// definitions in `os/mkosi/mkosi.repart/`:
///
/// | file | size |
/// |---|---|
/// | `00-esp.conf` | 1 GiB |
/// | `10-root.conf` (slot A) | 10 GiB |
/// | `11-root-b.conf` (slot B) | 10 GiB |
/// | `20-var.conf` | 2 GiB minimum |
///
/// 23 GiB. This is a floor, not a recommendation — `/var` grows into
/// whatever is left at first boot (`repart.d/50-var.conf`), so a disk at
/// exactly the floor boots with a model store of nothing. The release
/// notes ask for a 32 GB stick for that reason.
///
/// Change this only alongside those files; `check-repart-slots.py`
/// already guards the two root slots against each other.
pub const MIN_TARGET_BYTES: u64 = 23 * (1 << 30);

/// The EFI variable systemd-stub sets to the GPT UUID of the partition it
/// was loaded from — i.e. the ESP of the disk the machine booted. The
/// vendor GUID is systemd's own (`SD_LOADER_GUID`); the same path the
/// `lisa-loader-disk` udev helper reads, for the same reason (issue #16).
pub const LOADER_DEVICE_PART_UUID: &str =
    "/sys/firmware/efi/efivars/LoaderDevicePartUUID-4a67b082-0a4c-41cf-b6c7-440b29bb8c4f";

// ---------------------------------------------------------------------
// Topology
// ---------------------------------------------------------------------

/// One partition of a `Disk`. `partlabel` is what the fstab and
/// `root=PARTLABEL=` mount by (ADR-0018); `partuuid` is what the EFI
/// loader variable names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    pub path: String,
    pub size: u64,
    pub partlabel: Option<String>,
    pub partuuid: Option<String>,
}

/// Any node of the lsblk tree below (and including) a disk: the disk, a
/// partition, and — the point of this type — every `crypt`, `lvm`,
/// `raid*` and `dm-*` device stacked on top of one.
///
/// Issue #290 is what happens without it. `Disk` used to hold only its
/// depth-1 `part` children, so a `/` mounted from `/dev/mapper/vg-root`
/// or `/dev/md0` belonged to no disk at all, and the disk it was
/// physically on was offered as an install target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub path: String,
    /// lsblk's `TYPE`: `disk`, `part`, `crypt`, `lvm`, `raid1`, …
    pub kind: String,
    /// lsblk's `FSTYPE` — what is *on* the node, when anything is.
    pub fstype: Option<String>,
    /// lsblk's `UUID`: the filesystem's own UUID, which for btrfs is
    /// shared by every device of the pool. That sharing is the only
    /// thing that relates the members of a multi-device filesystem;
    /// lsblk does not nest them.
    pub fs_uuid: Option<String>,
}

/// Filesystems whose single UUID legitimately appears on several
/// devices at once, because the filesystem spans them. Seeing one of
/// these UUIDs on a second disk means that disk holds part of the same
/// filesystem — not that somebody cloned a device.
///
/// ZFS is deliberately absent: its root source (`rpool/ROOT/…`) resolves
/// to no block device at all, so the planner never learns where `/` is
/// and fails closed on every disk, which is the correct answer.
const MULTI_DEVICE_FSTYPES: &[&str] = &["btrfs", "bcachefs"];

/// A whole block device that could plausibly be an install target: a
/// `disk`, or a `loop` (so the whole path is exercisable against a
/// loopback file, which is the only way it gets tested off hardware).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disk {
    pub path: String,
    /// lsblk's `TYPE`: `disk` or `loop`.
    pub kind: String,
    pub size: u64,
    pub removable: bool,
    pub read_only: bool,
    pub model: Option<String>,
    /// The depth-1 `part` children, in lsblk order. This is what the
    /// erase preview itemises and what the EFI loader variable names —
    /// a PARTUUID is a GPT property, so only a partition has one.
    pub partitions: Vec<Partition>,
    /// The disk itself followed by every descendant, of any type and at
    /// any depth. `owns()` is membership in this list and nothing else.
    pub devices: Vec<Node>,
}

impl Disk {
    /// Is `dev` this disk, or anything stacked on it?
    ///
    /// Set membership over the topology, never a string prefix and never
    /// string equality against a partition list. Both of those have
    /// already cost this file a defect:
    ///
    /// - `starts_with` is not "is a partition of" — `/dev/mmcblk0boot0`
    ///   breaks it in both directions, and the expensive direction is
    ///   the permissive one (module docs).
    /// - equality against `partitions` alone cannot see `/` when `/` is
    ///   `/dev/mapper/vg-root` or `/dev/md0`, which is #290 — the whole
    ///   reason a disk carries its full subtree now.
    pub fn owns(&self, dev: &str) -> bool {
        self.devices.iter().any(|n| n.path == dev)
    }

    /// The filesystem UUIDs present anywhere on this disk, paired with
    /// the device carrying them.
    fn filesystems(&self) -> impl Iterator<Item = (&str, &str, &str)> {
        self.devices
            .iter()
            .filter_map(|n| Some((n.path.as_str(), n.fstype.as_deref()?, n.fs_uuid.as_deref()?)))
    }

    /// A one-line description for the picker: size, kind, model.
    pub fn describe(&self) -> String {
        let model = self.model.as_deref().unwrap_or("unknown");
        let kind = if self.removable {
            "removable"
        } else {
            "internal"
        };
        format!("{} — {} {kind}, {model}", self.path, human(self.size))
    }
}

/// The lsblk columns the planner reads. Kept next to the parser so the
/// two cannot drift: a column dropped from the command is a signal that
/// silently reads `None` for every device on the machine.
pub const LSBLK_COLUMNS: &str = "PATH,TYPE,SIZE,RM,RO,MODEL,PARTLABEL,PARTUUID,FSTYPE,UUID";

/// Parse `lsblk -J -b -o` [`LSBLK_COLUMNS`].
///
/// Tolerant on purpose about the two shapes util-linux has shipped: `rm`
/// and `ro` are JSON booleans in modern lsblk and the strings `"0"`/`"1"`
/// in older ones, and `size` is a number under `-b` but a string in some
/// builds. Getting either wrong silently is how a read-only disk becomes
/// writable in the planner's eyes.
///
/// The `children` array is walked to the bottom, keeping nodes of every
/// type. Only the depth-1 `part` children become `partitions`; all of
/// them, at every depth, become `devices`.
pub fn parse_lsblk(json: &str) -> anyhow::Result<Vec<Disk>> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let devices = v["blockdevices"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("lsblk output has no blockdevices array"))?;
    let mut out = Vec::new();
    for d in devices {
        let kind = str_field(d, "type").unwrap_or_default();
        if kind != "disk" && kind != "loop" {
            continue;
        }
        let Some(path) = str_field(d, "path") else {
            continue;
        };
        let partitions = d["children"]
            .as_array()
            .map(|c| {
                c.iter()
                    .filter(|p| str_field(p, "type").as_deref() == Some("part"))
                    .filter_map(|p| {
                        Some(Partition {
                            path: str_field(p, "path")?,
                            size: num_field(p, "size").unwrap_or(0),
                            partlabel: str_field(p, "partlabel"),
                            partuuid: str_field(p, "partuuid").map(|s| s.to_lowercase()),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut nodes = Vec::new();
        flatten_devices(d, &mut nodes);
        out.push(Disk {
            path,
            kind,
            size: num_field(d, "size").unwrap_or(0),
            removable: bool_field(d, "rm"),
            read_only: bool_field(d, "ro"),
            model: str_field(d, "model").filter(|s| !s.is_empty()),
            partitions,
            devices: nodes,
        });
    }
    Ok(out)
}

/// Depth-first flatten of one lsblk node and everything under it.
///
/// A node with no `path` is skipped but *not* pruned — its children are
/// still walked, because losing a subtree here means losing the evidence
/// that a disk is in use, and that failure is silent.
fn flatten_devices(v: &serde_json::Value, out: &mut Vec<Node>) {
    if let Some(path) = str_field(v, "path") {
        out.push(Node {
            path,
            kind: str_field(v, "type").unwrap_or_default(),
            fstype: str_field(v, "fstype")
                .filter(|s| !s.is_empty())
                .map(|s| s.to_lowercase()),
            fs_uuid: str_field(v, "uuid")
                .filter(|s| !s.is_empty())
                .map(|s| s.to_lowercase()),
        });
    }
    if let Some(children) = v["children"].as_array() {
        for c in children {
            flatten_devices(c, out);
        }
    }
}

fn str_field(v: &serde_json::Value, key: &str) -> Option<String> {
    v[key].as_str().map(|s| s.trim().to_string())
}

fn num_field(v: &serde_json::Value, key: &str) -> Option<u64> {
    match &v[key] {
        serde_json::Value::Number(n) => n.as_u64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// `true` only for the affirmative spellings. Anything unrecognised —
/// including a missing key — reads as `false`, which for `ro` means "we
/// will try to write and the kernel will refuse", never "we assumed it
/// was fine".
fn bool_field(v: &serde_json::Value, key: &str) -> bool {
    match &v[key] {
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::String(s) => s == "1" || s == "true",
        serde_json::Value::Number(n) => n.as_u64() == Some(1),
        _ => false,
    }
}

// ---------------------------------------------------------------------
// Where we are running from
// ---------------------------------------------------------------------

/// One line of `/proc/mounts`, reduced to what the planner needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub source: String,
    pub target: String,
}

/// Everything the planner knows about the running system. Injected, so
/// every branch below is reachable from a test.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SystemFacts {
    pub mounts: Vec<Mount>,
    /// The GPT UUID from `LoaderDevicePartUUID`, lowercased.
    pub loader_partuuid: Option<String>,
}

/// Parse `/proc/mounts`. Fields are octal-escaped by the kernel
/// (`\040` = space, `\011` = tab, `\012` = newline, `\134` = backslash);
/// a mount point under `/run/media/lisa/My Disk` is the everyday case and
/// an unescaped comparison would simply miss it.
pub fn parse_mounts(text: &str) -> Vec<Mount> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let source = unescape_octal(f.next()?);
            let target = unescape_octal(f.next()?);
            // findmnt-style `/dev/vda2[/subvol]` never appears in
            // /proc/mounts, but the same string reaches this function
            // from `findmnt -o SOURCE` in a shell pipeline, and a
            // source that keeps its subvolume suffix matches no
            // partition path.
            let source = source.split('[').next().unwrap_or(&source).to_string();
            Some(Mount { source, target })
        })
        .collect()
}

/// Undo the kernel's octal escaping, in **bytes**.
///
/// It decodes to a `Vec<u8>` and then to UTF-8 for one reason: an escape
/// is a byte, and a non-ASCII character in a mount source or mount point
/// is two or three of them. Pushing each byte as a `char` widens it to a
/// codepoint — Latin-1 — so `/dev/mapper/caféx` came back out as
/// `/dev/mapper/cafÃ©x` and compared equal to nothing lsblk had ever
/// said. Same for `\303\251`, which *is* how the kernel escapes a
/// non-ASCII byte that also happens to need escaping.
///
/// The digit test is on bytes as well. The previous `&s[i+1..i+4]` slice
/// panicked outright when a literal backslash was followed by a
/// multi-byte character, because that index is not a char boundary.
fn unescape_octal(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            let d = &bytes[i + 1..i + 4];
            if d.iter().all(|b| b.is_ascii_digit() && *b < b'8') {
                let code = (u32::from(d[0] - b'0') << 6)
                    | (u32::from(d[1] - b'0') << 3)
                    | u32::from(d[2] - b'0');
                // \400..\777 are not a byte; leave them literal rather
                // than truncate into a different character.
                if let Ok(byte) = u8::try_from(code) {
                    out.push(byte);
                    i += 4;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Why we believe a disk is the one this system is running from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evidence {
    /// `/` is mounted from this disk — from the disk itself, from one of
    /// its partitions, or from anything stacked on those: a dm-crypt
    /// container, an LV, an md array.
    RootMount { source: String },
    /// This disk carries a device of the *same filesystem* `/` is
    /// mounted from. A btrfs pool spanning two disks names only one of
    /// them in the mount table; erasing the other one ends the
    /// filesystem just as completely.
    RootFilesystemMember {
        device: String,
        fstype: String,
        fs_uuid: String,
    },
    /// systemd-stub says the boot loader was loaded from this partition.
    Loader { partition: String },
}

impl fmt::Display for Evidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Evidence::RootMount { source } => write!(f, "/ is mounted from {source}"),
            Evidence::RootFilesystemMember {
                device,
                fstype,
                fs_uuid,
            } => write!(
                f,
                "{device} is another device of the {fstype} filesystem / is mounted from \
                 (UUID {fs_uuid})"
            ),
            Evidence::Loader { partition } => {
                write!(
                    f,
                    "the boot loader was loaded from {partition} (EFI LoaderDevicePartUUID)"
                )
            }
        }
    }
}

/// Evidence that the running **root filesystem** lives on `disk`.
///
/// This is the signal that matters most, and the one #290 could not see:
/// it resolves `/` through the whole device stack rather than against a
/// list of partition paths. It is kept separate from `boot_evidence`
/// because it is the only signal that says anything about a *different*
/// disk — see `plan()`.
pub fn root_evidence(disk: &Disk, disks: &[Disk], facts: &SystemFacts) -> Vec<Evidence> {
    let mut out = Vec::new();
    for m in &facts.mounts {
        if m.target == "/" && disk.owns(&m.source) {
            out.push(Evidence::RootMount {
                source: m.source.clone(),
            });
        }
    }
    for (fstype, fs_uuid) in root_filesystems(disks, facts) {
        for (path, t, u) in disk.filesystems() {
            if t != fstype || u != fs_uuid {
                continue;
            }
            // The device `/` was mounted from is already covered by the
            // sharper sentence above.
            if out
                .iter()
                .any(|e| matches!(e, Evidence::RootMount { source } if source == path))
            {
                continue;
            }
            out.push(Evidence::RootFilesystemMember {
                device: path.to_string(),
                fstype: fstype.to_string(),
                fs_uuid: fs_uuid.to_string(),
            });
        }
    }
    out
}

/// The `(fstype, uuid)` of the filesystem `/` is mounted from, when it is
/// one that can span devices. Restricted to `MULTI_DEVICE_FSTYPES` on
/// purpose: a second device carrying the same *ext4* UUID is a clone, not
/// a member, and refusing a disk because somebody once `dd`'d it is a
/// different bug from the one this closes.
fn root_filesystems<'a>(disks: &'a [Disk], facts: &SystemFacts) -> Vec<(&'a str, &'a str)> {
    let mut out: Vec<(&str, &str)> = Vec::new();
    for m in facts.mounts.iter().filter(|m| m.target == "/") {
        for d in disks {
            for (path, fstype, uuid) in d.filesystems() {
                if path == m.source
                    && MULTI_DEVICE_FSTYPES.contains(&fstype)
                    && !out.contains(&(fstype, uuid))
                {
                    out.push((fstype, uuid));
                }
            }
        }
    }
    out
}

/// Does the EFI loader variable identify *this* disk?
///
/// A PARTUUID is supposed to be unique, and after `lisa install` it is
/// not: the byte copy duplicates the image's ESP PARTUUID onto the
/// target, so re-flashing a machine from the same stick sees the
/// variable match a partition on two disks at once (issue #301). When
/// that happens the variable identifies neither on its own. The ESP the
/// firmware actually used is the one on the disk the running root is on;
/// if exactly one claimant holds the root, the others are copies. If we
/// cannot tell, every claimant keeps the evidence and every claimant is
/// refused.
fn loader_identifies(disk: &Disk, disks: &[Disk], facts: &SystemFacts, uuid: &str) -> bool {
    let claims = |d: &Disk| {
        d.partitions
            .iter()
            .any(|p| p.partuuid.as_deref() == Some(uuid))
    };
    if !claims(disk) {
        return false;
    }
    let claimants: Vec<&Disk> = disks.iter().filter(|d| claims(d)).collect();
    if claimants.len() < 2 {
        return true;
    }
    let rooted: Vec<&&Disk> = claimants
        .iter()
        .filter(|d| !root_evidence(d, disks, facts).is_empty())
        .collect();
    match rooted.as_slice() {
        [only] => only.path == disk.path,
        _ => true,
    }
}

/// Evidence that `disk` is the booted disk. Independent signals, because
/// each one alone has a blind spot: `/` tells us nothing when the running
/// root is not a block device (initrd, rescue, overlay), and the EFI
/// variable is absent on a direct-kernel boot (CI) and on non-EFI
/// machines.
pub fn boot_evidence(disk: &Disk, disks: &[Disk], facts: &SystemFacts) -> Vec<Evidence> {
    let mut out = root_evidence(disk, disks, facts);
    if let Some(uuid) = &facts.loader_partuuid
        && loader_identifies(disk, disks, facts, uuid)
    {
        for p in &disk.partitions {
            if p.partuuid.as_deref() == Some(uuid.as_str()) {
                out.push(Evidence::Loader {
                    partition: p.path.clone(),
                });
            }
        }
    }
    out
}

/// Every disk the running root filesystem lives on — more than one when
/// it is an md mirror, a striped LV or a multi-device btrfs.
pub fn root_disks<'a>(disks: &'a [Disk], facts: &SystemFacts) -> Vec<&'a Disk> {
    disks
        .iter()
        .filter(|d| !root_evidence(d, disks, facts).is_empty())
        .collect()
}

/// The disks this system is running from. The root disks when we know
/// them; otherwise whatever the remaining signals resolve, which in
/// practice means the disk holding the ESP the firmware loaded.
///
/// The distinction is load-bearing and is the second half of #290: the
/// ESP disk is a fine answer to "which disk is refused", and no answer at
/// all to "is that other disk safe to erase".
pub fn boot_disks<'a>(disks: &'a [Disk], facts: &SystemFacts) -> Vec<&'a Disk> {
    let roots = root_disks(disks, facts);
    if !roots.is_empty() {
        return roots;
    }
    disks
        .iter()
        .filter(|d| !boot_evidence(d, disks, facts).is_empty())
        .collect()
}

/// The disk this system booted from, if any signal resolves it.
pub fn boot_disk<'a>(disks: &'a [Disk], facts: &SystemFacts) -> Option<&'a Disk> {
    boot_disks(disks, facts).into_iter().next()
}

/// Are we running from removable media — i.e. is this a live session
/// rather than an installed one?
///
/// Not a safety check and never used as one: it decides whether
/// `lisa install --list` says "you are running from a stick" or "you are
/// running from the disk you are about to be told you cannot erase".
/// `None` means we could not tell.
pub fn is_live_session(disks: &[Disk], facts: &SystemFacts) -> Option<bool> {
    boot_disk(disks, facts).map(|d| d.removable)
}

/// Every mount whose source lives on `disk`. A disk with a live mount is
/// not a legal target regardless of whose it is: writing under a mounted
/// filesystem corrupts it and the page cache will happily write the stale
/// version back over the image afterwards.
pub fn mounts_on<'a>(disk: &Disk, facts: &'a SystemFacts) -> Vec<&'a Mount> {
    facts
        .mounts
        .iter()
        .filter(|m| disk.owns(&m.source))
        .collect()
}

// ---------------------------------------------------------------------
// The decision
// ---------------------------------------------------------------------

/// Why a target was refused. Every variant carries what a person needs in
/// order to do something different — a refusal that does not say which
/// disk to use instead is a refusal that gets worked around with `dd`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Not in the block-device topology at all.
    NoSuchDevice {
        target: String,
    },
    /// A partition, not a whole disk.
    IsAPartition {
        target: String,
        disk: String,
    },
    /// Nothing told us where the running system lives. Fail closed.
    BootDiskUnknown {
        target: String,
    },
    /// This is the disk we are running from.
    IsTheBootDisk {
        target: String,
        evidence: Vec<Evidence>,
    },
    ReadOnly {
        target: String,
    },
    TooSmall {
        target: String,
        have: u64,
        need: u64,
    },
    /// Something on the disk is mounted right now.
    InUse {
        target: String,
        mounts: Vec<Mount>,
    },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::NoSuchDevice { target } => write!(
                f,
                "{target} is not a block device this system can see — `lisa install --list` \
                 shows the disks that are"
            ),
            Refusal::IsAPartition { target, disk } => write!(
                f,
                "{target} is not a whole disk — it lives on {disk}. The image carries its own \
                 partition table, so it is written to a whole disk — you probably meant \
                 {disk}, which erases {target} along with everything else on it"
            ),
            Refusal::BootDiskUnknown { target } => write!(
                f,
                "cannot tell which disk the running root filesystem is on — / did not resolve \
                 to any disk in the block-device tree — so {target} cannot be ruled out as \
                 that disk. Refusing to write"
            ),
            Refusal::IsTheBootDisk { target, evidence } => {
                write!(f, "{target} is the disk this system is running from")?;
                for e in evidence {
                    write!(f, "\n   - {e}")?;
                }
                write!(
                    f,
                    "\n   Boot the Lisa USB stick and install to the internal disk from there."
                )
            }
            Refusal::ReadOnly { target } => {
                write!(
                    f,
                    "{target} is read-only (a write-protect switch, or a locked card)"
                )
            }
            Refusal::TooSmall { target, have, need } => write!(
                f,
                "{target} is {} — the A/B layout needs at least {}. A partial write destroys \
                 what is there now and boots nothing",
                human(*have),
                human(*need)
            ),
            Refusal::InUse { target, mounts } => {
                write!(f, "{target} has mounted filesystems:")?;
                for m in mounts {
                    write!(f, "\n   - {} at {}", m.source, m.target)?;
                }
                write!(
                    f,
                    "\n   Unmount them first (writing under a mount corrupts it)."
                )
            }
        }
    }
}

/// What writing the image to a target would destroy, if it is allowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub target: String,
    pub size: u64,
    pub model: Option<String>,
    pub removable: bool,
    /// One line per partition that exists on the target today. Shown
    /// before the typed confirmation so "erases the disk" is a list of
    /// real things rather than an abstraction.
    pub destroys: Vec<String>,
    /// Every disk the running root filesystem lives on — never the
    /// target, by construction, and never empty: `plan` refuses when it
    /// cannot resolve `/` at all.
    ///
    /// A `Vec` rather than one path because #290's caption
    /// ("this system is running from /dev/sda, not this disk") was an
    /// affirmative sentence printed above the erase prompt while being
    /// wrong. A mirror or a striped LV really is two disks; saying so is
    /// cheaper than being confidently incomplete.
    pub boot_disks: Vec<String>,
}

/// Decide whether the image may be written to `target`.
///
/// Order is chosen for the message, not for speed: the most specific,
/// most actionable refusal wins. `IsAPartition` before anything about
/// booting, because "you meant the disk" is the answer; `IsTheBootDisk`
/// before `InUse`, because "that is your own stick" is a sharper sentence
/// than "unmount /var".
pub fn plan(
    target: &str,
    disks: &[Disk],
    facts: &SystemFacts,
    min_bytes: u64,
) -> Result<Plan, Refusal> {
    let target = target.to_string();

    let Some(disk) = disks.iter().find(|d| d.path == target) else {
        // Something on a known disk gets the useful message; anything
        // else is simply not here.
        if let Some(owner) = disks.iter().find(|d| d.owns(&target)) {
            return Err(Refusal::IsAPartition {
                target,
                disk: owner.path.clone(),
            });
        }
        return Err(Refusal::NoSuchDevice { target });
    };

    // Per-disk first, and before the fail-closed gate, so the refusal
    // for a disk we *have* identified says which disk it is rather than
    // "cannot tell".
    let evidence = boot_evidence(disk, disks, facts);
    if !evidence.is_empty() {
        return Err(Refusal::IsTheBootDisk { target, evidence });
    }

    // Fail closed, and specifically on *the root*. This is the second
    // half of #290. The old gate accepted any boot-disk signal found on
    // any disk — so one EFI `Loader` match, on whichever disk happened
    // to carry the ESP, certified that we knew where we were standing
    // and unlocked every other disk on the machine. It does not: the
    // loader variable names an ESP, and `/` can be on a different disk
    // entirely. Only resolving `/` clears a disk that is not this one.
    let roots = root_disks(disks, facts);
    if roots.is_empty() {
        return Err(Refusal::BootDiskUnknown { target });
    }

    if disk.read_only {
        return Err(Refusal::ReadOnly { target });
    }

    if disk.size < min_bytes {
        return Err(Refusal::TooSmall {
            target,
            have: disk.size,
            need: min_bytes,
        });
    }

    let live = mounts_on(disk, facts);
    if !live.is_empty() {
        return Err(Refusal::InUse {
            target,
            mounts: live.into_iter().cloned().collect(),
        });
    }

    Ok(Plan {
        target,
        size: disk.size,
        model: disk.model.clone(),
        removable: disk.removable,
        destroys: disk
            .partitions
            .iter()
            .map(|p| {
                let label = p.partlabel.as_deref().unwrap_or("no label");
                format!("{} ({}, {label})", p.path, human(p.size))
            })
            .collect(),
        boot_disks: roots.iter().map(|d| d.path.clone()).collect(),
    })
}

/// Every whole disk on the machine with its verdict — what
/// `lisa install --list` prints. A refusal is as informative as an
/// allowance here: the person is looking for the disk they meant.
pub fn candidates(
    disks: &[Disk],
    facts: &SystemFacts,
    min_bytes: u64,
) -> Vec<(String, Result<Plan, Refusal>)> {
    disks
        .iter()
        .map(|d| (d.describe(), plan(&d.path, disks, facts, min_bytes)))
        .collect()
}

/// Bytes as GiB with one decimal — the unit every size in this file is
/// reasoned about in.
pub fn human(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / (1u64 << 30) as f64)
}

/// The whole text of `lisa install --list`, as a string.
///
/// A pure function rather than a pile of `println!` in `main.rs` for one
/// reason: the picker is the surface a person reads before typing a
/// device node, and "did it say the right disk is installable" is
/// exactly the assertion that must survive a refactor. `main.rs` prints
/// what this returns and decides nothing.
pub fn render_targets(disks: &[Disk], facts: &SystemFacts, min_bytes: u64) -> String {
    let mut out = String::new();
    let booted: Vec<&str> = boot_disks(disks, facts)
        .iter()
        .map(|d| d.path.as_str())
        .collect();
    // Every disk, not the first one: an md mirror or a striped LV is
    // genuinely two, and naming one of them reads as an all-clear for
    // the other (#290).
    let named = booted.join(", ");
    // Whether `/` was resolved decides what this header may promise. A
    // header that says "installing to another disk is what this verb is
    // for" above a list in which every disk is refused is the same kind
    // of confident-and-wrong sentence #290 was filed about.
    let rooted = !root_disks(disks, facts).is_empty();
    match (booted.is_empty(), rooted, is_live_session(disks, facts)) {
        (false, true, Some(true)) => out.push_str(&format!(
            ">> running from removable media ({named}) — a live session; installing to \
             another disk is what this verb is for\n"
        )),
        (false, true, _) => out.push_str(&format!(
            ">> running from {named} — an installed system, not a live session\n"
        )),
        (false, false, _) => out.push_str(&format!(
            "!! the boot loader ran from {named}, but / did not resolve to any disk — so no \
             other disk can be ruled out, and every disk below is refused\n"
        )),
        _ => out.push_str(
            "!! cannot tell which disk this system is running from, so every disk below \
             is refused\n",
        ),
    }
    out.push('\n');

    for (description, verdict) in candidates(disks, facts, min_bytes) {
        match verdict {
            Ok(plan) => {
                out.push_str(&format!("  [installable] {description}\n"));
                if plan.destroys.is_empty() {
                    out.push_str("                (no partitions — empty disk)\n");
                }
                for line in &plan.destroys {
                    out.push_str(&format!("                erases {line}\n"));
                }
            }
            Err(refusal) => {
                out.push_str(&format!("  [   refused ] {description}\n"));
                for line in refusal.to_string().lines() {
                    out.push_str(&format!("                {}\n", line.trim_start()));
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------
// Reading the real machine. No decisions live here.
// ---------------------------------------------------------------------

/// Ask lsblk for the topology. Linux only — there is no `lsblk` anywhere
/// else, and `lisa install` already refuses block devices off Linux.
pub fn read_topology() -> anyhow::Result<Vec<Disk>> {
    let out = std::process::Command::new("lsblk")
        .args(["-J", "-b", "-o", LSBLK_COLUMNS])
        .output()
        .map_err(|e| anyhow::anyhow!("running lsblk: {e}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "lsblk failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    parse_lsblk(&String::from_utf8_lossy(&out.stdout))
}

/// Read `/proc/mounts` and the EFI loader variable. A missing file is an
/// absent signal, never an assumption: `plan` fails closed when the
/// signals together resolve nothing.
pub fn read_facts() -> SystemFacts {
    SystemFacts {
        mounts: std::fs::read_to_string("/proc/mounts")
            .map(|s| parse_mounts(&s))
            .unwrap_or_default(),
        loader_partuuid: std::fs::read(LOADER_DEVICE_PART_UUID)
            .ok()
            .and_then(|b| decode_loader_partuuid(&b)),
    }
}

/// An efivarfs payload is 4 bytes of attributes followed by the value —
/// here a UTF-16LE GPT UUID string, NUL-terminated. Returned lowercase to
/// match `parse_lsblk`.
pub fn decode_loader_partuuid(raw: &[u8]) -> Option<String> {
    let body = raw.get(4..)?;
    let s: String = body
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .filter_map(|u| char::from_u32(u as u32))
        .collect();
    let s = s.trim().to_lowercase();
    if s.is_empty() { None } else { Some(s) }
}

// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// The machine `lisa install` exists for: a Lisa stick in the USB
    /// port, a Windows laptop's NVMe inside. Written the way lsblk really
    /// prints it, including the `rm`/`ro` booleans and a null model on
    /// the stick.
    const LIVE_SESSION: &str = r#"{
      "blockdevices": [
        {"path":"/dev/sda","type":"disk","size":64023257088,"rm":true,"ro":false,
         "model":"Cruzer Blade",
         "children":[
           {"path":"/dev/sda1","type":"part","size":1073741824,"partlabel":"esp",
            "partuuid":"11111111-1111-1111-1111-111111111111"},
           {"path":"/dev/sda2","type":"part","size":10737418240,"partlabel":"root_1",
            "partuuid":"22222222-2222-2222-2222-222222222222"},
           {"path":"/dev/sda3","type":"part","size":10737418240,"partlabel":"_empty",
            "partuuid":"33333333-3333-3333-3333-333333333333"},
           {"path":"/dev/sda4","type":"part","size":34359738368,"partlabel":"var",
            "partuuid":"44444444-4444-4444-4444-444444444444"}
         ]},
        {"path":"/dev/nvme0n1","type":"disk","size":512110190592,"rm":false,"ro":false,
         "model":"SAMSUNG MZVLB512HBJQ",
         "children":[
           {"path":"/dev/nvme0n1p1","type":"part","size":272629760,"partlabel":"EFI system partition",
            "partuuid":"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"},
           {"path":"/dev/nvme0n1p2","type":"part","size":16777216,"partlabel":"Microsoft reserved partition",
            "partuuid":"bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"},
           {"path":"/dev/nvme0n1p3","type":"part","size":511288541184,"partlabel":"Basic data partition",
            "partuuid":"cccccccc-cccc-cccc-cccc-cccccccccccc"}
         ]}
      ]}"#;

    fn live_disks() -> Vec<Disk> {
        parse_lsblk(LIVE_SESSION).expect("fixture parses")
    }

    /// Booted from the stick: `/` on sda2, the loader on sda1, /var and
    /// /efi mounted from the stick as the shipped fstab does.
    fn live_facts() -> SystemFacts {
        SystemFacts {
            mounts: parse_mounts(
                "/dev/sda2 / btrfs rw,relatime 0 0\n\
                 /dev/sda4 /var btrfs rw,nodev 0 0\n\
                 /dev/sda1 /efi vfat rw,umask=0077 0 0\n\
                 tmpfs /run tmpfs rw 0 0\n",
            ),
            loader_partuuid: Some("11111111-1111-1111-1111-111111111111".into()),
        }
    }

    // -- the happy path ------------------------------------------------

    #[test]
    fn installs_to_the_internal_disk_from_a_live_stick() {
        let p = plan(
            "/dev/nvme0n1",
            &live_disks(),
            &live_facts(),
            MIN_TARGET_BYTES,
        )
        .expect("the internal disk is a legal target");
        assert_eq!(p.target, "/dev/nvme0n1");
        assert_eq!(p.boot_disks, vec!["/dev/sda".to_string()]);
        assert!(!p.removable);
        // The confirmation prompt gets real things, not "the disk".
        assert_eq!(p.destroys.len(), 3);
        assert!(
            p.destroys[2].contains("Basic data partition"),
            "destroys = {:?}",
            p.destroys
        );
    }

    #[test]
    fn the_live_session_knows_it_is_live() {
        assert_eq!(is_live_session(&live_disks(), &live_facts()), Some(true));
        // Installed on the NVMe instead: same code, opposite answer.
        let facts = SystemFacts {
            mounts: parse_mounts("/dev/nvme0n1p3 / btrfs rw 0 0\n"),
            loader_partuuid: None,
        };
        assert_eq!(is_live_session(&live_disks(), &facts), Some(false));
        // Nothing resolves: we do not guess.
        assert_eq!(
            is_live_session(&live_disks(), &SystemFacts::default()),
            None
        );
    }

    // -- the refusals, one test each -----------------------------------

    #[test]
    fn refuses_the_disk_it_booted_from() {
        let err = plan("/dev/sda", &live_disks(), &live_facts(), MIN_TARGET_BYTES)
            .expect_err("erasing the stick you are running from is never allowed");
        let Refusal::IsTheBootDisk { evidence, .. } = &err else {
            panic!("wrong refusal: {err:?}");
        };
        // Both signals fired; either one alone must be enough.
        assert_eq!(evidence.len(), 2, "{evidence:?}");
    }

    /// The case the old `/proc/mounts` prefix test could not see at all:
    /// a session whose `/` is not a block device. Only the EFI loader
    /// variable knows, and it is enough on its own.
    #[test]
    fn refuses_the_boot_disk_when_nothing_from_it_is_mounted() {
        let facts = SystemFacts {
            mounts: parse_mounts("rootfs / rootfs rw 0 0\ntmpfs /run tmpfs rw 0 0\n"),
            loader_partuuid: Some("11111111-1111-1111-1111-111111111111".into()),
        };
        let err = plan("/dev/sda", &live_disks(), &facts, MIN_TARGET_BYTES)
            .expect_err("the loader variable alone identifies the boot disk");
        assert!(matches!(err, Refusal::IsTheBootDisk { .. }), "{err:?}");
    }

    /// And the mirror: no EFI at all (direct-kernel boot, CI, non-UEFI),
    /// `/` on the disk. The mount signal alone is enough.
    #[test]
    fn refuses_the_boot_disk_with_no_efi_variable() {
        let facts = SystemFacts {
            mounts: parse_mounts("/dev/sda2 / btrfs rw 0 0\n"),
            loader_partuuid: None,
        };
        let err = plan("/dev/sda", &live_disks(), &facts, MIN_TARGET_BYTES)
            .expect_err("/ on the disk identifies the boot disk");
        assert!(matches!(err, Refusal::IsTheBootDisk { .. }), "{err:?}");
    }

    #[test]
    fn refuses_a_partition_and_names_the_disk_that_was_meant() {
        let err = plan(
            "/dev/nvme0n1p3",
            &live_disks(),
            &live_facts(),
            MIN_TARGET_BYTES,
        )
        .expect_err("a partition is never an install target");
        assert_eq!(
            err,
            Refusal::IsAPartition {
                target: "/dev/nvme0n1p3".into(),
                disk: "/dev/nvme0n1".into(),
            }
        );
    }

    /// eMMC: `/dev/mmcblk0boot0` and `/dev/mmcblk0` are separate block
    /// devices, and `"/dev/mmcblk0p2".starts_with("/dev/mmcblk0boot0")`
    /// is false — so a prefix test let you write the image over the
    /// running system's boot partition. Ownership comes from the
    /// topology, so both directions are right.
    #[test]
    fn a_sibling_device_with_a_shared_prefix_is_not_a_partition() {
        let json = r#"{"blockdevices":[
          {"path":"/dev/mmcblk0","type":"disk","size":64023257088,"rm":false,"ro":false,
           "children":[
             {"path":"/dev/mmcblk0p1","type":"part","size":1073741824,"partuuid":"dddddddd-0000-0000-0000-000000000001"},
             {"path":"/dev/mmcblk0p2","type":"part","size":62949672960,"partuuid":"dddddddd-0000-0000-0000-000000000002"}
           ]},
          {"path":"/dev/mmcblk0boot0","type":"disk","size":4194304,"rm":false,"ro":false},
          {"path":"/dev/sda","type":"disk","size":64023257088,"rm":true,"ro":false,
           "children":[{"path":"/dev/sda1","type":"part","size":1073741824,"partuuid":"eeeeeeee-0000-0000-0000-000000000001"}]}
        ]}"#;
        let disks = parse_lsblk(json).unwrap();
        let facts = SystemFacts {
            mounts: parse_mounts("/dev/mmcblk0p2 / ext4 rw 0 0\n"),
            loader_partuuid: None,
        };
        // The running system is on mmcblk0. mmcblk0boot0 is a separate
        // block device that lsblk relates to nothing, so the boot-disk
        // check does not catch it — the 4 MiB size floor does, which is
        // the honest state of this (see os/installer/README.md limits).
        let err = plan("/dev/mmcblk0boot0", &disks, &facts, MIN_TARGET_BYTES)
            .expect_err("a 4 MiB eMMC boot area is not an install target");
        assert!(matches!(err, Refusal::TooSmall { .. }), "{err:?}");
        // The other direction: mmcblk0 is correctly the boot disk, and
        // mmcblk0boot0 is NOT one of its partitions.
        let d = disks.iter().find(|d| d.path == "/dev/mmcblk0").unwrap();
        assert!(d.owns("/dev/mmcblk0p2"));
        assert!(!d.owns("/dev/mmcblk0boot0"));
    }

    #[test]
    fn refuses_a_disk_too_small_for_the_ab_layout() {
        let json = r#"{"blockdevices":[
          {"path":"/dev/sda","type":"disk","size":64023257088,"rm":true,"ro":false,
           "children":[{"path":"/dev/sda2","type":"part","size":10737418240,"partuuid":"22222222-2222-2222-2222-222222222222"}]},
          {"path":"/dev/sdb","type":"disk","size":16008609792,"rm":true,"ro":false,"model":"tiny stick"}
        ]}"#;
        let disks = parse_lsblk(json).unwrap();
        let facts = SystemFacts {
            mounts: parse_mounts("/dev/sda2 / btrfs rw 0 0\n"),
            loader_partuuid: None,
        };
        let err = plan("/dev/sdb", &disks, &facts, MIN_TARGET_BYTES)
            .expect_err("16 GB cannot hold a 23 GiB layout");
        assert_eq!(
            err,
            Refusal::TooSmall {
                target: "/dev/sdb".into(),
                have: 16008609792,
                need: MIN_TARGET_BYTES,
            }
        );
    }

    #[test]
    fn refuses_a_disk_with_a_mounted_filesystem_on_it() {
        let mut facts = live_facts();
        // The user opened the Windows partition in Files first.
        facts.mounts.push(Mount {
            source: "/dev/nvme0n1p3".into(),
            target: "/run/media/lisa/Windows".into(),
        });
        let err = plan("/dev/nvme0n1", &live_disks(), &facts, MIN_TARGET_BYTES)
            .expect_err("writing under a mount corrupts it");
        let Refusal::InUse { mounts, .. } = &err else {
            panic!("wrong refusal: {err:?}");
        };
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].target, "/run/media/lisa/Windows");
    }

    #[test]
    fn refuses_a_read_only_disk() {
        let json = r#"{"blockdevices":[
          {"path":"/dev/sda","type":"disk","size":64023257088,"rm":true,"ro":false,
           "children":[{"path":"/dev/sda2","type":"part","size":10737418240,"partuuid":"22222222-2222-2222-2222-222222222222"}]},
          {"path":"/dev/sdb","type":"disk","size":64023257088,"rm":true,"ro":"1","model":"write-protected"}
        ]}"#;
        let disks = parse_lsblk(json).unwrap();
        let facts = SystemFacts {
            mounts: parse_mounts("/dev/sda2 / btrfs rw 0 0\n"),
            loader_partuuid: None,
        };
        assert_eq!(
            plan("/dev/sdb", &disks, &facts, MIN_TARGET_BYTES).unwrap_err(),
            Refusal::ReadOnly {
                target: "/dev/sdb".into()
            }
        );
    }

    #[test]
    fn refuses_a_device_it_cannot_see() {
        assert_eq!(
            plan("/dev/sdz", &live_disks(), &live_facts(), MIN_TARGET_BYTES).unwrap_err(),
            Refusal::NoSuchDevice {
                target: "/dev/sdz".into()
            }
        );
    }

    /// Fail closed. If neither `/` nor the EFI variable resolves to a
    /// disk, we do not know which disk we are standing on — and every
    /// disk on the machine might be it.
    #[test]
    fn refuses_everything_when_the_boot_disk_cannot_be_identified() {
        let facts = SystemFacts {
            mounts: parse_mounts("overlay / overlay rw 0 0\ntmpfs /run tmpfs rw 0 0\n"),
            loader_partuuid: None,
        };
        for target in ["/dev/sda", "/dev/nvme0n1"] {
            let err = plan(target, &live_disks(), &facts, MIN_TARGET_BYTES)
                .expect_err("unknown boot disk must refuse everything");
            assert!(matches!(err, Refusal::BootDiskUnknown { .. }), "{err:?}");
        }
    }

    // -- the picker ----------------------------------------------------

    #[test]
    fn the_picker_offers_exactly_one_disk_in_a_live_session() {
        let disks = live_disks();
        let facts = live_facts();
        let rows = candidates(&disks, &facts, MIN_TARGET_BYTES);
        assert_eq!(rows.len(), 2);
        let ok: Vec<_> = rows
            .iter()
            .filter_map(|(_, r)| r.as_ref().ok())
            .map(|p| p.target.as_str())
            .collect();
        assert_eq!(ok, vec!["/dev/nvme0n1"]);
        assert!(rows[0].0.contains("removable"), "{}", rows[0].0);
        assert!(rows[1].0.contains("internal"), "{}", rows[1].0);
    }

    // -- parsing -------------------------------------------------------

    /// The text a person reads before typing a device node. Asserted on
    /// the whole string, because "which line said installable" is the
    /// only question the picker exists to answer.
    #[test]
    fn the_picker_names_the_stick_and_offers_the_internal_disk() {
        let text = render_targets(&live_disks(), &live_facts(), MIN_TARGET_BYTES);
        assert!(
            text.contains("running from removable media (/dev/sda) — a live session"),
            "{text}"
        );
        let installable: Vec<&str> = text
            .lines()
            .filter(|l| l.contains("[installable]"))
            .collect();
        assert_eq!(installable.len(), 1, "{text}");
        assert!(installable[0].contains("/dev/nvme0n1"), "{text}");
        // The stick is present, refused, and says why in the same block.
        assert!(text.contains("[   refused ] /dev/sda"), "{text}");
        assert!(
            text.contains("is the disk this system is running from"),
            "{text}"
        );
        // And the erase preview is itemised, not the word "everything".
        assert!(text.contains("erases /dev/nvme0n1p3"), "{text}");
    }

    /// When nothing resolves the boot disk, the picker must not present a
    /// menu of erasable disks — it must say it cannot tell.
    #[test]
    fn the_picker_offers_nothing_when_the_boot_disk_is_unknown() {
        let text = render_targets(&live_disks(), &SystemFacts::default(), MIN_TARGET_BYTES);
        assert!(text.contains("cannot tell which disk"), "{text}");
        assert!(!text.contains("[installable]"), "{text}");
    }

    #[test]
    fn lsblk_booleans_are_read_in_both_spellings() {
        let json = r#"{"blockdevices":[
          {"path":"/dev/sda","type":"disk","size":"64023257088","rm":"1","ro":"0"},
          {"path":"/dev/sdb","type":"disk","size":64023257088,"rm":false,"ro":true},
          {"path":"/dev/sr0","type":"rom","size":1,"rm":true,"ro":true},
          {"path":"/dev/loop0","type":"loop","size":20401094656,"rm":false,"ro":false}
        ]}"#;
        let disks = parse_lsblk(json).unwrap();
        // rom is not an install target; loop is (the only way this path
        // is exercised without hardware).
        assert_eq!(
            disks.iter().map(|d| d.path.as_str()).collect::<Vec<_>>(),
            vec!["/dev/sda", "/dev/sdb", "/dev/loop0"]
        );
        assert!(disks[0].removable && !disks[0].read_only);
        assert_eq!(disks[0].size, 64023257088);
        assert!(!disks[1].removable && disks[1].read_only);
        assert_eq!(disks[2].kind, "loop");
    }

    #[test]
    fn mount_points_with_spaces_are_unescaped() {
        let m = parse_mounts("/dev/sdb1 /run/media/lisa/My\\040Backup\\040Disk vfat rw 0 0\n");
        assert_eq!(m[0].target, "/run/media/lisa/My Backup Disk");
        assert_eq!(m[0].source, "/dev/sdb1");
    }

    #[test]
    fn a_btrfs_subvolume_suffix_does_not_hide_the_partition() {
        let m = parse_mounts("/dev/vda2[/@home] /home btrfs rw 0 0\n");
        assert_eq!(m[0].source, "/dev/vda2");
    }

    #[test]
    fn the_loader_variable_is_utf16_after_four_attribute_bytes() {
        let uuid = "11111111-1111-1111-1111-111111111111";
        let mut raw = vec![0x07, 0x00, 0x00, 0x00];
        for u in uuid.to_uppercase().encode_utf16() {
            raw.extend_from_slice(&u.to_le_bytes());
        }
        raw.extend_from_slice(&[0, 0]);
        assert_eq!(decode_loader_partuuid(&raw).as_deref(), Some(uuid));
        // Truncated / empty payloads are an absent signal, not a match.
        assert_eq!(decode_loader_partuuid(&[0x07, 0x00]), None);
        assert_eq!(
            decode_loader_partuuid(&[0x07, 0x00, 0x00, 0x00, 0, 0]),
            None
        );
    }

    #[test]
    fn the_minimum_target_matches_the_repart_layout() {
        // 1 GiB ESP + 10 + 10 GiB roots + 2 GiB var. Spelled out so a
        // change to os/mkosi/mkosi.repart/ that forgets this constant
        // fails here rather than in the field.
        assert_eq!(MIN_TARGET_BYTES, (1 + 10 + 10 + 2) * (1 << 30));
    }

    // -- stacked block devices (issue #290) ----------------------------
    //
    // Every fixture below is an ordinary Linux desktop, not an attack.
    // What they have in common is that the source of `/` is not a
    // partition path: it is a dm node, an md node, or one member of a
    // filesystem that spans two disks. Before #290 was fixed, `owns()`
    // compared strings against the depth-1 `part` children only, so no
    // disk claimed the root mount, and the ESP disk's `Loader` match
    // satisfied a *global* gate that then unlocked every other disk on
    // the machine.

    /// `/` on an LV, inside a VG, inside a LUKS container, on the NVMe.
    /// The ESP is on an older SATA SSD — "the disk I added later holds
    /// the system, the old one still boots it" is a normal history.
    const LUKS_LVM: &str = r#"{
      "blockdevices": [
        {"path":"/dev/sda","type":"disk","size":64023257088,"rm":false,"ro":false,
         "model":"boot ssd",
         "children":[
           {"path":"/dev/sda1","type":"part","size":1073741824,"partlabel":"esp",
            "partuuid":"aaaaaaaa-0000-0000-0000-00000000000a","fstype":"vfat","uuid":"1234-ABCD"}
         ]},
        {"path":"/dev/nvme0n1","type":"disk","size":512110190592,"rm":false,"ro":false,
         "model":"root nvme",
         "children":[
           {"path":"/dev/nvme0n1p1","type":"part","size":512108658688,"partlabel":"luks",
            "partuuid":"ffffffff-0000-0000-0000-00000000000f","fstype":"crypto_LUKS",
            "uuid":"6f1b0e1a-luks-header",
            "children":[
              {"path":"/dev/mapper/luks-6f1b0e1a","type":"crypt","size":512106561536,
               "fstype":"LVM2_member","uuid":"pv-2f9c",
               "children":[
                 {"path":"/dev/mapper/vg-root","type":"lvm","size":483183820800,
                  "fstype":"ext4","uuid":"root-ext4-uuid"},
                 {"path":"/dev/mapper/vg-swap","type":"lvm","size":17179869184,
                  "fstype":"swap","uuid":"swap-uuid"}
               ]}
            ]}
         ]},
        {"path":"/dev/sdc","type":"disk","size":256060514304,"rm":false,"ro":false,
         "model":"genuinely spare"}
      ]}"#;

    fn luks_lvm_facts() -> SystemFacts {
        SystemFacts {
            mounts: parse_mounts(
                "/dev/mapper/vg-root / ext4 rw,relatime 0 0\n\
                 /dev/sda1 /efi vfat rw,umask=0077 0 0\n\
                 tmpfs /run tmpfs rw 0 0\n",
            ),
            loader_partuuid: Some("aaaaaaaa-0000-0000-0000-00000000000a".into()),
        }
    }

    /// The headline of #290: the disk holding the running root was
    /// offered, recommended, and captioned "not this disk".
    #[test]
    fn refuses_the_disk_whose_lvm_over_luks_holds_the_running_root() {
        let disks = parse_lsblk(LUKS_LVM).unwrap();
        let err = plan("/dev/nvme0n1", &disks, &luks_lvm_facts(), MIN_TARGET_BYTES)
            .expect_err("/ lives on this disk, three layers down");
        let Refusal::IsTheBootDisk { evidence, .. } = &err else {
            panic!("wrong refusal: {err:?}");
        };
        assert!(
            evidence.iter().any(
                |e| matches!(e, Evidence::RootMount { source } if source == "/dev/mapper/vg-root")
            ),
            "{evidence:?}"
        );
    }

    /// The same topology, the same run: the spare disk stays usable.
    /// Without this the "fix" could be `Err` everywhere, which is not a
    /// fix, it is a broken verb.
    #[test]
    fn a_genuinely_spare_disk_is_still_installable_under_lvm() {
        let disks = parse_lsblk(LUKS_LVM).unwrap();
        let p = plan("/dev/sdc", &disks, &luks_lvm_facts(), MIN_TARGET_BYTES)
            .expect("an unrelated empty disk is a legal target");
        assert_eq!(p.target, "/dev/sdc");
        // And the caption above the ERASE prompt names where we really
        // are — the root disk, not merely whichever disk holds the ESP.
        assert_eq!(p.boot_disks, vec!["/dev/nvme0n1".to_string()]);
    }

    /// md RAID1: `/` is `/dev/md0`, and erasing *either* member ends the
    /// array. lsblk nests the md node under both.
    const MD_RAID1: &str = r#"{
      "blockdevices": [
        {"path":"/dev/sda","type":"disk","size":512110190592,"rm":false,"ro":false,
         "model":"mirror a",
         "children":[
           {"path":"/dev/sda1","type":"part","size":1073741824,"partlabel":"esp",
            "partuuid":"aaaaaaaa-0000-0000-0000-00000000000a","fstype":"vfat","uuid":"1234-ABCD"},
           {"path":"/dev/sda2","type":"part","size":511035400192,"partlabel":"raid",
            "partuuid":"aaaaaaaa-0000-0000-0000-00000000000b","fstype":"linux_raid_member",
            "children":[
              {"path":"/dev/md0","type":"raid1","size":511034351616,"fstype":"ext4",
               "uuid":"md-root-uuid"}
            ]}
         ]},
        {"path":"/dev/sdb","type":"disk","size":512110190592,"rm":false,"ro":false,
         "model":"mirror b",
         "children":[
           {"path":"/dev/sdb1","type":"part","size":511035400192,"partlabel":"raid",
            "partuuid":"bbbbbbbb-0000-0000-0000-00000000000b","fstype":"linux_raid_member",
            "children":[
              {"path":"/dev/md0","type":"raid1","size":511034351616,"fstype":"ext4",
               "uuid":"md-root-uuid"}
            ]}
         ]}
      ]}"#;

    #[test]
    fn refuses_both_halves_of_the_mirror_the_root_array_is_built_from() {
        let disks = parse_lsblk(MD_RAID1).unwrap();
        let facts = SystemFacts {
            mounts: parse_mounts("/dev/md0 / ext4 rw 0 0\n/dev/sda1 /efi vfat rw 0 0\n"),
            loader_partuuid: Some("aaaaaaaa-0000-0000-0000-00000000000a".into()),
        };
        for target in ["/dev/sda", "/dev/sdb"] {
            let err = plan(target, &disks, &facts, MIN_TARGET_BYTES)
                .expect_err("erasing either mirror half destroys the running root");
            assert!(
                matches!(err, Refusal::IsTheBootDisk { .. }),
                "{target}: {err:?}"
            );
        }
    }

    /// Multi-device btrfs: lsblk relates the two members to each other
    /// through nothing but the filesystem UUID they share. `/` names
    /// only the device it was mounted from; the other one is just as
    /// load-bearing and looks empty from the mount table.
    const BTRFS_TWO_DEVICES: &str = r#"{
      "blockdevices": [
        {"path":"/dev/sda","type":"disk","size":512110190592,"rm":false,"ro":false,
         "model":"btrfs a",
         "children":[
           {"path":"/dev/sda1","type":"part","size":1073741824,"partlabel":"esp",
            "partuuid":"aaaaaaaa-0000-0000-0000-00000000000a","fstype":"vfat","uuid":"1234-ABCD"},
           {"path":"/dev/sda2","type":"part","size":511035400192,"partlabel":"root",
            "partuuid":"aaaaaaaa-0000-0000-0000-00000000000b","fstype":"btrfs",
            "uuid":"9d0c7e3a-btrfs-pool"}
         ]},
        {"path":"/dev/sdb","type":"disk","size":512110190592,"rm":false,"ro":false,
         "model":"btrfs b",
         "children":[
           {"path":"/dev/sdb1","type":"part","size":512108658688,"partlabel":"root",
            "partuuid":"bbbbbbbb-0000-0000-0000-00000000000b","fstype":"btrfs",
            "uuid":"9d0c7e3a-btrfs-pool"}
         ]}
      ]}"#;

    #[test]
    fn refuses_the_second_device_of_the_btrfs_the_root_is_on() {
        let disks = parse_lsblk(BTRFS_TWO_DEVICES).unwrap();
        let facts = SystemFacts {
            mounts: parse_mounts("/dev/sda2 / btrfs rw 0 0\n/dev/sda1 /efi vfat rw 0 0\n"),
            loader_partuuid: Some("aaaaaaaa-0000-0000-0000-00000000000a".into()),
        };
        let err = plan("/dev/sdb", &disks, &facts, MIN_TARGET_BYTES)
            .expect_err("the other half of the running filesystem is not a target");
        let Refusal::IsTheBootDisk { evidence, .. } = &err else {
            panic!("wrong refusal: {err:?}");
        };
        assert_eq!(
            evidence,
            &vec![Evidence::RootFilesystemMember {
                device: "/dev/sdb1".into(),
                fstype: "btrfs".into(),
                fs_uuid: "9d0c7e3a-btrfs-pool".into(),
            }],
        );
        // Both members named, so neither reads as an all-clear.
        let text = render_targets(&disks, &facts, MIN_TARGET_BYTES);
        assert!(text.contains("running from /dev/sda, /dev/sdb"), "{text}");
        assert!(!text.contains("[installable]"), "{text}");
    }

    /// The gate itself, isolated from the topologies above: one ESP
    /// match on one disk must not certify that we know where we are
    /// standing, because the loader variable says nothing about `/`.
    #[test]
    fn a_loader_match_on_one_disk_does_not_clear_a_different_disk() {
        let facts = SystemFacts {
            // A root we cannot resolve — an initrd that never
            // switch-rooted is the textbook case.
            mounts: parse_mounts("rootfs / rootfs rw 0 0\n/dev/sda1 /efi vfat rw 0 0\n"),
            loader_partuuid: Some("11111111-1111-1111-1111-111111111111".into()),
        };
        // /dev/sda carries the ESP, so it is refused by name.
        assert!(matches!(
            plan("/dev/sda", &live_disks(), &facts, MIN_TARGET_BYTES).unwrap_err(),
            Refusal::IsTheBootDisk { .. }
        ));
        // The other disk is a different question, and nothing here
        // answers it.
        let err = plan("/dev/nvme0n1", &live_disks(), &facts, MIN_TARGET_BYTES)
            .expect_err("knowing the ESP is not knowing where / lives");
        assert!(matches!(err, Refusal::BootDiskUnknown { .. }), "{err:?}");
        // And the header must not promise a menu it cannot serve.
        let text = render_targets(&live_disks(), &facts, MIN_TARGET_BYTES);
        assert!(text.contains("but / did not resolve to any disk"), "{text}");
        assert!(!text.contains("[installable]"), "{text}");
        assert!(!text.contains("what this verb is for"), "{text}");
    }

    /// The picker is the surface #290 lied on. Under LVM it must offer
    /// the spare disk and refuse the two that carry the running system.
    #[test]
    fn the_picker_does_not_offer_the_running_system_under_lvm() {
        let disks = parse_lsblk(LUKS_LVM).unwrap();
        let text = render_targets(&disks, &luks_lvm_facts(), MIN_TARGET_BYTES);
        let installable: Vec<&str> = text
            .lines()
            .filter(|l| l.contains("[installable]"))
            .collect();
        assert_eq!(installable.len(), 1, "{text}");
        assert!(installable[0].contains("/dev/sdc"), "{text}");
        assert!(text.contains("[   refused ] /dev/nvme0n1"), "{text}");
        assert!(text.contains("running from /dev/nvme0n1"), "{text}");
    }

    // -- issue #301 ----------------------------------------------------

    /// The one mutation the suite did not kill: every fixture was
    /// already lowercase, so `parse_lsblk`'s `.to_lowercase()` on
    /// PARTUUID was asserted nowhere. systemd-stub writes the UUID
    /// uppercase, so without the normalization the `Loader` signal
    /// stops matching — in silence, and on a machine where the mount
    /// signal resolves a different disk that silence is a wrong-disk
    /// `Ok`, not a refusal.
    #[test]
    fn an_uppercase_partuuid_from_lsblk_still_matches_the_loader_variable() {
        let json = r#"{"blockdevices":[
          {"path":"/dev/sda","type":"disk","size":64023257088,"rm":true,"ro":false,
           "children":[{"path":"/dev/sda1","type":"part","size":1073741824,"partlabel":"esp",
            "partuuid":"AAAAAAAA-0000-0000-0000-00000000000A","fstype":"vfat"}]},
          {"path":"/dev/sdb","type":"disk","size":512110190592,"rm":false,"ro":false,
           "children":[{"path":"/dev/sdb1","type":"part","size":511035400192,"partlabel":"root",
            "partuuid":"bbbbbbbb-0000-0000-0000-00000000000b","fstype":"ext4",
            "uuid":"other-root"}]}
        ]}"#;
        let disks = parse_lsblk(json).unwrap();
        assert_eq!(
            disks[0].partitions[0].partuuid.as_deref(),
            Some("aaaaaaaa-0000-0000-0000-00000000000a"),
            "lsblk's PARTUUID is normalised on the way in"
        );
        // The signal, end to end: `/` is on sdb, so nothing but the
        // loader variable can refuse sda — and it is spelled the way
        // systemd-stub spells it.
        let facts = SystemFacts {
            mounts: parse_mounts("/dev/sdb1 / ext4 rw 0 0\n"),
            loader_partuuid: decode_loader_partuuid(&utf16_efivar(
                "AAAAAAAA-0000-0000-0000-00000000000A",
            )),
        };
        let err = plan("/dev/sda", &disks, &facts, MIN_TARGET_BYTES)
            .expect_err("the stick the loader ran from is not a target");
        let Refusal::IsTheBootDisk { evidence, .. } = &err else {
            panic!("wrong refusal: {err:?}");
        };
        assert_eq!(
            evidence,
            &vec![Evidence::Loader {
                partition: "/dev/sda1".into()
            }]
        );
    }

    /// An efivarfs payload: four attribute bytes, then UTF-16LE, NUL.
    fn utf16_efivar(s: &str) -> Vec<u8> {
        let mut raw = vec![0x07, 0x00, 0x00, 0x00];
        for u in s.encode_utf16() {
            raw.extend_from_slice(&u.to_le_bytes());
        }
        raw.extend_from_slice(&[0, 0]);
        raw
    }

    /// Re-flashing a machine from the stick that installed it. The byte
    /// copy duplicated the ESP's PARTUUID, so `LoaderDevicePartUUID`
    /// matches a partition on both disks — and the old code took the
    /// first match, refusing the target and naming the wrong disk as
    /// the one we booted from. A PARTUUID that names two partitions
    /// identifies neither; the root mount breaks the tie.
    #[test]
    fn a_duplicated_esp_partuuid_does_not_refuse_the_reinstall() {
        let json = r#"{"blockdevices":[
          {"path":"/dev/sda","type":"disk","size":64023257088,"rm":true,"ro":false,
           "model":"Cruzer Blade",
           "children":[
             {"path":"/dev/sda1","type":"part","size":1073741824,"partlabel":"esp",
              "partuuid":"aaaaaaaa-0000-0000-0000-00000000000a","fstype":"vfat","uuid":"1234-ABCD"},
             {"path":"/dev/sda2","type":"part","size":10737418240,"partlabel":"root_1",
              "partuuid":"aaaaaaaa-0000-0000-0000-00000000000b","fstype":"btrfs",
              "uuid":"stick-root-fsid"}
           ]},
          {"path":"/dev/nvme0n1","type":"disk","size":512110190592,"rm":false,"ro":false,
           "model":"the machine",
           "children":[
             {"path":"/dev/nvme0n1p1","type":"part","size":1073741824,"partlabel":"esp",
              "partuuid":"aaaaaaaa-0000-0000-0000-00000000000a","fstype":"vfat","uuid":"1234-ABCD"},
             {"path":"/dev/nvme0n1p2","type":"part","size":10737418240,"partlabel":"root_1",
              "partuuid":"aaaaaaaa-0000-0000-0000-00000000000b","fstype":"btrfs",
              "uuid":"installed-root-fsid"}
           ]}
        ]}"#;
        let disks = parse_lsblk(json).unwrap();
        let facts = SystemFacts {
            mounts: parse_mounts("/dev/sda2 / btrfs rw 0 0\n/dev/sda1 /efi vfat rw 0 0\n"),
            loader_partuuid: Some("aaaaaaaa-0000-0000-0000-00000000000a".into()),
        };
        // The stick is still refused, by the root mount.
        assert!(matches!(
            plan("/dev/sda", &disks, &facts, MIN_TARGET_BYTES).unwrap_err(),
            Refusal::IsTheBootDisk { .. }
        ));
        // And the machine can be re-flashed.
        let p = plan("/dev/nvme0n1", &disks, &facts, MIN_TARGET_BYTES)
            .expect("re-installing over the previous install is the whole point");
        assert_eq!(p.boot_disks, vec!["/dev/sda".to_string()]);

        // The tie-break is the root mount, so when nothing resolves the
        // root, the ambiguous PARTUUID must refuse *both* disks rather
        // than pick one.
        let blind = SystemFacts {
            mounts: parse_mounts("rootfs / rootfs rw 0 0\n"),
            loader_partuuid: Some("aaaaaaaa-0000-0000-0000-00000000000a".into()),
        };
        for target in ["/dev/sda", "/dev/nvme0n1"] {
            let err = plan(target, &disks, &blind, MIN_TARGET_BYTES)
                .expect_err("an ambiguous loader match clears nothing");
            assert!(
                matches!(err, Refusal::IsTheBootDisk { .. }),
                "{target}: {err:?}"
            );
        }
    }

    /// `unescape_octal` used to widen each byte to a codepoint, which is
    /// Latin-1: a UTF-8 mount source came back mangled and compared
    /// equal to no device lsblk had ever named.
    #[test]
    fn octal_escapes_decode_to_utf8_not_latin_1() {
        // /dev/mapper/caféx, escaped the way the kernel escapes it.
        let m = parse_mounts("/dev/mapper/caf\\303\\251x /mnt ext4 rw 0 0\n");
        assert_eq!(m[0].source, "/dev/mapper/caféx");
        // The same string arriving already-decoded must survive too.
        let m = parse_mounts("/dev/mapper/caféx /mnt ext4 rw 0 0\n");
        assert_eq!(m[0].source, "/dev/mapper/caféx");
        // …and it must be the string a disk can own.
        let json = r#"{"blockdevices":[
          {"path":"/dev/sda","type":"disk","size":512110190592,"rm":false,"ro":false,
           "children":[{"path":"/dev/sda1","type":"part","size":511035400192,
            "partuuid":"aaaaaaaa-0000-0000-0000-00000000000a",
            "children":[{"path":"/dev/mapper/caféx","type":"crypt","size":511034351616,
             "fstype":"ext4","uuid":"cafe-root"}]}]}
        ]}"#;
        let disks = parse_lsblk(json).unwrap();
        assert!(disks[0].owns("/dev/mapper/caféx"));
        let facts = SystemFacts {
            mounts: parse_mounts("/dev/mapper/caf\\303\\251x / ext4 rw 0 0\n"),
            loader_partuuid: None,
        };
        assert!(matches!(
            plan("/dev/sda", &disks, &facts, MIN_TARGET_BYTES).unwrap_err(),
            Refusal::IsTheBootDisk { .. }
        ));
    }

    /// A literal backslash followed by a multi-byte character used to
    /// index a `&str` off a char boundary, which panics — `lisa install`
    /// aborting on a mount table it merely had to read.
    #[test]
    fn a_backslash_before_a_multibyte_character_does_not_panic() {
        let m = parse_mounts("/dev/sda1 /mnt/\\éz ext4 rw 0 0\n");
        assert_eq!(m[0].target, "/mnt/\\éz");
        // Out-of-range escapes stay literal rather than truncating into
        // some other character.
        let m = parse_mounts("/dev/sda1 /mnt/\\777 ext4 rw 0 0\n");
        assert_eq!(m[0].target, "/mnt/\\777");
        // A backslash followed by three characters that are not all
        // *octal* digits is a literal backslash. Both of these decode
        // to a byte in range if the digit test is skipped, so without
        // it `\099` silently becomes `I` and `\00a` becomes `1` — a
        // mount point quietly renamed, which is an `InUse` miss.
        for literal in ["/mnt/\\099", "/mnt/\\00a", "/mnt/\\08!"] {
            let line = format!("/dev/sda1 {literal} ext4 rw 0 0\n");
            assert_eq!(parse_mounts(&line)[0].target, literal, "{literal}");
        }
        // …while the real escapes still decode.
        assert_eq!(
            parse_mounts("/dev/sda1 /mnt/a\\040b ext4 rw 0 0\n")[0].target,
            "/mnt/a b"
        );
    }
}
