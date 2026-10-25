//! Parsers for modem AT responses (3GPP TS 27.005 / 27.007 plus Quectel conventions).

/// Rest of the first line in `resp` that starts with `prefix`, trimmed.
fn line_after<'a>(resp: &'a str, prefix: &str) -> Option<&'a str> {
    resp.lines().map(str::trim).find_map(|l| l.strip_prefix(prefix)).map(str::trim)
}

/// All `"quoted"` fields of one line, in order.
fn quoted_fields(line: &str) -> Vec<&str> {
    line.split('"').skip(1).step_by(2).collect()
}

/// `+CSQ: <rssi>,<ber>` → signal in dBm.
pub fn parse_csq(resp: &str) -> Option<i32> {
    let rssi: i32 = line_after(resp, "+CSQ:")?.split(',').next()?.trim().parse().ok()?;
    (0..=31).contains(&rssi).then(|| -113 + 2 * rssi)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operator {
    pub name: String,
    /// 3GPP access technology (`<AcT>`).
    pub act: Option<i32>,
}

/// `+COPS: <mode>,<format>,"<oper>",<AcT>`
pub fn parse_cops(resp: &str) -> Option<Operator> {
    let rest = line_after(resp, "+COPS:")?;
    let mut parts = rest.splitn(3, '"');
    let _ = parts.next()?;
    let name = parts.next()?.to_string();
    let act = parts.next().and_then(|tail| tail.trim_start_matches(',').trim().parse().ok());
    Some(Operator { name, act })
}

/// Human-readable name for a 3GPP `<AcT>` value.
pub fn rat_name(act: Option<i32>) -> &'static str {
    match act {
        Some(0 | 1 | 3) => "2G",
        Some(2 | 4 | 5 | 6) => "3G",
        Some(7 | 9) => "LTE",
        Some(10) => "LTE (5GC)",
        Some(11 | 12) => "5G SA",
        Some(13) => "5G NSA",
        _ => "—",
    }
}

/// `+C5GREG: <n>,<stat>` / `+CEREG: <n>,<stat>` → `<stat>`.
pub fn parse_registration(resp: &str, prefix: &str) -> Option<u8> {
    line_after(resp, prefix)?.split(',').nth(1)?.trim().parse().ok()
}

/// Registered on the home network (1) or roaming (5).
pub fn is_registered(stat: u8) -> bool {
    matches!(stat, 1 | 5)
}

/// `+CPIN: <code>`
pub fn parse_cpin(resp: &str) -> Option<&str> {
    line_after(resp, "+CPIN:")
}

/// `+CPMS: "SM",<used>,<total>,...` → (used, total) of the read storage.
pub fn parse_cpms(resp: &str) -> Option<(u32, u32)> {
    let mut parts = line_after(resp, "+CPMS:")?.split(',');
    let _mem = parts.next()?;
    let used = parts.next()?.trim().parse().ok()?;
    let total = parts.next()?.trim().parse().ok()?;
    Some((used, total))
}

/// `+CMTI: "SM",<index>` (new SMS stored) → index.
pub fn parse_cmti(line: &str) -> Option<u32> {
    line.trim().strip_prefix("+CMTI:")?.rsplit(',').next()?.trim().parse().ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClccEntry {
    pub id: u8,
    pub outgoing: bool,
    pub stat: u8,
    pub voice: bool,
    pub number: String,
}

/// `+CLCC: <id>,<dir>,<stat>,<mode>,<mpty>[,"<number>",<type>]` (one line per call).
pub fn parse_clcc(resp: &str) -> Vec<ClccEntry> {
    resp.lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("+CLCC:")?;
            let nums: Vec<u8> = rest.split(',').take(5).map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
            if nums.len() < 5 {
                return None;
            }
            Some(ClccEntry {
                id: nums[0],
                outgoing: nums[1] == 0,
                stat: nums[2],
                voice: nums[3] == 0,
                number: quoted_fields(rest).first().map(|s| s.to_string()).unwrap_or_default(),
            })
        })
        .collect()
}

/// An SMS exactly as `AT+CMGR` returned it (fields may still be UCS2 hex).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSms {
    pub stat: String,
    pub sender: String,
    pub timestamp: String,
    pub body: String,
}

/// `+CMGR: "<stat>","<oa>",[<alpha>],"<scts>"` followed by the body line.
/// Returns `None` for an empty storage slot.
pub fn parse_cmgr(resp: &str) -> Option<RawSms> {
    let mut lines = resp.lines().map(str::trim).skip_while(|l| !l.starts_with("+CMGR:"));
    let header = lines.next()?;
    let body = lines.find(|l| !l.is_empty()).filter(|l| *l != "OK").unwrap_or("");
    let fields = quoted_fields(header);
    if fields.len() < 2 {
        return None;
    }
    Some(RawSms {
        stat: fields[0].to_string(),
        sender: fields[1].to_string(),
        timestamp: if fields.len() >= 3 { fields[fields.len() - 1].to_string() } else { String::new() },
        body: body.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csq() {
        assert_eq!(parse_csq("\r\n+CSQ: 20,99\r\n\r\nOK\r\n"), Some(-73));
        assert_eq!(parse_csq("+CSQ: 99,99\r\nOK"), None);
    }

    #[test]
    fn cops() {
        assert_eq!(
            parse_cops("+COPS: 0,0,\"T-Mobile\",11\r\nOK"),
            Some(Operator { name: "T-Mobile".into(), act: Some(11) })
        );
        assert_eq!(parse_cops("+COPS: 0\r\nOK"), None);
        assert_eq!(rat_name(Some(13)), "5G NSA");
        assert_eq!(rat_name(None), "—");
    }

    #[test]
    fn registration() {
        assert_eq!(parse_registration("+C5GREG: 0,1\r\nOK", "+C5GREG:"), Some(1));
        assert_eq!(parse_registration("+CEREG: 2,5,\"1A2B\",\"01A2D101\",7\r\nOK", "+CEREG:"), Some(5));
        assert!(is_registered(5) && !is_registered(2));
    }

    #[test]
    fn storage_and_new_sms() {
        assert_eq!(parse_cpms("+CPMS: \"SM\",3,50,\"SM\",3,50,\"SM\",3,50\r\nOK"), Some((3, 50)));
        assert_eq!(parse_cmti("+CMTI: \"SM\",7"), Some(7));
        assert_eq!(parse_cmti("RING"), None);
        assert_eq!(parse_cpin("+CPIN: READY\r\nOK"), Some("READY"));
    }

    #[test]
    fn clcc() {
        let resp = "\r\n+CLCC: 1,1,4,0,0,\"+15551234567\",145\r\n+CLCC: 2,0,1,0,0,\"5550000\",129\r\n\r\nOK\r\n";
        let calls = parse_clcc(resp);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], ClccEntry { id: 1, outgoing: false, stat: 4, voice: true, number: "+15551234567".into() });
        assert!(calls[1].outgoing);
        assert!(parse_clcc("\r\nOK\r\n").is_empty());
    }

    #[test]
    fn cmgr() {
        let resp = "\r\n+CMGR: \"REC UNREAD\",\"002B0031\",,\"25/10/25,14:03:11+32\"\r\n00480069\r\n\r\nOK\r\n";
        assert_eq!(
            parse_cmgr(resp),
            Some(RawSms {
                stat: "REC UNREAD".into(),
                sender: "002B0031".into(),
                timestamp: "25/10/25,14:03:11+32".into(),
                body: "00480069".into(),
            })
        );
        assert_eq!(parse_cmgr("\r\nOK\r\n"), None);
    }
}
