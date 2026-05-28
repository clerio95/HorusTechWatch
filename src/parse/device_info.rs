//! Parser for cmd 0x12 Device info response.
//!
//! Layout per DT214 §3.4.2: space-delimited positional fields up through
//! the IP address, then `;`-separated groups for the rest. Older firmware
//! truncates trailing groups, so every post-IP field is optional.

use serde::Serialize;

use super::ParseError;

#[derive(Serialize, Debug, Clone, Default)]
pub struct DeviceInfoParsed {
    pub boot_descriptor: String,
    pub boot_version: String,
    pub firmware_descriptor: String,
    pub firmware_version: String,
    pub firmware_date: String,

    pub battery_level: Option<u8>,
    pub battery_level_label: Option<String>,
    pub battery_voltage_raw: String,

    pub external_net: Option<u8>,
    pub external_net_label: Option<String>,
    pub external_net_voltage_raw: String,

    pub permissions: String,
    pub serial: String,
    pub manufacturing_date: String,
    pub last_valid_date: String,
    pub mac: String,
    pub ip: String,

    // `;`-separated tail; all optional, depend on firmware.
    pub cert_validity_date: Option<String>,
    pub ip_type: Option<String>,           // 'D' DHCP or 'F' Fixed
    pub active_protocol: Option<String>,   // C/c/P/D
    pub cert_logged: Option<bool>,
    pub cert_locked: Option<bool>,
    pub cert_code: Option<String>,

    pub font_type: Option<String>,
    pub icom_types: Option<String>,
    pub automation_type: Option<String>,
    pub battery_hw_status: Option<String>,
    pub battery_hw_label: Option<String>,
    pub price_change_by_idf: Option<String>,
    pub tank_meter_integration: Option<String>,

    pub meter_ip: Option<String>,
    pub meter_port: Option<String>,
    pub meter_client: Option<String>,

    pub positions: Option<String>,
    pub mqtt_ip: Option<String>,
    pub mqtt_port: Option<String>,
    pub mqtt_status: Option<String>,
    pub comm_ports: Option<String>,
    pub firmware_type: Option<String>,
    pub memory_type: Option<String>,
}

pub fn parse(payload: &[u8]) -> Result<DeviceInfoParsed, ParseError> {
    let s = std::str::from_utf8(payload).map_err(|_| ParseError::new("device_info not ASCII"))?;

    // Split on the first ';'. Everything before is the positional fixed-width prefix;
    // everything after is `;`-separated optional groups.
    let (head, tail) = match s.find(';') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    };

    let mut out = parse_head(head)?;
    parse_tail(&mut out, tail);
    Ok(out)
}

/// Parse the positional, space-delimited prefix up through the IP address.
fn parse_head(head: &str) -> Result<DeviceInfoParsed, ParseError> {
    let parts: Vec<&str> = head.split(' ').collect();
    // Expected order:
    //   0: vVV.VV (boot)
    //   1: fFF.FF (firmware)
    //   2: DD/MM/AA (firmware date)
    //   3: B (battery level)
    //   4: bbbbb (battery voltage)
    //   5: E (external net)
    //   6: eeee (external net voltage)
    //   7: C-NNNNNNNN (perms-serial)
    //   8: DD/MM/AA (mfg date)
    //   9: DD/MM/AA (last valid date)
    //  10: MAC (XX:XX:XX:XX:XX:XX)
    //  11: IP (zero-padded)
    if parts.len() < 12 {
        return Err(ParseError::new(format!(
            "device_info head has {} space-delimited fields, expected ≥12",
            parts.len()
        )));
    }

    let (boot_desc, boot_ver) = split_descriptor_version(parts[0]);
    let (fw_desc, fw_ver) = split_descriptor_version(parts[1]);

    let battery_level = parts[3].parse::<u8>().ok();
    let battery_level_label = battery_level.map(|b| match b {
        0 => "normal".to_string(),
        1 => "low".to_string(),
        2 => "critical".to_string(),
        _ => format!("unknown_{}", b),
    });

    let external_net = parts[5].parse::<u8>().ok();
    let external_net_label = external_net.map(|e| match e {
        0 => "off".to_string(),
        1 => "low".to_string(),
        2 => "normal".to_string(),
        3 => "high".to_string(),
        _ => format!("unknown_{}", e),
    });

    let (permissions, serial) = split_perms_serial(parts[7]);

    Ok(DeviceInfoParsed {
        boot_descriptor: boot_desc,
        boot_version: boot_ver,
        firmware_descriptor: fw_desc,
        firmware_version: fw_ver,
        firmware_date: parts[2].to_string(),
        battery_level,
        battery_level_label,
        battery_voltage_raw: parts[4].to_string(),
        external_net,
        external_net_label,
        external_net_voltage_raw: parts[6].to_string(),
        permissions,
        serial,
        manufacturing_date: parts[8].to_string(),
        last_valid_date: parts[9].to_string(),
        mac: parts[10].to_string(),
        ip: parts[11].to_string(),
        ..Default::default()
    })
}

fn split_descriptor_version(s: &str) -> (String, String) {
    let mut chars = s.chars();
    let desc = chars.next().map(|c| c.to_string()).unwrap_or_default();
    let rest: String = chars.collect();
    (desc, rest)
}

fn split_perms_serial(s: &str) -> (String, String) {
    if let Some((a, b)) = s.split_once('-') {
        (a.to_string(), b.to_string())
    } else {
        (s.to_string(), String::new())
    }
}

/// Parse the `;`-separated tail. Each group is optional; absent groups
/// leave their fields as `None`.
fn parse_tail(out: &mut DeviceInfoParsed, tail: &str) {
    let groups: Vec<&str> = tail.split(';').collect();

    // Group 0: "DD/MM/AA d f l t CCCCCCCC" — cert date + IP type + protocol + login + lock + cert code
    if let Some(g) = groups.first() {
        parse_cert_block(out, g);
    }

    // Group 1: "FIIIHDPT" — font + ICOM[3] + automation + battery_hw + price_idf + tank_meter
    if let Some(g) = groups.get(1) {
        parse_fiihdpt_block(out, g);
    }

    // Group 2: meter IP
    if let Some(g) = groups.get(2) {
        if !g.is_empty() {
            out.meter_ip = Some(g.to_string());
        }
    }
    // Group 3: meter port
    if let Some(g) = groups.get(3) {
        if !g.is_empty() {
            out.meter_port = Some(g.to_string());
        }
    }
    // Group 4: meter activation char
    if let Some(g) = groups.get(4) {
        if !g.is_empty() {
            out.meter_client = Some(g.to_string());
        }
    }
    // Group 5: positions (w)
    if let Some(g) = groups.get(5) {
        if !g.is_empty() {
            out.positions = Some(g.to_string());
        }
    }
    // Group 6: MQTT IP
    if let Some(g) = groups.get(6) {
        if !g.is_empty() {
            out.mqtt_ip = Some(g.to_string());
        }
    }
    // Group 7: MQTT port
    if let Some(g) = groups.get(7) {
        if !g.is_empty() {
            out.mqtt_port = Some(g.to_string());
        }
    }
    // Group 8: MQTT status
    if let Some(g) = groups.get(8) {
        if !g.is_empty() {
            out.mqtt_status = Some(g.to_string());
        }
    }
    // Group 9: available comm ports
    if let Some(g) = groups.get(9) {
        if !g.is_empty() {
            out.comm_ports = Some(g.to_string());
        }
    }
    // Group 10: firmware type + memory type — packed g[1] h[1]
    if let Some(g) = groups.get(10) {
        if g.len() >= 1 {
            out.firmware_type = Some((&g[..1]).to_string());
        }
        if g.len() >= 2 {
            out.memory_type = Some((&g[1..2]).to_string());
        }
    }
}

/// Cert block format: "DD/MM/AA dflt CCCCCCCC"
///   - 8-char cert validity date
///   - SPACE
///   - 1-char IP type (d)
///   - 1-char active protocol (f)
///   - 1-char login status (l): 'L' or ' '
///   - 1-char cert lock (t): 'T' or ' '
///   - 8-char cert code
fn parse_cert_block(out: &mut DeviceInfoParsed, g: &str) {
    if g.len() < 8 {
        return;
    }
    out.cert_validity_date = Some(g[0..8].to_string());
    let rest = &g[8..].trim_start_matches(' ');
    if rest.len() >= 4 {
        out.ip_type = Some((&rest[0..1]).to_string());
        out.active_protocol = Some((&rest[1..2]).to_string());
        out.cert_logged = Some(&rest[2..3] == "L");
        out.cert_locked = Some(&rest[3..4] == "T");
    }
    if rest.len() >= 12 {
        out.cert_code = Some(rest[4..12].to_string());
    }
}

/// FIIIHDPT block: F[1] I[3] H[1] D[1] P[1] T[1].
fn parse_fiihdpt_block(out: &mut DeviceInfoParsed, g: &str) {
    let b = g.as_bytes();
    if b.len() >= 1 {
        out.font_type = Some((b[0] as char).to_string());
    }
    if b.len() >= 4 {
        out.icom_types = Some(std::str::from_utf8(&b[1..4]).unwrap_or("").to_string());
    }
    if b.len() >= 5 {
        out.automation_type = Some((b[4] as char).to_string());
    }
    if b.len() >= 6 {
        let hw = b[5];
        out.battery_hw_status = Some((hw as char).to_string());
        out.battery_hw_label = Some(match hw {
            b'D' => "present_charging".to_string(),
            b'L' => "present_not_charging".to_string(),
            b'F' => "absent".to_string(),
            b'i' => "inverted".to_string(),
            _ => format!("unknown_{}", hw as char),
        });
    }
    if b.len() >= 7 {
        out.price_change_by_idf = Some((b[6] as char).to_string());
    }
    if b.len() >= 8 {
        out.tank_meter_integration = Some((b[7] as char).to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live 2026-05-26 reading from CLAUDE.md (payload after stripping "12").
    const LIVE: &[u8] = b"B01.00 F08.03 22/07/19 0 12,84 2 0113 3-00010427 17/01/17 26/05/26 00:26:28:11:04:27 192.168.025.091;00/00/00 Fc  00000000;c900HDNN;000.000.000.000;00000;D;";

    #[test]
    fn parses_live_device_info() {
        let p = parse(LIVE).unwrap();
        assert_eq!(p.boot_descriptor, "B");
        assert_eq!(p.boot_version, "01.00");
        assert_eq!(p.firmware_descriptor, "F");
        assert_eq!(p.firmware_version, "08.03");
        assert_eq!(p.firmware_date, "22/07/19");
        assert_eq!(p.battery_level, Some(0));
        assert_eq!(p.battery_level_label.as_deref(), Some("normal"));
        assert_eq!(p.battery_voltage_raw, "12,84");
        assert_eq!(p.external_net, Some(2));
        assert_eq!(p.external_net_label.as_deref(), Some("normal"));
        assert_eq!(p.permissions, "3");
        assert_eq!(p.serial, "00010427");
        assert_eq!(p.mac, "00:26:28:11:04:27");
        assert_eq!(p.ip, "192.168.025.091");

        // Cert block
        assert_eq!(p.cert_validity_date.as_deref(), Some("00/00/00"));
        assert_eq!(p.ip_type.as_deref(), Some("F"));
        assert_eq!(p.active_protocol.as_deref(), Some("c"));
        assert_eq!(p.cert_logged, Some(false));
        assert_eq!(p.cert_locked, Some(false));
        assert_eq!(p.cert_code.as_deref(), Some("00000000"));

        // FIIIHDPT block
        assert_eq!(p.font_type.as_deref(), Some("c"));
        assert_eq!(p.icom_types.as_deref(), Some("900"));
        assert_eq!(p.automation_type.as_deref(), Some("H"));
        assert_eq!(p.battery_hw_status.as_deref(), Some("D"));
        assert_eq!(p.battery_hw_label.as_deref(), Some("present_charging"));
        assert_eq!(p.price_change_by_idf.as_deref(), Some("N"));
        assert_eq!(p.tank_meter_integration.as_deref(), Some("N"));

        // Older firmware truncates the rest
        assert_eq!(p.meter_ip.as_deref(), Some("000.000.000.000"));
        assert_eq!(p.meter_port.as_deref(), Some("00000"));
        assert_eq!(p.meter_client.as_deref(), Some("D"));
        assert_eq!(p.positions, None);
        assert_eq!(p.mqtt_ip, None);
    }

    #[test]
    fn parses_critical_battery() {
        let mut s = String::from_utf8(LIVE.to_vec()).unwrap();
        s = s.replacen(" 0 12,84", " 2 12,84", 1);
        let p = parse(s.as_bytes()).unwrap();
        assert_eq!(p.battery_level, Some(2));
        assert_eq!(p.battery_level_label.as_deref(), Some("critical"));
    }

    #[test]
    fn parses_absent_battery_hw() {
        let mut s = String::from_utf8(LIVE.to_vec()).unwrap();
        s = s.replacen(";c900HDNN;", ";c900HFNN;", 1);
        let p = parse(s.as_bytes()).unwrap();
        assert_eq!(p.battery_hw_status.as_deref(), Some("F"));
        assert_eq!(p.battery_hw_label.as_deref(), Some("absent"));
    }

    #[test]
    fn rejects_truncated_head() {
        // Only 5 space-delimited fields, far too few.
        assert!(parse(b"B01.00 F08.03 22/07/19 0 12,84").is_err());
    }
}
