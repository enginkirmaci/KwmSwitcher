//! Monitor input-source / PiP VCP codes and the per-protocol wire mappings
//! (standard DDC/CI vs LG's vendor-specific registers).

use crate::config::InputSwitchProtocol;

pub const DISPLAY_PORT: u8 = 0x0F;
pub const HDMI1: u8 = 0x11;
pub const HDMI2: u8 = 0x12;
pub const VGA: u8 = 0x01;
pub const DVI: u8 = 0x03;
pub const USB_C: u8 = 0x20;

/// VCP feature code for the input source (standard DDC/CI).
pub const INPUT_VCP_CODE: u8 = 0x60;
/// VCP feature code for PiP/PBP mode (standard DDC/CI).
pub const PIP_VCP_CODE: u8 = 0xCC;

const LG_INPUT_VCP_CODE: u8 = 0xF4;
const LG_PIP_VCP_CODE: u8 = 0xD7;
const LG_PIP_OFF: u8 = 0x01;
const LG_PIP_PBP: u8 = 0x05;

/// A PiP/PBP mode as the app models it.
///
/// `Unknown` keeps the raw wire value instead of falling back to a number:
/// a display answering with something unexpected shows up in the UI and the
/// log as "Unknown (0x..)", and — importantly — is *not* treated as a split
/// mode, so one stray VCP read cannot suspend automatic switching forever.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PipMode {
    #[default]
    Off,
    Pip,
    Pbp,
    Unknown(u8),
}

impl PipMode {
    /// True only for the modes this app can recognize as a split mode.
    pub fn is_active(self) -> bool {
        matches!(self, PipMode::Pip | PipMode::Pbp)
    }

    /// Short static name, for places that need `&'static str`.
    pub const fn name(self) -> &'static str {
        match self {
            PipMode::Off => "Off",
            PipMode::Pip => "PiP",
            PipMode::Pbp => "PBP",
            PipMode::Unknown(_) => "Unknown",
        }
    }

    /// User-facing label, including the raw value for an unrecognized mode.
    pub fn label(self) -> String {
        match self {
            PipMode::Unknown(wire) => format!("Unknown (0x{wire:02X})"),
            other => other.name().to_string(),
        }
    }
}

pub fn input_name(code: u8) -> String {
    match code {
        DISPLAY_PORT => "DisplayPort".into(),
        HDMI1 => "HDMI-1".into(),
        HDMI2 => "HDMI-2".into(),
        USB_C => "USB-C".into(),
        VGA => "VGA".into(),
        DVI => "DVI".into(),
        other => format!("Unknown (0x{other:02X})"),
    }
}

/// The selectable input options per protocol (LG monitors hide DVI/VGA).
pub fn input_options(protocol: InputSwitchProtocol) -> Vec<(u8, &'static str)> {
    match protocol {
        InputSwitchProtocol::Lg => vec![
            (DISPLAY_PORT, "DisplayPort"),
            (USB_C, "USB-C"),
            (HDMI1, "HDMI-1"),
            (HDMI2, "HDMI-2"),
        ],
        InputSwitchProtocol::Standard => vec![
            (DISPLAY_PORT, "DisplayPort"),
            (HDMI1, "HDMI-1"),
            (HDMI2, "HDMI-2"),
            (DVI, "DVI"),
            (VGA, "VGA"),
        ],
    }
}

pub fn input_vcp_code(protocol: InputSwitchProtocol) -> u8 {
    match protocol {
        InputSwitchProtocol::Lg => LG_INPUT_VCP_CODE,
        InputSwitchProtocol::Standard => INPUT_VCP_CODE,
    }
}

pub fn pip_vcp_code(protocol: InputSwitchProtocol) -> u8 {
    match protocol {
        InputSwitchProtocol::Lg => LG_PIP_VCP_CODE,
        InputSwitchProtocol::Standard => PIP_VCP_CODE,
    }
}

/// Whether PiP/PBP control is implemented for this protocol.
///
/// It is only implemented for LG's vendor register `0xD7`. Standard DDC/CI
/// code `0xCC` is the MCCS "OSD Language" feature: reading it returns the
/// selected language and writing it would change the monitor's OSD language,
/// not its PiP mode. Treating that value as a PiP mode would also suspend
/// automatic switching whenever the language is non-zero.
pub fn pip_supported(protocol: InputSwitchProtocol) -> bool {
    protocol == InputSwitchProtocol::Lg
}

/// Whether an input write may be verified by reading VCP `0x60` afterwards.
///
/// Only standard DDC/CI supports this. On LG, ddcutil's own write
/// verification is disabled (`--noverify`) because any DDC traffic right after
/// the `0xF4` write makes the display abort the input switch; a read-back
/// reintroduces exactly that failure, so the LG write is trusted as-is.
pub fn input_verification_supported(protocol: InputSwitchProtocol) -> bool {
    protocol == InputSwitchProtocol::Standard
}

/// Custom i2c source address LG needs (standard monitors use 0x00 = default).
pub fn input_i2c_source_addr(protocol: InputSwitchProtocol) -> u8 {
    match protocol {
        InputSwitchProtocol::Lg => 0x50,
        InputSwitchProtocol::Standard => 0x00,
    }
}

pub fn pip_i2c_source_addr(protocol: InputSwitchProtocol) -> u8 {
    match protocol {
        InputSwitchProtocol::Lg => 0x51,
        InputSwitchProtocol::Standard => 0x00,
    }
}

/// Maps a logical input code onto the protocol's wire value.
pub fn encode_input(protocol: InputSwitchProtocol, logical: u8) -> u8 {
    match protocol {
        InputSwitchProtocol::Lg => match logical {
            DISPLAY_PORT => 0xD0,
            USB_C => 0xD1,
            HDMI1 => 0x90,
            HDMI2 => 0x91,
            other => other,
        },
        InputSwitchProtocol::Standard => logical,
    }
}

/// Reverse of [`encode_input`].
pub fn decode_input(protocol: InputSwitchProtocol, wire: u8) -> u8 {
    match protocol {
        InputSwitchProtocol::Lg => match wire {
            0xD0 => DISPLAY_PORT,
            0xD1 => USB_C,
            0x90 => HDMI1,
            0x91 => HDMI2,
            other => other,
        },
        InputSwitchProtocol::Standard => wire,
    }
}

/// The wire value for setting `mode`, or `None` when the mode is not one the
/// app can encode (an unrecognized read-back must not be written back).
pub fn encode_pip(protocol: InputSwitchProtocol, mode: PipMode) -> Option<u8> {
    match protocol {
        InputSwitchProtocol::Lg => match mode {
            PipMode::Off => Some(LG_PIP_OFF),
            // LG reports PiP and PBP as the same wire value.
            PipMode::Pip | PipMode::Pbp => Some(LG_PIP_PBP),
            PipMode::Unknown(_) => None,
        },
        InputSwitchProtocol::Standard => match mode {
            PipMode::Off => Some(0x00),
            PipMode::Pip => Some(0x01),
            PipMode::Pbp => Some(0x02),
            PipMode::Unknown(_) => None,
        },
    }
}

/// Decodes a PiP wire value. Values outside the known set become
/// [`PipMode::Unknown`] with the raw value kept for diagnostics; they are not
/// guessed into PiP/PBP.
pub fn decode_pip(protocol: InputSwitchProtocol, wire: u8) -> PipMode {
    match protocol {
        InputSwitchProtocol::Lg => match wire {
            LG_PIP_OFF => PipMode::Off,
            LG_PIP_PBP => PipMode::Pbp,
            other => PipMode::Unknown(other),
        },
        InputSwitchProtocol::Standard => match wire {
            0x00 => PipMode::Off,
            0x01 => PipMode::Pip,
            0x02 => PipMode::Pbp,
            other => PipMode::Unknown(other),
        },
    }
}

/// Normalizes a mode to its canonical logical value for this protocol
/// (LG reports PiP and PBP as the same wire value, so a request for PiP comes
/// back as PBP on LG).
pub fn canonicalize_pip(protocol: InputSwitchProtocol, mode: PipMode) -> PipMode {
    encode_pip(protocol, mode).map_or(mode, |wire| decode_pip(protocol, wire))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_roundtrips() {
        for code in [DISPLAY_PORT, HDMI1, HDMI2, DVI, VGA] {
            assert_eq!(
                decode_input(
                    InputSwitchProtocol::Standard,
                    encode_input(InputSwitchProtocol::Standard, code)
                ),
                code
            );
        }
        assert_eq!(input_vcp_code(InputSwitchProtocol::Standard), 0x60);
        assert_eq!(input_i2c_source_addr(InputSwitchProtocol::Standard), 0x00);
    }

    #[test]
    fn lg_roundtrips() {
        for code in [DISPLAY_PORT, USB_C, HDMI1, HDMI2] {
            assert_eq!(
                decode_input(
                    InputSwitchProtocol::Lg,
                    encode_input(InputSwitchProtocol::Lg, code)
                ),
                code
            );
        }
        assert_eq!(encode_input(InputSwitchProtocol::Lg, DISPLAY_PORT), 0xD0);
        assert_eq!(input_vcp_code(InputSwitchProtocol::Lg), 0xF4);
        assert_eq!(input_i2c_source_addr(InputSwitchProtocol::Lg), 0x50);
    }

    #[test]
    fn standard_pip_decode_keeps_unknown_values_unknown() {
        assert_eq!(
            decode_pip(InputSwitchProtocol::Standard, 0x00),
            PipMode::Off
        );
        assert_eq!(
            decode_pip(InputSwitchProtocol::Standard, 0x01),
            PipMode::Pip
        );
        assert_eq!(
            decode_pip(InputSwitchProtocol::Standard, 0x02),
            PipMode::Pbp
        );
        assert_eq!(
            decode_pip(InputSwitchProtocol::Standard, 0x03),
            PipMode::Unknown(0x03)
        );
    }

    #[test]
    fn lg_pip_values_do_not_collide_with_logical_modes() {
        assert_eq!(decode_pip(InputSwitchProtocol::Lg, 0x01), PipMode::Off);
        assert_eq!(decode_pip(InputSwitchProtocol::Lg, 0x05), PipMode::Pbp);
        // 0x02 equals the *standard* PBP code; on LG it must not be guessed.
        assert_eq!(
            decode_pip(InputSwitchProtocol::Lg, 0x02),
            PipMode::Unknown(0x02)
        );
        assert_eq!(
            decode_pip(InputSwitchProtocol::Lg, 0x00),
            PipMode::Unknown(0x00)
        );
    }

    #[test]
    fn only_known_split_modes_are_active() {
        assert!(!PipMode::Off.is_active());
        assert!(PipMode::Pip.is_active());
        assert!(PipMode::Pbp.is_active());
        assert!(!PipMode::Unknown(0x03).is_active());
    }

    #[test]
    fn unknown_modes_are_not_encodable() {
        assert_eq!(
            encode_pip(InputSwitchProtocol::Standard, PipMode::Unknown(0x09)),
            None
        );
        assert_eq!(
            encode_pip(InputSwitchProtocol::Lg, PipMode::Unknown(0x02)),
            None
        );
        assert_eq!(
            encode_pip(InputSwitchProtocol::Lg, PipMode::Off),
            Some(0x01)
        );
        assert_eq!(
            encode_pip(InputSwitchProtocol::Lg, PipMode::Pip),
            Some(0x05)
        );
        assert_eq!(
            encode_pip(InputSwitchProtocol::Lg, PipMode::Pbp),
            Some(0x05)
        );
        assert_eq!(
            encode_pip(InputSwitchProtocol::Standard, PipMode::Pbp),
            Some(0x02)
        );
    }

    #[test]
    fn canonicalize_matches_the_protocol() {
        assert_eq!(
            canonicalize_pip(InputSwitchProtocol::Standard, PipMode::Pip),
            PipMode::Pip
        );
        assert_eq!(
            canonicalize_pip(InputSwitchProtocol::Lg, PipMode::Pip),
            PipMode::Pbp
        );
        assert_eq!(
            canonicalize_pip(InputSwitchProtocol::Lg, PipMode::Off),
            PipMode::Off
        );
        assert_eq!(
            canonicalize_pip(InputSwitchProtocol::Lg, PipMode::Unknown(0x00)),
            PipMode::Unknown(0x00)
        );
    }

    #[test]
    fn unknown_labels_show_the_raw_value() {
        assert_eq!(PipMode::Pbp.label(), "PBP");
        assert_eq!(PipMode::Unknown(0x03).label(), "Unknown (0x03)");
    }

    #[test]
    fn pip_control_is_lg_only() {
        // 0xCC is OSD Language on standard monitors, not a PiP mode.
        assert!(!pip_supported(InputSwitchProtocol::Standard));
        assert!(pip_supported(InputSwitchProtocol::Lg));
    }

    #[test]
    fn input_verification_is_standard_only() {
        // Reading VCP 0x60 right after the LG F4 write aborts the switch.
        assert!(input_verification_supported(InputSwitchProtocol::Standard));
        assert!(!input_verification_supported(InputSwitchProtocol::Lg));
    }
}
