//! Opt-in regression checks against Dell's stock M3B107 UPG, which this repository does not ship.
//! Set `AWTOOL_STOCK_UPG` to its path and run `cargo test -- --include-ignored`.

use crate::commands::{STOCK_M3B107_SHA256, apply_patch, sha256_hex};
use crate::config::{PROFILE_175HZ, PatchConfig};
use crate::{edid, upg::Upg};
use sha2::{Digest, Sha256};

const PROFILE_175HZ_FREESYNC: &str = include_str!("../profiles/175hz-freesync.toml");
const PROFILE_177HZ_FREESYNC: &str = include_str!("../profiles/177hz-freesync.toml");

fn stock() -> Upg {
    let path = std::env::var("AWTOOL_STOCK_UPG")
        .expect("set AWTOOL_STOCK_UPG to Dell's AW3423DWF_M3B107_20240130(ISP).UPG");
    let upg = Upg::load(&path).expect("could not load AWTOOL_STOCK_UPG");
    assert!(
        Sha256::digest(upg.bytes())[..] == STOCK_M3B107_SHA256,
        "AWTOOL_STOCK_UPG is not the stock M3B107 image"
    );
    upg
}

#[test]
#[ignore = "requires AWTOOL_STOCK_UPG"]
fn profiles_reproduce_published_hashes() {
    for (text, want) in [
        (
            PROFILE_175HZ,
            "56f7bdb33d0da8322da62b88ef180d7b9db9295ce5f4ee6e5b66fae937a7b871",
        ),
        (
            PROFILE_175HZ_FREESYNC,
            "27bb8a4ebfbf92a8654903b9cbf2d57a27e1a1c6e5c0a4b9dcb3e0d544b4ab71",
        ),
        (
            PROFILE_177HZ_FREESYNC,
            "0ef141aa78185e0955c3b4aa128367eb7f34bfa105799ac3d2c7f30309195950",
        ),
    ] {
        let mut upg = stock();
        apply_patch(&mut upg, &PatchConfig::parse(text).unwrap()).unwrap();
        upg.fix_checksums();
        assert_eq!(sha256_hex(upg.bytes()), want);
    }
}

#[test]
#[ignore = "requires AWTOOL_STOCK_UPG"]
fn profiles_have_coherent_input_limits_and_output_window() {
    for text in [
        PROFILE_175HZ,
        PROFILE_175HZ_FREESYNC,
        PROFILE_177HZ_FREESYNC,
    ] {
        let config = PatchConfig::parse(text).unwrap();
        let mut upg = stock();
        apply_patch(&mut upg, &config).unwrap();
        let ceiling = config.range.as_ref().unwrap().max_vhz;
        let input_h = config.input.as_ref().map_or(285, |i| i.max_h_khz);
        let output = config.output.as_ref().unwrap();
        let floor = config.panel.as_ref().unwrap().dv_total_min;
        for h in [3584u64, 3600] {
            assert!(
                h * u64::from(floor) * u64::from(ceiling) <= u64::from(output.clock_max_khz) * 1000
            );
        }
        for block in upg.blocks() {
            let view = upg.flash_view(&block);
            for (at, ext) in edid::find(&view) {
                let e = &view[at..at + 128 * (ext + 1)];
                assert!(edid::checksums_ok(e));
                for timing in edid::timings(e)
                    .into_iter()
                    .filter(|t| t.kind == edid::Kind::DisplayId1)
                {
                    assert!(timing.hz() <= f64::from(ceiling));
                    assert!(
                        timing.pclk <= u64::from(input_h) * 1000 * u64::from(timing.ha + timing.hb)
                    );
                }
            }
            for (low, high) in [(0xA357, 0xA35A), (0xA3A1, 0xA3A4)] {
                assert_eq!(u16::from_be_bytes([view[high], view[low]]), input_h * 10);
            }
        }
        upg.fix_checksums();
        upg.require_valid_checksums().unwrap();
    }
}

#[test]
#[ignore = "requires AWTOOL_STOCK_UPG"]
fn freesync_profile_changes_only_dp_maxima_and_cta_checksums() {
    let mut upg = stock();
    let mut baseline = stock();
    apply_patch(&mut baseline, &PatchConfig::parse(PROFILE_175HZ).unwrap()).unwrap();
    let config = PatchConfig::parse(PROFILE_175HZ_FREESYNC).unwrap();
    apply_patch(&mut upg, &config).unwrap();
    for block in upg.blocks() {
        let mut expected = baseline.flash_view(&block);
        let mut count = 0;
        for (at, ext) in edid::find(&expected) {
            // Independent exact stock AMD payload anchors, including SDR/HDR flags.
            for base in (128..128 * (ext + 1)).step_by(128) {
                if expected[at + base] != 2 || expected[at + 20] & 15 != 5 {
                    continue;
                }
                let end = at + base + expected[at + base + 2] as usize;
                let mut pos = at + base + 4;
                while pos < end {
                    let size = (expected[pos] & 31) as usize;
                    if expected[pos] == 0x72 && expected[pos + 1..pos + 5] == [0x1a, 0, 0, 3] {
                        assert_eq!(expected[pos + 7], 165);
                        assert_eq!(expected[pos + 14], 165);
                        expected[pos + 7] = 175;
                        expected[pos + 14] = 175;
                        count += 1;
                    }
                    pos += 1 + size;
                }
                let start = at + base;
                let sum = expected[start..start + 127]
                    .iter()
                    .fold(0u8, |s, &b| s.wrapping_add(b));
                expected[start + 127] = sum.wrapping_neg();
            }
        }
        assert_eq!(count, 6);
        assert_eq!(upg.flash_view(&block), expected);
        for (at, ext) in edid::find(&expected) {
            assert!(edid::checksums_ok(&expected[at..at + 128 * (ext + 1)]));
        }
    }
    upg.fix_checksums();
    upg.require_valid_checksums().unwrap();
}

#[test]
#[ignore = "requires AWTOOL_STOCK_UPG"]
fn wrong_second_panel_freesync_range_is_an_atomic_error() {
    let mut upg = stock();
    let block = upg.blocks()[1].clone();
    let mut view = upg.flash_view(&block);
    let (at, ext) = edid::find(&view)
        .into_iter()
        .find(|&(at, ext)| ext == 2 && view[at + 20] & 15 == 5)
        .unwrap();
    let e = &mut view[at..at + 128 * (ext + 1)];
    let hit = e
        .windows(7)
        .position(|s| s == [0x1a, 0, 0, 3, 1, 48, 165])
        .unwrap();
    e[hit + 13] = 164;
    edid::fix_block_checksums(e);
    upg.replace_flash_view(&block, &view).unwrap();
    upg.fix_checksums();
    let before = upg.bytes().to_vec();
    let config = PatchConfig::parse(PROFILE_175HZ_FREESYNC).unwrap();
    assert!(apply_patch(&mut upg, &config).is_err());
    assert_eq!(upg.bytes(), before);
}

#[test]
#[ignore = "requires AWTOOL_STOCK_UPG"]
fn missing_second_panel_anchor_leaves_both_panels_unchanged() {
    let mut upg = stock();
    let block = upg.blocks()[1].clone();
    let mut view = upg.flash_view(&block);
    let anchor = hex_literal::hex!("947290D24DE09406");
    let at = view
        .windows(anchor.len())
        .position(|w| w == anchor)
        .unwrap();
    view[at] = 0;
    upg.replace_flash_view(&block, &view).unwrap();
    upg.fix_checksums();
    let before = upg.bytes().to_vec();
    assert!(apply_patch(&mut upg, &PatchConfig::parse(PROFILE_175HZ).unwrap()).is_err());
    assert_eq!(upg.bytes(), before);
}

#[test]
#[ignore = "requires AWTOOL_STOCK_UPG"]
fn each_mode_keeps_its_geometry_and_slot_count_mismatch_is_an_error() {
    let mut config = PatchConfig::parse(PROFILE_175HZ).unwrap();
    config.mode[0].ha = 3200;
    let mut upg = stock();
    apply_patch(&mut upg, &config).unwrap();
    for b in upg.blocks() {
        let view = upg.flash_view(&b);
        for (at, ext) in edid::find(&view) {
            let e = &view[at..at + 128 * (ext + 1)];
            assert!(edid::checksums_ok(e));
            let slots: Vec<_> = edid::timings(e)
                .into_iter()
                .filter(|t| t.kind == edid::Kind::DisplayId1)
                .collect();
            if slots.is_empty() {
                continue;
            }
            assert_eq!(slots[0].ha, 3200);
            assert_eq!(slots[1].ha, 3440);
            assert_eq!(slots[2].ha, 3440);
        }
    }
    config.mode.pop();
    let mut upg = stock();
    let before = upg.bytes().to_vec();
    assert!(apply_patch(&mut upg, &config).is_err());
    assert_eq!(upg.bytes(), before);
}

#[test]
#[ignore = "requires AWTOOL_STOCK_UPG"]
fn output_policy_changes_only_traced_immediates_in_both_panels() {
    let mut upg = stock();
    let original = stock();
    let config = PatchConfig::parse(
        "[refresh]\ncap_hz = 175\n[output]\nnominal_hz = 170\nclock_nominal_khz = 1077576\nclock_min_khz = 1000000\nclock_max_khz = 1083685",
    ).unwrap();
    assert_eq!(apply_patch(&mut upg, &config).unwrap(), 32);
    let mut refresh_only = stock();
    apply_patch(
        &mut refresh_only,
        &PatchConfig::parse("[refresh]\ncap_hz = 175").unwrap(),
    )
    .unwrap();
    for b in upg.blocks() {
        let before = refresh_only.flash_view(&b);
        let after = upg.flash_view(&b);
        let mut expected = before.clone();
        // Independently list addresses/values from the disassembly, not the patch table.
        for (high, low, value) in [
            (0x009267, 0x00926A, 1750u16),
            (0x00E43A, 0x00E43E, 1750),
            (0x19E498, 0x19E49A, 1700),
        ] {
            let bytes = value.to_be_bytes();
            expected[high] = bytes[0];
            expected[low] = bytes[1];
        }
        for (offset, value) in [
            (0x009285, 1_077_576u32),
            (0x00E459, 1_077_576),
            (0x00928F, 1_083_685),
            (0x00E463, 1_083_685),
            (0x009299, 1_000_000),
            (0x00E46D, 1_000_000),
        ] {
            expected[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        }
        assert_eq!(after, expected, "panel {}", b.panel);
        // The other getter case (60 Hz), OSD value, totals and floors stay original.
        for (offset, length) in [(0x19E49C, 5), (0x18EA65, 4), (0x14D2CD, 13), (0x1B69D9, 17)] {
            assert_eq!(
                &after[offset..offset + length],
                &original.flash_view(&b)[offset..offset + length]
            );
        }
    }
    upg.fix_checksums();
    upg.require_valid_checksums().unwrap();
    // Container metadata, record offsets, signatures and keys must be unchanged.
    let mut restored = stock();
    for b in restored.blocks() {
        restored
            .replace_flash_view(&b, &upg.flash_view(&b))
            .unwrap();
    }
    restored.fix_checksums();
    assert_eq!(upg.bytes(), restored.bytes());
}

#[test]
#[ignore = "requires AWTOOL_STOCK_UPG"]
fn wrong_second_panel_output_site_leaves_both_panels_unchanged() {
    let mut upg = stock();
    let b = upg.blocks()[1].clone();
    let mut view = upg.flash_view(&b);
    view[0x00E467] ^= 1;
    upg.replace_flash_view(&b, &view).unwrap();
    upg.fix_checksums();
    let before = upg.bytes().to_vec();
    let config = PatchConfig::parse(include_str!("testdata/output175.toml")).unwrap();
    assert!(apply_patch(&mut upg, &config).is_err());
    assert_eq!(upg.bytes(), before);
}

#[test]
#[ignore = "requires AWTOOL_STOCK_UPG"]
fn experimental_profile_has_valid_edids_and_no_mode_above_175_hz() {
    let mut upg = stock();
    let config = PatchConfig::parse(include_str!("testdata/output175.toml")).unwrap();
    assert_eq!(apply_patch(&mut upg, &config).unwrap(), 212);
    for b in upg.blocks() {
        let view = upg.flash_view(&b);
        let mut did_slots = 0;
        for (at, ext) in edid::find(&view) {
            let e = &view[at..at + 128 * (ext + 1)];
            assert!(edid::checksums_ok(e));
            for t in edid::timings(e) {
                if t.kind != edid::Kind::DisplayId1 {
                    continue;
                }
                did_slots += 1;
                let total =
                    (u64::from(t.ha) + u64::from(t.hb)) * (u64::from(t.va) + u64::from(t.vb));
                assert!(t.pclk <= 175 * total);
            }
        }
        assert!(did_slots > 0);
    }
}
