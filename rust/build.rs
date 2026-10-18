//! Windows resources for the exe: the icon (assets/app.ico) and the version info (name in Task
//! Manager, version in file properties). Written as a .res file that link.exe takes directly, so no
//! resource compiler is needed. The version comes from Cargo.toml.

use std::{env, fs, path::PathBuf};

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
const LANG_EN: u16 = 0x0409;

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("..");
    let ico_path = root.join("assets").join("app.ico");
    println!("cargo:rerun-if-changed={}", ico_path.display());
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let ver = env::var("CARGO_PKG_VERSION").unwrap();
    let ico = fs::read(&ico_path).expect("assets/app.ico");
    let icons = read_ico(&ico);

    let mut res = Vec::new();
    res.extend(entry(0, 0, 0, &[])); // a .res file starts with an empty entry
    for (i, ic) in icons.iter().enumerate() {
        res.extend(entry(RT_ICON, i as u16 + 1, 0, &ic.data));
    }
    res.extend(entry(RT_GROUP_ICON, 1, 0, &group_icon(&icons)));
    res.extend(entry(RT_VERSION, 1, LANG_EN, &version_info(&ver)));

    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("arcmix.res");
    fs::write(&out, res).unwrap();
    println!("cargo:rustc-link-arg-bins={}", out.display());
}

struct Icon {
    w: u8,
    h: u8,
    colors: u8,
    planes: u16,
    bits: u16,
    data: Vec<u8>,
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn read_ico(d: &[u8]) -> Vec<Icon> {
    assert_eq!(u16_at(d, 2), 1, "not an icon file");
    (0..u16_at(d, 4) as usize)
        .map(|i| {
            let e = 6 + 16 * i;
            let (size, off) = (u32_at(d, e + 8) as usize, u32_at(d, e + 12) as usize);
            Icon { w: d[e], h: d[e + 1], colors: d[e + 2], planes: u16_at(d, e + 4), bits: u16_at(d, e + 6), data: d[off..off + size].to_vec() }
        })
        .collect()
}

fn group_icon(icons: &[Icon]) -> Vec<u8> {
    let mut b = Vec::new();
    for v in [0u16, 1, icons.len() as u16] {
        b.extend(v.to_le_bytes());
    }
    for (i, ic) in icons.iter().enumerate() {
        b.extend([ic.w, ic.h, ic.colors, 0]);
        b.extend(ic.planes.to_le_bytes());
        b.extend(ic.bits.to_le_bytes());
        b.extend((ic.data.len() as u32).to_le_bytes());
        b.extend((i as u16 + 1).to_le_bytes());
    }
    b
}

fn pad4(b: &mut Vec<u8>) {
    while b.len() % 4 != 0 {
        b.push(0);
    }
}

/// One resource of a .res file: header (type and name by number) and data, both DWORD aligned.
fn entry(typ: u16, id: u16, lang: u16, data: &[u8]) -> Vec<u8> {
    let mut hdr = Vec::new();
    hdr.extend((data.len() as u32).to_le_bytes());
    hdr.extend(32u32.to_le_bytes()); // header size: fixed, as type and name are numbers
    hdr.extend([0xFF, 0xFF]);
    hdr.extend(typ.to_le_bytes());
    hdr.extend([0xFF, 0xFF]);
    hdr.extend(id.to_le_bytes());
    hdr.extend(0u32.to_le_bytes()); // data version
    let flags: u16 = if typ == 0 { 0 } else { 0x1030 }; // MOVEABLE | PURE | DISCARDABLE
    hdr.extend(flags.to_le_bytes());
    hdr.extend(lang.to_le_bytes());
    hdr.extend(0u32.to_le_bytes()); // version
    hdr.extend(0u32.to_le_bytes()); // characteristics
    hdr.extend(data);
    pad4(&mut hdr);
    hdr
}

fn utf16z(s: &str) -> Vec<u8> {
    s.encode_utf16().chain([0]).flat_map(|c| c.to_le_bytes()).collect()
}

/// VS_VERSIONINFO node: wLength, wValueLength, wType, szKey, padding, value, padding, children.
fn ver_node(key: &str, value: &[u8], text: bool, value_len: Option<u16>, children: &[Vec<u8>]) -> Vec<u8> {
    let mut b = vec![0, 0, 0, 0];
    b.extend((text as u16).to_le_bytes());
    b.extend(utf16z(key));
    pad4(&mut b);
    b.extend(value);
    for c in children {
        pad4(&mut b);
        b.extend(c);
    }
    let len = b.len() as u16;
    let vl = value_len.unwrap_or(value.len() as u16);
    b[0..2].copy_from_slice(&len.to_le_bytes());
    b[2..4].copy_from_slice(&vl.to_le_bytes());
    b
}

fn version_info(ver: &str) -> Vec<u8> {
    let core = ver.split('-').next().unwrap();
    let mut nums: Vec<u32> = core.split('.').filter_map(|x| x.parse().ok()).take(4).collect();
    nums.resize(4, 0);
    let (ms, ls) = ((nums[0] << 16) | nums[1], (nums[2] << 16) | nums[3]);
    let prerelease = if ver.contains('-') { 0x2 } else { 0 };
    let mut fixed = Vec::new();
    for v in [0xFEEF04BDu32, 0x00010000, ms, ls, ms, ls, 0x3F, prerelease, 0x40004, 1, 0, 0, 0] {
        fixed.extend(v.to_le_bytes());
    }
    let strings = [
        ("CompanyName", "Arcmix"),
        ("FileDescription", "Arcmix"),
        ("FileVersion", ver),
        ("InternalName", "Arcmix"),
        ("LegalCopyright", "GNU GPL v3"),
        ("OriginalFilename", "Arcmix.exe"),
        ("ProductName", "Arcmix"),
        ("ProductVersion", ver),
    ];
    let nodes: Vec<Vec<u8>> = strings
        .iter()
        .map(|(k, v)| ver_node(k, &utf16z(v), true, Some(v.encode_utf16().count() as u16 + 1), &[]))
        .collect();
    let table = ver_node("040904B0", &[], true, None, &nodes);
    let sfi = ver_node("StringFileInfo", &[], true, None, &[table]);
    let mut tr = Vec::new();
    tr.extend(LANG_EN.to_le_bytes());
    tr.extend(0x04B0u16.to_le_bytes());
    let var = ver_node("Translation", &tr, false, None, &[]);
    let vfi = ver_node("VarFileInfo", &[], true, None, &[var]);
    ver_node("VS_VERSION_INFO", &fixed, false, None, &[sfi, vfi])
}
