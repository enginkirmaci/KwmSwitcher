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

pub const PIP_OFF: u8 = 0x00;
pub const PIP_ON: u8 = 0x01;
pub const PIP_PBP: u8 = 0x02;

const LG_INPUT_VCP_CODE: u8 = 0xF4;
const LG_PIP_VCP_CODE: u8 = 0xD7;
const LG_PIP_OFF: u8 = 0x01;
const LG_PIP_PBP: u8 = 0x05;

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

pub fn pip_mode_name(code: u8) -> String {
    match code {
        PIP_OFF => "Off".into(),
        PIP_ON => "PiP".into(),
        PIP_PBP => "PBP".into(),
        other => format!("Unknown (0x{other:02X})"),
    }
}

pub fn is_pip_active(code: u8) -> bool {
    code != PIP_OFF
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

pub fn encode_pip(protocol: InputSwitchProtocol, mode: u8) -> u8 {
    match protocol {
        InputSwitchProtocol::Lg => match mode {
            PIP_OFF => LG_PIP_OFF,
            PIP_ON | PIP_PBP => LG_PIP_PBP,
            other => other,
        },
        InputSwitchProtocol::Standard => mode,
    }
}

pub fn decode_pip(protocol: InputSwitchProtocol, wire: u8) -> u8 {
    match protocol {
        InputSwitchProtocol::Lg => match wire {
            LG_PIP_OFF => PIP_OFF,
            LG_PIP_PBP => PIP_PBP,
            other => other,
        },
        InputSwitchProtocol::Standard => wire,
    }
}

/// Normalizes a mode to its canonical logical value for this protocol
/// (LG reports PiP and PBP as the same wire value).
pub fn canonicalize_pip(protocol: InputSwitchProtocol, mode: u8) -> u8 {
    decode_pip(protocol, encode_pip(protocol, mode))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_roundtrips() {
        for code in [DISPLAY_PORT, HDMI1, HDMI2, DVI, VGA] {
            assert_eq!(decode_input(InputSwitchProtocol::Standard, encode_input(InputSwitchProtocol::Standard, code)), code);
        }
        assert_eq!(input_vcp_code(InputSwitchProtocol::Standard), 0x60);
        assert_eq!(input_i2c_source_addr(InputSwitchProtocol::Standard), 0x00);
    }

    #[test]
    fn lg_roundtrips() {
        for code in [DISPLAY_PORT, USB_C, HDMI1, HDMI2] {
            assert_eq!(decode_input(InputSwitchProtocol::Lg, encode_input(InputSwitchProtocol::Lg, code)), code);
        }
        assert_eq!(encode_input(InputSwitchProtocol::Lg, DISPLAY_PORT), 0xD0);
        assert_eq!(input_vcp_code(InputSwitchProtocol::Lg), 0xF4);
        assert_eq!(input_i2c_source_addr(InputSwitchProtocol::Lg), 0x50);
    }

    #[test]
    fn pip_mapping() {
        assert_eq!(canonicalize_pip(InputSwitchProtocol::Standard, PIP_ON), PIP_ON);
        assert_eq!(canonicalize_pip(InputSwitchProtocol::Lg, PIP_ON), PIP_PBP);
        assert_eq!(canonicalize_pip(InputSwitchProtocol::Lg, PIP_OFF), PIP_OFF);
        assert!(is_pip_active(PIP_PBP));
        assert!(!is_pip_active(PIP_OFF));
    }
}
