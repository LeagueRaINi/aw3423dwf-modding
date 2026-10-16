//! Declarative patch specification, loaded from TOML (`--config`) or the embedded default.
//!
//! This holds the *user-tunable* data for `awtool patch`: which product IDs to remap, the DisplayID
//! modes to bake into the EDID, the monitor-name suffix, the EDID range limit and the firmware refresh
//! ceiling. The firmware byte-anchors for the clamp/validator/cap-store code sites are NOT here — those
//! are reverse-engineered firmware structure (see `firmware` module), not something a user should edit.

use serde::Deserialize;

/// The hardware-tested 175 Hz profile, used as the reference config in tests.
#[cfg(test)]
pub const PROFILE_175HZ: &str = include_str!("../profiles/175hz.toml");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchConfig {
    pub pid: Option<PidRemap>,
    pub name: Option<NamePatch>,
    pub range: Option<RangePatch>,
    pub refresh: Option<RefreshPatch>,
    pub output: Option<OutputPatch>,
    pub panel: Option<PanelPatch>,
    pub freesync: Option<FreeSyncPatch>,
    pub input: Option<InputPatch>,
    #[serde(default)]
    pub mode: Vec<Mode>,
}

/// Remap `count` consecutive product IDs starting at `from` to the block starting at `to`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PidRemap {
    pub from: u16,
    pub to: u16,
    pub count: u16,
}

impl PidRemap {
    pub fn remap(&self, pid: u16) -> Option<u16> {
        let delta = pid.checked_sub(self.from)?;
        (delta < self.count)
            .then(|| self.to.checked_add(delta))
            .flatten()
    }
}

/// Append a suffix to the 13-char monitor-name descriptor (e.g. "AW3423DWF" + "-CF").
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamePatch {
    pub base: String,
    pub suffix: String,
}

/// Change the EDID range descriptor's maximum vertical frequency; other VRR/timing fields are separate.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangePatch {
    pub max_vhz: u8,
    pub max_h_khz: Option<u16>,
    pub max_pclk_mhz: Option<u16>,
}

/// Input horizontal-rate validator; separate from panel output clock policy.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputPatch {
    pub max_h_khz: u16,
}

/// AMD version-3 FreeSync range in the six stock DP templates per panel.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreeSyncPatch {
    pub dp_max_hz: u8,
}

/// Raise seven known input/clamp/D5D3 cap anchors, in 0.1 Hz units internally.
/// With `[output]`, also supplies the maximum for both D60E descriptor constructors.
/// `cap_hz` is whole Hz, not a claim of supported physical panel refresh.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefreshPatch {
    pub cap_hz: u16,
}

/// Explicit output policy for the statically traced D60E descriptor and timing getter.
/// Clock units are inferred kHz; representability does not establish PLL/panel support.
/// All fields are required to avoid silently choosing fallback or clock behavior.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputPatch {
    pub nominal_hz: u16,
    pub clock_nominal_khz: u32,
    pub clock_min_khz: u32,
    pub clock_max_khz: u32,
}

impl OutputPatch {
    pub fn validate(&self, cap_hz: u16) -> Result<(), String> {
        if cap_hz.checked_mul(10).is_none() || !(47..=cap_hz).contains(&self.nominal_hz) {
            return Err("output.nominal_hz must be at least the stock 47 Hz minimum and no greater than refresh.cap_hz (maximum 6553 Hz)".into());
        }
        if self.clock_min_khz == 0
            || self.clock_min_khz > self.clock_nominal_khz
            || self.clock_nominal_khz > self.clock_max_khz
        {
            return Err("output clocks must satisfy 0 < clock_min_khz <= clock_nominal_khz <= clock_max_khz".into());
        }
        Ok(())
    }
}

/// Panel output timing floors. `dv_total_min` replaces the stock 1708-line vertical-total floor;
/// shorter vertical blanking than stock is not established as safe or supported by the panel.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelPatch {
    pub dv_total_min: u16,
}

/// A DisplayID Type-I timing to write into one of the three DID1 slots (in declared order).
/// Geometry is encoded per mode. Pixel clocks are quantized down to DisplayID's 10 kHz units.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mode {
    pub label: String,
    pub pclk_khz: u32,
    pub ha: u16,
    pub hfp: u16,
    pub hsw: u16,
    pub hbl: u16,
    pub va: u16,
    pub vfp: u16,
    pub vsw: u16,
    pub vbl: u16,
}

impl PatchConfig {
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let config: Self = toml::from_str(text).map_err(|e| format!("config parse error: {e}"))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(r) = &self.range {
            if r.max_h_khz.is_some_and(|v| !(285..=300).contains(&v)) {
                return Err("range.max_h_khz must be in 285..=300 for this experiment".into());
            }
            if r.max_pclk_mhz
                .is_some_and(|v| !(1020..=1050).contains(&v) || v % 10 != 0)
            {
                return Err("range.max_pclk_mhz must be a multiple of 10 in 1020..=1050".into());
            }
        }
        if let Some(i) = &self.input {
            if !(285..=300).contains(&i.max_h_khz) {
                return Err("input.max_h_khz must be in 285..=300 for this experiment".into());
            }
            if let Some(h) = self.range.as_ref().and_then(|r| r.max_h_khz)
                && h != i.max_h_khz
            {
                return Err("input.max_h_khz must match range.max_h_khz".into());
            }
        }
        if let Some(f) = &self.freesync {
            if f.dp_max_hz < 48 {
                return Err(
                    "freesync.dp_max_hz must be at least the stock minimum of 48 Hz".into(),
                );
            }
            if let Some(r) = &self.range
                && f.dp_max_hz != r.max_vhz
            {
                return Err(
                    "freesync.dp_max_hz must match range.max_vhz when both are specified".into(),
                );
            }
        }
        if let Some(p) = &self.pid
            && (p.count == 0
                || u32::from(p.from) + u32::from(p.count) > 65536
                || u32::from(p.to) + u32::from(p.count) > 65536)
        {
            return Err(
                "pid: count must be nonzero and source/destination ranges must fit u16".into(),
            );
        }
        if let Some(n) = &self.name {
            let name = format!("{}{}", n.base, n.suffix);
            if n.base.is_empty()
                || name.len() > 12
                || !name.bytes().all(|b| (0x20..=0x7E).contains(&b))
            {
                return Err("name: require a nonempty base and at most 12 printable ASCII bytes including suffix".into());
            }
        }
        if let Some(r) = &self.range
            && r.max_vhz < 48
        {
            return Err("range.max_vhz must be at least the stock minimum of 48 Hz".into());
        }
        if let Some(r) = &self.refresh
            && (r.cap_hz == 0 || r.cap_hz.checked_mul(10).is_none())
        {
            return Err(
                "refresh.cap_hz must be in 1..=6553 (firmware uses u16 tenths of Hz)".into(),
            );
        }
        if let Some(output) = &self.output {
            let refresh = self.refresh.as_ref().ok_or(
                "[output] requires [refresh]; refresh.cap_hz sets all known maximum-refresh sites",
            )?;
            output.validate(refresh.cap_hz)?;
        }
        if let Some(p) = &self.panel
            && !(1456..=1708).contains(&p.dv_total_min)
        {
            return Err("panel.dv_total_min must be in 1456..=1708 (stock floor 1708)".into());
        }
        for m in &self.mode {
            let fields = [m.ha, m.hfp, m.hsw, m.hbl, m.va, m.vfp, m.vsw, m.vbl];
            if fields.contains(&0)
                || m.hfp > 32768
                || m.vfp > 32768
                || u32::from(m.hfp) + u32::from(m.hsw) > u32::from(m.hbl)
                || u32::from(m.vfp) + u32::from(m.vsw) > u32::from(m.vbl)
                || !(10..=167_772_160).contains(&m.pclk_khz)
            {
                return Err(format!(
                    "mode '{}': invalid DisplayID geometry or pixel clock",
                    m.label
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_profiles_are_valid() {
        for text in [
            PROFILE_175HZ,
            include_str!("../profiles/175hz-freesync.toml"),
            include_str!("../profiles/177hz-freesync.toml"),
        ] {
            PatchConfig::parse(text).unwrap();
        }
    }

    #[test]
    fn rejects_typos_overflow_and_unencodable_geometry() {
        for text in [
            "[refresh]\ncap_hzz = 175",
            "[refresh]\ncap_hz = 6554",
            "[pid]\nfrom = 65535\nto = 0\ncount = 2",
            "[pid]\nfrom = 0\nto = 65535\ncount = 2",
            "[name]\nbase = 'AW3423DWF'\nsuffix = '💥'",
            "[range]\nmax_vhz = 0",
        ] {
            assert!(PatchConfig::parse(text).is_err(), "{text}");
        }
        let mut config = PatchConfig::parse(PROFILE_175HZ).unwrap();
        config.mode[0].hfp = 0;
        assert!(config.validate().is_err());
        config.mode[0].hfp = 32769;
        assert!(config.validate().is_err());
    }

    #[test]
    fn pid_range_can_include_u16_max_without_wrapping() {
        let p = PidRemap {
            from: 65535,
            to: 65535,
            count: 1,
        };
        assert_eq!(p.remap(65535), Some(65535));
        assert_eq!(p.remap(0), None);
    }

    #[test]
    fn experimental_output_profile_is_valid_and_requires_explicit_policy() {
        let text = include_str!("testdata/output175.toml");
        let mut config = PatchConfig::parse(text).unwrap();
        assert_eq!(config.refresh.as_ref().unwrap().cap_hz, 175);
        for (from, to) in [
            ("[refresh]\ncap_hz = 175", ""),
            ("nominal_hz = 175", "nominal_hz = 176"),
            ("nominal_hz = 175", "nominal_hz = 46"),
            ("clock_nominal_khz = 1077576", ""),
            ("clock_min_khz = 1010205", "clock_min_khz = 0"),
            ("clock_min_khz = 1010205", "clock_min_khz = 1077577"),
            ("clock_max_khz = 1083685", "clock_max_khz = 1077575"),
            ("clock_max_khz = 1083685", "clock_max_khz = 4294967296"),
        ] {
            assert!(
                PatchConfig::parse(&text.replace(from, to)).is_err(),
                "{from} -> {to}"
            );
        }
        // A raised ceiling may intentionally retain the original nominal fallback.
        config.output.as_mut().unwrap().nominal_hz = 165;
        config.validate().unwrap();
    }
}
