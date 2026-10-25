//! Carrier detection from the SIM's IMSI (MCC + MNC) to pick a data APN automatically,
//! so the same firmware image works with common SIMs without rebuilding.

/// The build-time APN that means "not configured": auto-detection may replace it.
pub const DEFAULT_APN: &str = "internet";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Carrier {
    pub name: &'static str,
    pub apn: &'static str,
}

/// (MCC, MNC, carrier). Mainland China (MCC 460) for now; extend as SIMs get tested.
const CARRIERS: &[(&str, &str, Carrier)] = &[
    ("460", "00", CHINA_MOBILE),
    ("460", "02", CHINA_MOBILE),
    ("460", "04", CHINA_MOBILE),
    ("460", "07", CHINA_MOBILE),
    ("460", "08", CHINA_MOBILE),
    ("460", "01", CHINA_UNICOM),
    ("460", "06", CHINA_UNICOM),
    ("460", "09", CHINA_UNICOM),
    ("460", "03", CHINA_TELECOM),
    ("460", "05", CHINA_TELECOM),
    ("460", "11", CHINA_TELECOM),
    ("460", "15", CHINA_BROADNET),
];

const CHINA_MOBILE: Carrier = Carrier { name: "China Mobile", apn: "cmnet" };
const CHINA_UNICOM: Carrier = Carrier { name: "China Unicom", apn: "3gnet" };
const CHINA_TELECOM: Carrier = Carrier { name: "China Telecom", apn: "ctnet" };
const CHINA_BROADNET: Carrier = Carrier { name: "China Broadnet", apn: "cbnet" };

/// `AT+CIMI` answers with the bare IMSI (15 digits) on its own line.
pub fn parse_cimi(resp: &str) -> Option<&str> {
    resp.lines().map(str::trim).find(|l| (6..=15).contains(&l.len()) && l.bytes().all(|b| b.is_ascii_digit()))
}

pub fn carrier_for_imsi(imsi: &str) -> Option<Carrier> {
    CARRIERS.iter().find(|(mcc, mnc, _)| imsi.starts_with(mcc) && imsi[mcc.len()..].starts_with(mnc)).map(|e| e.2)
}

/// An explicitly configured APN always wins; otherwise use the detected carrier's APN.
pub fn choose_apn<'a>(configured: &'a str, imsi: Option<&str>) -> &'a str {
    if configured != DEFAULT_APN {
        return configured;
    }
    imsi.and_then(carrier_for_imsi).map(|c| c.apn).unwrap_or(configured)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cimi() {
        assert_eq!(parse_cimi("\r\n460001234567890\r\n\r\nOK\r\n"), Some("460001234567890"));
        assert_eq!(parse_cimi("\r\n+CME ERROR: 10\r\n"), None);
        assert_eq!(parse_cimi("\r\nOK\r\n"), None);
    }

    #[test]
    fn detects_chinese_carriers() {
        assert_eq!(carrier_for_imsi("460001234567890").map(|c| c.apn), Some("cmnet"));
        assert_eq!(carrier_for_imsi("460011234567890").map(|c| c.name), Some("China Unicom"));
        assert_eq!(carrier_for_imsi("460111234567890").map(|c| c.apn), Some("ctnet"));
        assert_eq!(carrier_for_imsi("460151234567890").map(|c| c.apn), Some("cbnet"));
        assert_eq!(carrier_for_imsi("310260123456789"), None);
        assert_eq!(carrier_for_imsi("4601"), None);
    }

    #[test]
    fn explicit_apn_wins() {
        assert_eq!(choose_apn("internet", Some("460031234567890")), "ctnet");
        assert_eq!(choose_apn("my.apn", Some("460031234567890")), "my.apn");
        assert_eq!(choose_apn("internet", Some("310260123456789")), "internet");
        assert_eq!(choose_apn("internet", None), "internet");
    }
}
