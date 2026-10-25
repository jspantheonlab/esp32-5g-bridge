//! Battery fuel gauge (MAX17048) and charger (BQ25895) register decoding.
//! Register maps: MAX17048 datasheet "Register Summary", BQ25895 datasheet REG0B.

pub const MAX17048_ADDR: u8 = 0x36;
pub const MAX17048_REG_VCELL: u8 = 0x02;
pub const MAX17048_REG_SOC: u8 = 0x04;

pub const BQ25895_ADDR: u8 = 0x6A;
pub const BQ25895_REG_STATUS: u8 = 0x0B;

/// Below this state of charge (and not on USB power) the bridge shuts down cleanly.
pub const CRITICAL_PERCENT: u8 = 3;

/// SOC register: MSB is whole percent, LSB is 1/256 %.
pub fn soc_percent(raw: [u8; 2]) -> u8 {
    raw[0].min(100)
}

/// VCELL register: 78.125 µV per LSB.
pub fn vcell_millivolts(raw: [u8; 2]) -> u32 {
    u32::from(u16::from_be_bytes(raw)) * 78_125 / 1_000_000
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeState {
    NotCharging,
    PreCharge,
    FastCharge,
    Done,
}

impl ChargeState {
    pub fn is_charging(self) -> bool {
        matches!(self, Self::PreCharge | Self::FastCharge)
    }
}

/// REG0B bits 4:3 (CHRG_STAT).
pub fn charge_state(reg0b: u8) -> ChargeState {
    match (reg0b >> 3) & 0b11 {
        0 => ChargeState::NotCharging,
        1 => ChargeState::PreCharge,
        2 => ChargeState::FastCharge,
        _ => ChargeState::Done,
    }
}

/// REG0B bit 2 (PG_STAT): input power good.
pub fn input_power_good(reg0b: u8) -> bool {
    reg0b & 0b100 != 0
}

pub fn is_critical(percent: u8, on_usb_power: bool) -> bool {
    percent <= CRITICAL_PERCENT && !on_usb_power
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gauge_decoding() {
        assert_eq!(soc_percent([82, 128]), 82);
        assert_eq!(soc_percent([0xFF, 0]), 100);
        // 0xD000 * 78.125 µV = 4160 mV
        assert_eq!(vcell_millivolts([0xD0, 0x00]), 4160);
    }

    #[test]
    fn charger_decoding() {
        assert_eq!(charge_state(0b0001_0100), ChargeState::FastCharge);
        assert!(charge_state(0b0000_1000).is_charging());
        assert_eq!(charge_state(0b0001_1100), ChargeState::Done);
        assert!(!charge_state(0).is_charging());
        assert!(input_power_good(0b0000_0100));
        assert!(!input_power_good(0b0001_0000));
    }

    #[test]
    fn critical_threshold() {
        assert!(is_critical(3, false));
        assert!(!is_critical(3, true));
        assert!(!is_critical(4, false));
    }
}
