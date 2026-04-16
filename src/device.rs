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
    let out = match Command::new("idevice_id").arg("-l").output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return vec![],
    };
    out.lines()
        .filter(|l| !l.is_empty())
        .map(|udid| {
            let udid = udid.trim().to_string();
            let name = device_info(&udid, "DeviceName").unwrap_or_else(|| udid.clone());
            let ios = device_info(&udid, "ProductVersion");
            let model = device_info(&udid, "ProductType");
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

fn device_info(udid: &str, key: &str) -> Option<String> {
    let out = Command::new("ideviceinfo")
        .args(["-u", udid, "-k", key])
        .output()
        .ok()?;
    if out.status.success() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    } else {
        None
    }
}
