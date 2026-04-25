use std::process::Command;
use std::sync::mpsc::Sender;

use crate::imd;

#[derive(Debug, Clone, PartialEq)]
pub enum Connection {
    Usb,
    Network,
    Both,
    UsbUnpaired,
}

#[derive(Debug, Clone)]
pub struct Device {
    pub udid: String,
    pub name: String,
    pub ios: Option<String>,
    pub model: Option<String>,
    pub connection: Connection,
}

pub fn list_connected() -> Vec<Device> {
    let usb_set: std::collections::HashSet<String> =
        imd::list_usb().unwrap_or_default().into_iter().collect();
    let net_set: std::collections::HashSet<String> =
        imd::list_network().unwrap_or_default().into_iter().collect();

    let mut seen = std::collections::HashSet::new();
    let udids: Vec<String> = usb_set
        .iter()
        .chain(net_set.iter())
        .filter(|u| seen.insert((*u).clone()))
        .cloned()
        .collect();

    if udids.is_empty() {
        return detect_usb_ios();
    }

    let mut devices: Vec<Device> = udids
        .into_iter()
        .map(|udid| {
            let in_usb = usb_set.contains(&udid);
            let in_net = net_set.contains(&udid);
            let connection = match (in_usb, in_net) {
                (true, true) => Connection::Both,
                (true, false) => Connection::Usb,
                _ => Connection::Network,
            };
            let info = imd::device_info(&udid).unwrap_or_default();
            let name = info
                .get("DeviceName")
                .cloned()
                .unwrap_or_else(|| udid.clone());
            let ios = info.get("ProductVersion").cloned();
            let model = info.get("ProductType").cloned();
            Device { udid, name, ios, model, connection }
        })
        .collect();

    devices.sort_by_key(|d| matches!(d.connection, Connection::Network) as u8);
    devices
}

/// Run `idevicepair pair` for the given UDID (or all devices if None).
/// Streams log lines to `tx`.
pub fn pair(udid: Option<&str>, tx: &Sender<String>) {
    let mut cmd = Command::new("idevicepair");
    if let Some(u) = udid {
        cmd.args(["-u", u]);
    }
    cmd.arg("pair");

    let _ = tx.send("Running idevicepair pair...".into());
    match cmd.output() {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            for line in stdout.lines().chain(stderr.lines()) {
                if !line.trim().is_empty() {
                    let _ = tx.send(line.to_string());
                }
            }
            if o.status.success() {
                let _ = tx.send("✓ Pairing successful.".into());
            } else {
                let _ = tx.send("✗ Pairing failed. Make sure the device is connected via USB and trust has been accepted.".into());
            }
        }
        Err(e) => {
            let _ = tx.send(format!(
                "ERROR: idevicepair not found ({e}). Install with: brew install libimobiledevice"
            ));
        }
    }
}

/// Detect iOS devices connected via USB using ioreg, regardless of pairing state.
/// Falls back to this when idevice_id finds nothing (device connected but not yet paired).
fn detect_usb_ios() -> Vec<Device> {
    let out = match Command::new("ioreg")
        .args(["-p", "IOUSB", "-r", "-l", "-c", "IOUSBHostDevice"])
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return vec![],
    };

    let mut devices = Vec::new();
    let mut vendor: Option<u32> = None;
    let mut product_name: Option<String> = None;
    let mut serial: Option<String> = None;

    for line in out.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("+-o") {
            if vendor == Some(1452) {
                if let Some(name) = product_name.take() {
                    if name.contains("iPhone") || name.contains("iPad") || name.contains("iPod") {
                        let udid = imd::normalize_udid(&serial.take().unwrap_or_default());
                        devices.push(Device {
                            udid,
                            name,
                            ios: None,
                            model: None,
                            connection: Connection::UsbUnpaired,
                        });
                    }
                }
            }
            vendor = None;
            product_name = None;
            serial = None;
            continue;
        }

        if let Some(val) = imd::parse_ioreg_int(trimmed, "idVendor") {
            vendor = Some(val);
        } else if let Some(val) = imd::parse_ioreg_str(trimmed, "kUSBProductString") {
            product_name = Some(val);
        } else if let Some(val) = imd::parse_ioreg_str(trimmed, "kUSBSerialNumberString") {
            serial = Some(val);
        }
    }

    if vendor == Some(1452) {
        if let Some(name) = product_name {
            if name.contains("iPhone") || name.contains("iPad") || name.contains("iPod") {
                let udid = imd::normalize_udid(&serial.unwrap_or_default());
                devices.push(Device {
                    udid,
                    name,
                    ios: None,
                    model: None,
                    connection: Connection::UsbUnpaired,
                });
            }
        }
    }

    devices
}
