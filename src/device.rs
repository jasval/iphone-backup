use std::process::Command;
use std::sync::mpsc::Sender;

#[derive(Debug, Clone)]
pub struct Device {
    pub udid: String,
    pub name: String,
    pub ios: Option<String>,
    pub model: Option<String>,
}

pub fn list_connected() -> Vec<Device> {
    let out = match Command::new("idevice_id").args(["--network", "--list"]).output() {
        Ok(o) if o.status.success() && !o.stdout.is_empty() => {
            String::from_utf8_lossy(&o.stdout).to_string()
        }
        // Fall back to all devices (USB + network) if no network devices found.
        _ => match Command::new("idevice_id").arg("--list").output() {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
            _ => return vec![],
        },
    };
    out.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|udid| {
            let udid = udid.trim().to_string();
            // Fetch all properties in one call instead of three.
            let info = device_info_all(&udid);
            let name = info
                .get("DeviceName")
                .cloned()
                .unwrap_or_else(|| udid.clone());
            let ios = info.get("ProductVersion").cloned();
            let model = info.get("ProductType").cloned();
            Device {
                udid,
                name,
                ios,
                model,
            }
        })
        .collect()
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

/// Fetch all device properties in one `ideviceinfo` call.
fn device_info_all(udid: &str) -> std::collections::HashMap<String, String> {
    let out = match Command::new("ideviceinfo")
        .args(["--udid", udid])
        .output()
    {
        Ok(o) if o.status.success() => o.stdout,
        _ => return std::collections::HashMap::new(),
    };
    String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, ": ");
            let key = parts.next()?.trim().to_string();
            let val = parts.next()?.trim().to_string();
            if key.is_empty() || val.is_empty() {
                None
            } else {
                Some((key, val))
            }
        })
        .collect()
}
