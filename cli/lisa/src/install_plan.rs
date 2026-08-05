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
    pub partitions: Vec<Partition>,
}

impl Disk {
    /// Is `dev` this disk, or one of its partitions?
    ///
    /// Set membership over the topology, never a string prefix. See the
    /// module docs for what `starts_with` did to eMMC.
    pub fn owns(&self, dev: &str) -> bool {
        self.path == dev || self.partitions.iter().any(|p| p.path == dev)
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

/// Parse `lsblk -J -b -o PATH,TYPE,SIZE,RM,RO,MODEL,PARTLABEL,PARTUUID`.
///
/// Tolerant on purpose about the two shapes util-linux has shipped: `rm`
/// and `ro` are JSON booleans in modern lsblk and the strings `"0"`/`"1"`
/// in older ones, and `size` is a number under `-b` but a string in some
/// builds. Getting either wrong silently is how a read-only disk becomes
/// writable in the planner's eyes.
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
        out.push(Disk {
            path,
            kind,
            size: num_field(d, "size").unwrap_or(0),
            removable: bool_field(d, "rm"),
            read_only: bool_field(d, "ro"),
            model: str_field(d, "model").filter(|s| !s.is_empty()),
            partitions,
        });
    }
    Ok(out)
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

fn unescape_octal(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            let digits = &s[i + 1..i + 4];
            if let Ok(code) = u8::from_str_radix(digits, 8) {
                out.push(code as char);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Why we believe a disk is the one this system is running from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evidence {
    /// `/` is mounted from a partition of this disk.
    RootMount { source: String },
    /// systemd-stub says the boot loader was loaded from this partition.
    Loader { partition: String },
}

impl fmt::Display for Evidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Evidence::RootMount { source } => write!(f, "/ is mounted from {source}"),
            Evidence::Loader { partition } => {
                write!(
                    f,
                    "the boot loader was loaded from {partition} (EFI LoaderDevicePartUUID)"
                )
            }
        }
    }
}

/// Evidence that `disk` is the booted disk. Two independent signals,
/// because each one alone has a blind spot: `/` tells us nothing when the
/// running root is not a block device (initrd, rescue, overlay), and the
/// EFI variable is absent on a direct-kernel boot (CI) and on non-EFI
/// machines.
pub fn boot_evidence(disk: &Disk, facts: &SystemFacts) -> Vec<Evidence> {
    let mut out = Vec::new();
    for m in &facts.mounts {
        if m.target == "/" && disk.owns(&m.source) {
            out.push(Evidence::RootMount {
                source: m.source.clone(),
            });
        }
    }
    if let Some(uuid) = &facts.loader_partuuid {
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

/// The disk this system booted from, if any signal resolves it.
pub fn boot_disk<'a>(disks: &'a [Disk], facts: &SystemFacts) -> Option<&'a Disk> {
    disks.iter().find(|d| !boot_evidence(d, facts).is_empty())
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
                "{target} is a partition, not a disk. The image carries its own partition \
                 table, so it is written to a whole disk — you probably meant {disk}, which \
                 erases {target} along with everything else on it"
            ),
            Refusal::BootDiskUnknown { target } => write!(
                f,
                "cannot tell which disk this system is running from — neither / nor the EFI \
                 loader variable resolved to a disk — so {target} cannot be ruled out as that \
                 disk. Refusing to write"
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
    /// The disk we are running from — never the target, by construction.
    pub boot_disk: String,
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
        // A partition of a known disk gets the useful message; anything
        // else is simply not here.
        if let Some(owner) = disks.iter().find(|d| d.owns(&target)) {
            return Err(Refusal::IsAPartition {
                target,
                disk: owner.path.clone(),
            });
        }
        return Err(Refusal::NoSuchDevice { target });
    };

    let Some(booted) = boot_disk(disks, facts) else {
        return Err(Refusal::BootDiskUnknown { target });
    };

    let evidence = boot_evidence(disk, facts);
    if !evidence.is_empty() {
        return Err(Refusal::IsTheBootDisk { target, evidence });
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
        boot_disk: booted.path.clone(),
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
    let booted = boot_disk(disks, facts).map(|d| d.path.as_str());
    match (booted, is_live_session(disks, facts)) {
        (Some(path), Some(true)) => out.push_str(&format!(
            ">> running from removable media ({path}) — a live session; installing to \
             another disk is what this verb is for\n"
        )),
        (Some(path), _) => out.push_str(&format!(
            ">> running from {path} — an installed system, not a live session\n"
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
        .args([
            "-J",
            "-b",
            "-o",
            "PATH,TYPE,SIZE,RM,RO,MODEL,PARTLABEL,PARTUUID",
        ])
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
        assert_eq!(p.boot_disk, "/dev/sda");
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
}
